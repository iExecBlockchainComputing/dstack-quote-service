use anyhow::{Context, Result};
use axum::{Router, routing::get};
use std::sync::Arc;

use dstack_sdk::dstack_client::DstackClient;
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::handlers;
use crate::instance_file;

#[derive(Clone)]
pub struct AppState {
    pub dstack_client: Arc<DstackClient>,
}

pub struct Application {
    config: Config,
    state: AppState,
}

impl Application {
    pub fn new(config: Config) -> Result<Self> {
        let dstack_client = Arc::new(DstackClient::new(None));
        Ok(Self {
            config,
            state: AppState { dstack_client },
        })
    }

    fn build_router(&self) -> Router {
        debug!("Building application router");

        let cors = CorsLayer::new()
            .allow_methods([axum::http::Method::GET, axum::http::Method::OPTIONS])
            .allow_origin(tower_http::cors::Any);

        Router::new()
            // Root endpoint
            .route("/", get(handlers::root))
            // Health check endpoint
            .route("/health", get(handlers::health_check))
            // Quote endpoint
            .route("/quote", get(handlers::get_quote))
            // Attest endpoint
            .route("/attest", get(handlers::attest))
            // Info endpoint
            .route("/info", get(handlers::info))
            .with_state(self.state.clone())
            .layer(TraceLayer::new_for_http())
            .layer(cors)
    }

    /// Writes the instance file, when enabled, before the listener is bound.
    ///
    /// Doing it first means that a successful `/health` response also guarantees
    /// the file is on disk, so consumers can simply wait for the container to be
    /// healthy.
    async fn write_instance_file(&self) -> Result<()> {
        let cfg = &self.config.instance_file;
        if !cfg.enabled {
            debug!("Instance file disabled, skipping");
            return Ok(());
        }

        match instance_file::write(&self.state.dstack_client, cfg).await {
            Ok(()) => {
                info!("Instance file written to {}", cfg.path.display());
                Ok(())
            }
            Err(e) if cfg.required => Err(e.context("Failed to write the required instance file")),
            Err(e) => {
                warn!("Failed to write the instance file, continuing anyway: {e:#}");
                Ok(())
            }
        }
    }

    pub async fn run(self) -> Result<()> {
        self.write_instance_file().await?;

        let addr = self.config.bind_addr();
        let app = self.build_router();
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .with_context(|| format!("Failed to bind server to address {}", addr))?;

        info!("Server bound to {}", addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("Server encountered an error during execution")?;

        info!("Server shutdown complete");
        Ok(())
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, shutting down gracefully...");
        },
        _ = terminate => {
            info!("Received SIGTERM, shutting down gracefully...");
        },
    }

    warn!("Shutdown signal received, cleaning up...");
}

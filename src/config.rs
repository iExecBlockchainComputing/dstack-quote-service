use config::{Config as ConfigBuilder, ConfigError, Environment};
use serde::Deserialize;
use std::path::PathBuf;
use tracing::debug;
#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub fluent_bit_fragment: FluentBitFragmentConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

/// Configuration of the startup Fluent Bit fragment.
///
/// When generation is on, the service queries the dstack guest agent once at
/// startup and persists the CVM identity as a Fluent Bit configuration fragment.
/// Fluent Bit picks it up through an `@INCLUDE` and can then label its records
/// without needing its own access to the dstack socket.
///
/// There is no opt-out of aborting on failure: asking for the fragment means
/// something downstream needs it, and the dstack socket is this service's only
/// external dependency, so a guest agent that cannot be reached leaves every
/// other endpoint broken anyway.
#[derive(Debug, Clone, Deserialize)]
pub struct FluentBitFragmentConfig {
    /// Whether the fragment is written at startup.
    pub generate: bool,
    /// Destination path of the fragment.
    pub path: PathBuf,
    /// Number of extra attempts after the initial one.
    pub retries: u32,
    /// Delay between two attempts, in milliseconds.
    pub retry_delay_ms: u64,
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let config = ConfigBuilder::builder()
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 9999)?
            .set_default("fluent_bit_fragment.generate", false)?
            .set_default("fluent_bit_fragment.path", "/shared/instance.conf")?
            .set_default("fluent_bit_fragment.retries", 5)?
            .set_default("fluent_bit_fragment.retry_delay_ms", 2000)?
            // Load environment variables (QUOTE_SIDECAR_*)
            .add_source(
                Environment::with_prefix("QUOTE_SIDECAR")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?;

        config.try_deserialize()
    }

    pub fn bind_addr(&self) -> String {
        let addr = format!("{}:{}", self.server.host, self.server.port);
        debug!("Starting Quote Sidecar server on {}", addr);
        addr
    }
}

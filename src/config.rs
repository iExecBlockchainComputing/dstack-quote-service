use config::{Config as ConfigBuilder, ConfigError, Environment};
use serde::Deserialize;
use std::path::PathBuf;
use tracing::debug;
#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub instance_file: InstanceFileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

/// Configuration of the startup instance file.
///
/// When enabled, the service queries the dstack guest agent once at startup and
/// persists the CVM identity fields to a shell-sourceable `.env` file. Sidecars
/// such as Fluent Bit can then label their records without needing their own
/// access to the dstack socket.
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceFileConfig {
    /// Whether the instance file is written at startup.
    pub enabled: bool,
    /// Destination path of the `.env` file.
    pub path: PathBuf,
    /// When true, failing to write the file aborts startup.
    pub required: bool,
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
            .set_default("instance_file.enabled", false)?
            .set_default("instance_file.path", "/shared/instance.env")?
            .set_default("instance_file.required", true)?
            .set_default("instance_file.retries", 5)?
            .set_default("instance_file.retry_delay_ms", 2000)?
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

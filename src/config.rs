use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GrpcProcessor {
    pub addr: String,
    pub port: u16,
    pub tls_dir: Option<PathBuf>,
}

impl Default for GrpcProcessor {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1".to_string(),
            port: 50051,
            tls_dir: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WalletConfig {
    pub mnemonic_seed: String,
    pub mint_urls: Vec<String>,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            mnemonic_seed: String::new(),
            mint_urls: vec!["https://mint.example.com".to_string()],
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1".to_string(),
            port: 3000,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Settings {
    pub grpc_processor: GrpcProcessor,
    pub wallet: WalletConfig,
    pub server: ServerConfig,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        Self::with_work_dir(None)
    }

    pub fn with_work_dir(work_dir: Option<&str>) -> Result<Self, ConfigError> {
        // Start with default settings
        let mut s = Config::builder()
            // Start with default values
            .add_source(Config::try_from(&Self::default())?)
            // Add in the current environment
            // Prefix can be empty, or set to something like "CDK_GATEWAY"
            .add_source(Environment::with_prefix("CDK_GATEWAY").separator("__"));

        // If work_dir is provided, look for config file there
        if let Some(dir) = work_dir {
            let config_path = std::path::Path::new(dir).join("config.toml");
            tracing::debug!("Looking for config file at: {:?}", config_path);
            if config_path.exists() {
                tracing::info!("Found config file at: {:?}", config_path);
            }
            s = s.add_source(File::from(config_path).required(false));
        } else {
            // Otherwise look in the current directory
            tracing::debug!("Looking for config.toml in current directory");
            s = s.add_source(File::with_name("config").required(false));
        }

        // You can also specify a different config file path with an environment variable
        if let Ok(config_path) = std::env::var("CDK_GATEWAY_CONFIG") {
            tracing::info!("Using config file specified by CDK_GATEWAY_CONFIG: {}", config_path);
            s = s.add_source(File::with_name(&config_path).required(true));
        }

        // Build and deserialize the config
        tracing::debug!("Building configuration");
        let result = s.build()?.try_deserialize::<Self>();
        match &result {
            Ok(settings) => {
                tracing::info!("Configuration successfully loaded");
                tracing::debug!("Server configured to listen on {}:{}", settings.server.listen_addr, settings.server.port);
                tracing::debug!("Payment processor configured at {}:{}", settings.grpc_processor.addr, settings.grpc_processor.port);
                tracing::debug!("Configured with {} mint URLs", settings.wallet.mint_urls.len());
            }
            Err(e) => {
                tracing::error!("Failed to load configuration: {}", e);
            }
        }
        result
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            grpc_processor: GrpcProcessor::default(),
            wallet: WalletConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

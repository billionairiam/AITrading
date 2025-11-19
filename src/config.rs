use chrono::Duration;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use thiserror::Error;

// --- Custom Error Type ---

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse JSON config: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Configuration validation failed: {0}")]
    Validation(String),
}

// --- Enums for Type Safety ---

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum AiModel {
    Qwen,
    Deepseek,
    Custom,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Exchange {
    Binance,
    Hyperliquid,
    Aster,
}

// Default value for Exchange if not specified in the JSON
fn default_exchange() -> Exchange {
    Exchange::Binance
}

// --- Configuration Structs ---

#[derive(Serialize, Deserialize, Debug)]
pub struct TraderConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(rename = "ai_model")]
    pub ai_model: AiModel,
    #[serde(default = "default_exchange")]
    pub exchange: Exchange,

    // Binance config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binance_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binance_secret_key: Option<String>,

    // Hyperliquid config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hyperliquid_private_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hyperliquid_wallet_addr: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hyperliquid_testnet: bool,

    // Aster config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aster_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aster_signer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aster_private_key: Option<String>,

    // AI config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qwen_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deepseek_key: Option<String>,

    // Custom AI API config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_api_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_model_name: Option<String>,

    pub initial_balance: f64,
    #[serde(default = "default_scan_interval")]
    pub scan_interval_minutes: i32,
}

fn default_scan_interval() -> i32 {
    3
}

impl TraderConfig {
    /// Returns the scan interval as a `chrono::Duration`.
    pub fn get_scan_interval(&self) -> Duration {
        Duration::minutes(self.scan_interval_minutes as i64)
    }

    /// Validates a single trader's configuration.
    fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("ID cannot be empty".to_string());
        }
        if self.name.is_empty() {
            return Err("Name cannot be empty".to_string());
        }
        if self.initial_balance <= 0.0 {
            return Err("initial_balance must be greater than 0".to_string());
        }

        // Validate exchange-specific keys
        match self.exchange {
            Exchange::Binance => {
                if self.binance_api_key.is_none() || self.binance_secret_key.is_none() {
                    return Err(
                        "Binance exchange requires 'binance_api_key' and 'binance_secret_key'"
                            .to_string(),
                    );
                }
            }
            Exchange::Hyperliquid => {
                if self.hyperliquid_private_key.is_none() {
                    return Err(
                        "Hyperliquid exchange requires 'hyperliquid_private_key'".to_string()
                    );
                }
            }
            Exchange::Aster => {
                if self.aster_user.is_none()
                    || self.aster_signer.is_none()
                    || self.aster_private_key.is_none()
                {
                    return Err("Aster exchange requires 'aster_user', 'aster_signer', and 'aster_private_key'".to_string());
                }
            }
        }

        // Validate AI model-specific keys
        match self.ai_model {
            AiModel::Qwen => {
                if self.qwen_key.is_none() {
                    return Err("Qwen AI model requires 'qwen_key'".to_string());
                }
            }
            AiModel::Deepseek => {
                if self.deepseek_key.is_none() {
                    return Err("DeepSeek AI model requires 'deepseek_key'".to_string());
                }
            }
            AiModel::Custom => {
                if self.custom_api_url.is_none() {
                    return Err("Custom AI model requires 'custom_api_url'".to_string());
                }
                if self.custom_api_key.is_none() {
                    return Err("Custom AI model requires 'custom_api_key'".to_string());
                }
                if self.custom_model_name.is_none() {
                    return Err("Custom AI model requires 'custom_model_name'".to_string());
                }
            }
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(default)] // Allows serde to fill in missing fields from the Default impl
pub struct LeverageConfig {
    pub btc_eth_leverage: i32,
    pub altcoin_leverage: i32,
}

impl Default for LeverageConfig {
    fn default() -> Self {
        Self {
            btc_eth_leverage: 5, // Safe default, compatible with sub-accounts
            altcoin_leverage: 5, // Safe default, compatible with sub-accounts
        }
    }
}

impl LeverageConfig {
    /// Prints warnings if leverage is set to a potentially risky value for sub-accounts.
    fn check_warnings(&self) {
        if self.btc_eth_leverage > 5 {
            println!(
                "⚠️ Warning: BTC/ETH leverage is set to {}x, which may fail on sub-accounts (limit is often ≤5x)",
                self.btc_eth_leverage
            );
        }
        if self.altcoin_leverage > 5 {
            println!(
                "⚠️ Warning: Altcoin leverage is set to {}x, which may fail on sub-accounts (limit is often ≤5x)",
                self.altcoin_leverage
            );
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(default)] // Allows serde to fill in missing fields from the Default impl
pub struct Config {
    pub traders: Vec<TraderConfig>,
    pub use_default_coins: bool,
    pub default_coins: Vec<String>,
    pub api_server_port: u16,
    pub max_daily_loss: f64,
    pub max_drawdown: f64,
    pub stop_trading_minutes: i32,
    pub leverage: LeverageConfig,
}

fn default_coin_list() -> Vec<String> {
    vec![
        "BTCUSDT".to_string(),
        "ETHUSDT".to_string(),
        "SOLUSDT".to_string(),
        "BNBUSDT".to_string(),
        "XRPUSDT".to_string(),
        "DOGEUSDT".to_string(),
        "ADAUSDT".to_string(),
        "HYPEUSDT".to_string(),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            traders: Vec::new(),
            use_default_coins: true,
            default_coins: default_coin_list(),
            api_server_port: 8080,
            max_daily_loss: 0.0,
            max_drawdown: 0.0,
            stop_trading_minutes: 0,
            leverage: LeverageConfig::default(),
        }
    }
}

impl Config {
    /// Validates the entire configuration.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.traders.is_empty() {
            return Err(ConfigError::Validation(
                "At least one trader must be configured".to_string(),
            ));
        }

        let mut trader_ids = HashSet::new();
        for (i, trader) in self.traders.iter().enumerate() {
            if !trader_ids.insert(&trader.id) {
                return Err(ConfigError::Validation(format!(
                    "Trader ID '{}' is duplicated at index {}",
                    trader.id, i
                )));
            }
            // Add context (trader index) to any validation errors
            trader.validate().map_err(|e| {
                ConfigError::Validation(format!(
                    "Trader[{}][id={}] validation failed: {}",
                    i, trader.id, e
                ))
            })?;
        }
        Ok(())
    }
}

/// Loads, parses, and validates the configuration from a JSON file.
pub fn load_config(filename: &str) -> Result<Config, ConfigError> {
    let data = fs::read_to_string(filename)?;
    let mut config: Config = serde_json::from_str(&data)?;

    // Handle special default case: if default_coins is provided but empty, populate it.
    if config.use_default_coins && config.default_coins.is_empty() {
        config.default_coins = default_coin_list();
    }

    // Run all validation checks.
    config.validate()?;

    // Print non-fatal warnings after validation is successful.
    config.leverage.check_warnings();

    Ok(config)
}

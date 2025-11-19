use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Data {
    pub symbol: String,
    pub current_price: f64,
    pub price_change_1h: f64,
    pub price_change_4h: f64,
    pub current_ema20: f64,
    pub current_macd: f64,
    pub current_rsi7: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_interest: Option<OIData>,
    pub funding_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intraday_series: Option<IntradayData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longer_term_context: Option<LongerTermData>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OIData {
    pub latest: f64,
    pub average: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct IntradayData {
    pub mid_prices: Vec<f64>,
    pub ema20_values: Vec<f64>,
    pub macd_values: Vec<f64>,
    pub rsi7_values: Vec<f64>,
    pub rsi14_values: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LongerTermData {
    pub ema20: f64,
    pub ema50: f64,
    pub atr3: f64,
    pub atr14: f64,
    pub current_volume: f64,
    pub average_volume: f64,
    pub macd_values: Vec<f64>,
    pub rsi14_values: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExchangeInfo {
    pub symbols: Vec<SymbolInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInfo {
    pub symbol: String,
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub contract_type: String,
    pub price_precision: i32,
    pub quantity_precision: i32,
}

/// Represents a single Kline (candlestick). Note: Binance often sends this
/// as a JSON array, not an object. If so, a custom deserializer would be needed.
/// This struct assumes a JSON object response as defined by the Go struct tags.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Kline {
    pub open_time: i64,
    pub open: f64, // Prices/volumes are often strings to avoid precision loss
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub close_time: i64,
    pub quote_volume: f64,
    pub trades: i64,
    pub taker_buy_base_volume: f64,
    pub taker_buy_quote_volume: f64,
}

impl From<Vec<serde_json::Value>> for Kline {
    fn from(value: Vec<serde_json::Value>) -> Self {
        fn parse_val<T: std::str::FromStr>(val: &serde_json::Value) -> T {
            val.as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or_else(|_| T::from_str("0").ok().unwrap())
        }
        fn parse_int(val: &serde_json::Value) -> i64 {
            val.as_i64().unwrap_or(0)
        }

        Kline {
            open_time: parse_int(&value[0]),
            open: parse_val(&value[1]),
            high: parse_val(&value[2]),
            low: parse_val(&value[3]),
            close: parse_val(&value[4]),
            volume: parse_val(&value[5]),
            close_time: parse_int(&value[6]),
            quote_volume: parse_val(&value[7]),
            trades: parse_int(&value[8]),
            taker_buy_base_volume: parse_val(&value[10]),
            taker_buy_quote_volume: parse_val(&value[11]),
        }
    }
}

pub type KlineResponse = Vec<serde_json::Value>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PriceTicker {
    pub symbol: String,
    pub price: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Ticker24hr {
    pub symbol: String,
    pub price_change: String,
    pub price_change_percent: String,
    pub volume: String,
    pub quote_volume: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SymbolFeatures {
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub price: f64,
    pub price_change_15min: f64,
    pub price_change_1h: f64,
    pub price_change_4h: f64,
    pub volume: f64,
    pub volume_ratio_5: f64,
    pub volume_ratio_20: f64,
    pub volume_trend: f64,
    pub rsi_14: f64,
    pub sma_5: f64,
    pub sma_10: f64,
    pub sma_20: f64,
    pub high_low_ratio: f64,
    pub volatility_20: f64,
    pub position_in_range: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Alert {
    #[serde(rename = "type")]
    pub alert_type: String,
    pub symbol: String,
    pub value: f64,
    pub threshold: f64,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub alert_thresholds: AlertThresholds,
    pub update_interval: u64, // seconds
    pub cleanup_config: CleanupConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AlertThresholds {
    pub volume_spike: f64,
    pub price_change_15min: f64,
    pub volume_trend: f64,
    pub rsi_overbought: f64,
    pub rsi_oversold: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CleanupConfig {
    #[serde(with = "humantime_serde")]
    pub inactive_timeout: Duration,
    pub min_score_threshold: f64,
    #[serde(with = "humantime_serde")]
    pub no_alert_timeout: Duration,
    #[serde(with = "humantime_serde")]
    pub check_interval: Duration,
}

pub static CONFIG: Lazy<Config> = Lazy::new(|| Config {
    alert_thresholds: AlertThresholds {
        volume_spike: 3.0,
        price_change_15min: 0.05,
        volume_trend: 2.0,
        rsi_overbought: 70.0,
        rsi_oversold: 30.0,
    },
    cleanup_config: CleanupConfig {
        inactive_timeout: Duration::from_secs(30 * 60), // 30 minutes
        min_score_threshold: 15.0,
        no_alert_timeout: Duration::from_secs(20 * 60), // 20 minutes
        check_interval: Duration::from_secs(5 * 60),    // 5 minutes
    },
    update_interval: 60, // 1 minute
});

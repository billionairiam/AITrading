use anyhow::{Context, Ok, Result};
use serde::Deserialize;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use std::fmt;
use std::time::Duration;

use crate::types::{ExchangeInfo, Kline, PriceTicker};

const BASE_URL: &str = "https://fapi.binance.com";

pub struct ApiClient {
    client: reqwest::blocking::Client,
}

impl ApiClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client })
    }

    pub fn get_exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/fapi/v1/exchangeInfo", BASE_URL);
        let resp = self.client.get(url).send()?;
        let exchange_info = resp
            .json::<ExchangeInfo>()
            .context("Failed to deserialize ExchangeInfo")?;

        Ok(exchange_info)
    }

    pub fn get_klines(&self, symbol: &str, interval: &str, limit: i32) -> Result<Vec<Kline>> {
        let url = format!("{}/fapi/v1/klines", BASE_URL);
        let klines = self
            .client
            .get(&url)
            .query(&[
                ("symbol", symbol),
                ("interval", interval),
                ("limit", &limit.to_string()),
            ])
            .send()?
            .json::<Vec<Kline>>()
            .context("Failed to deserialize Klines")?;

        Ok(klines)
    }

    pub fn get_current_price(&self, symbol: &str) -> Result<f64> {
        let url = format!("{}/fapi/v1/ticker/price", BASE_URL);
        let ticker = self
            .client
            .get(&url)
            .query(&[("symbol", symbol)])
            .send()?
            .json::<PriceTicker>()
            .context("Failed to deserialize PriceTicker")?;

        // Parse the price string into a float
        let price = ticker
            .price
            .parse::<f64>()
            .context(format!("Failed to parse price '{}'", ticker.price))?;

        Ok(price)
    }
}

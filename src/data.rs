use serde::Deserialize;
use std::fmt::Write;
use thiserror::Error;

use crate::types::{Data, IntradayData, Kline, LongerTermData, OIData};

#[derive(Error, Debug)]
pub enum MarketError {
    #[error("Failed to fetch data from API: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("Failed to parse string to float: {0}")]
    ParseFloatError(#[from] std::num::ParseFloatError),
    #[error("Failed to parse JSON response: {0}")]
    ParseJsonError(#[from] serde_json::Error),
    #[error("Insufficient data for calculation: {0}")]
    InsufficientData(String),
}

/// Get market data for a specific symbol.
pub async fn get(symbol: &str) -> Result<Data, MarketError> {
    let symbol = normalize(symbol);

    // Concurrently fetch all required data
    let (klines3m, klines4h, oi_data, funding_rate) = tokio::try_join!(
        get_klines(&symbol, "3m", 50), // Fetch more for calculations
        get_klines(&symbol, "4h", 60), // Fetch more for calculations
        get_open_interest_data(&symbol),
        get_funding_rate(&symbol)
    )?;

    let current_price = klines3m.last().map_or(0.0, |k| k.close);
    if current_price == 0.0 {
        return Err(MarketError::InsufficientData(
            "Could not get current price from 3m klines.".into(),
        ));
    }

    let current_ema20 = calculate_ema(&klines3m, 20);
    let current_macd = calculate_macd(&klines3m);
    let current_rsi7 = calculate_rsi(&klines3m, 7);

    // Calculate price change percentages
    let price_change_1h = if klines3m.len() >= 21 {
        let price_1h_ago = klines3m[klines3m.len() - 21].close;
        if price_1h_ago > 0.0 {
            (current_price - price_1h_ago) / price_1h_ago * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    let price_change_4h = if klines4h.len() >= 2 {
        let price_4h_ago = klines4h[klines4h.len() - 2].close;
        if price_4h_ago > 0.0 {
            (current_price - price_4h_ago) / price_4h_ago * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    let intraday_data = calculate_intraday_series(&klines3m);
    let longer_term_data = calculate_longer_term_data(&klines4h);

    Ok(Data {
        symbol,
        current_price,
        price_change_1h,
        price_change_4h,
        current_ema20,
        current_macd,
        current_rsi7,
        open_interest: oi_data,
        funding_rate: funding_rate.unwrap(),
        intraday_series: Some(intraday_data),
        longer_term_context: Some(longer_term_data),
    })
}

// --- Indicator Calculations ---

fn calculate_ema(klines: &[Kline], period: usize) -> f64 {
    if klines.len() < period {
        return 0.0;
    }
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();

    // Calculate SMA for the first value
    let mut ema = closes[..period].iter().sum::<f64>() / period as f64;
    let multiplier = 2.0 / (period as f64 + 1.0);

    // Calculate EMA for the rest of the values
    for price in closes[period..].iter() {
        ema = (price - ema) * multiplier + ema;
    }
    ema
}

fn calculate_macd(klines: &[Kline]) -> f64 {
    if klines.len() < 26 {
        return 0.0;
    }
    let ema12 = calculate_ema(klines, 12);
    let ema26 = calculate_ema(klines, 26);
    ema12 - ema26
}

fn calculate_rsi(klines: &[Kline], period: usize) -> f64 {
    if klines.len() <= period {
        return 0.0;
    }
    let mut gains = 0.0;
    let mut losses = 0.0;

    for i in 1..=period {
        let change = klines[i].close - klines[i - 1].close;
        if change > 0.0 {
            gains += change;
        } else {
            losses += -change;
        }
    }

    let mut avg_gain = gains / period as f64;
    let mut avg_loss = losses / period as f64;

    for i in (period + 1)..klines.len() {
        let change = klines[i].close - klines[i - 1].close;
        let (gain, loss) = if change > 0.0 {
            (change, 0.0)
        } else {
            (0.0, -change)
        };
        avg_gain = (avg_gain * (period - 1) as f64 + gain) / period as f64;
        avg_loss = (avg_loss * (period - 1) as f64 + loss) / period as f64;
    }

    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

fn calculate_atr(klines: &[Kline], period: usize) -> f64 {
    if klines.len() <= period {
        return 0.0;
    }
    let mut trs = Vec::with_capacity(klines.len());
    trs.push(0.0); // No TR for the first candle

    for i in 1..klines.len() {
        let high = klines[i].high;
        let low = klines[i].low;
        let prev_close = klines[i - 1].close;

        let tr1 = high - low;
        let tr2 = (high - prev_close).abs();
        let tr3 = (low - prev_close).abs();

        trs.push(tr1.max(tr2).max(tr3));
    }

    // Initial ATR is a simple moving average
    let mut atr = trs[1..=period].iter().sum::<f64>() / period as f64;

    // Wilder's smoothing
    for i in (period + 1)..trs.len() {
        atr = (atr * (period - 1) as f64 + trs[i]) / period as f64;
    }

    atr
}

fn calculate_intraday_series(klines: &[Kline]) -> IntradayData {
    let mut data = IntradayData::default();
    let total_len = klines.len();
    if total_len == 0 {
        return data;
    }

    let start = total_len.saturating_sub(10);

    for i in start..total_len {
        let kline_slice = &klines[..=i];
        data.mid_prices.push(kline_slice.last().unwrap().close);

        if kline_slice.len() >= 20 {
            data.ema20_values.push(calculate_ema(kline_slice, 20));
        }
        if kline_slice.len() >= 26 {
            data.macd_values.push(calculate_macd(kline_slice));
        }
        if kline_slice.len() > 7 {
            data.rsi7_values.push(calculate_rsi(kline_slice, 7));
        }
        if kline_slice.len() > 14 {
            data.rsi14_values.push(calculate_rsi(kline_slice, 14));
        }
    }
    data
}

fn calculate_longer_term_data(klines: &[Kline]) -> LongerTermData {
    let mut data = LongerTermData::default();
    let total_len = klines.len();
    if total_len == 0 {
        return data;
    }

    data.ema20 = calculate_ema(klines, 20);
    data.ema50 = calculate_ema(klines, 50);
    data.atr3 = calculate_atr(klines, 3);
    data.atr14 = calculate_atr(klines, 14);

    data.current_volume = klines.last().map_or(0.0, |k| k.volume);
    let volume_sum: f64 = klines.iter().map(|k| k.volume).sum();
    data.average_volume = if !klines.is_empty() {
        volume_sum / klines.len() as f64
    } else {
        0.0
    };

    let start = total_len.saturating_sub(10);
    for i in start..total_len {
        let kline_slice = &klines[..=i];
        if kline_slice.len() >= 26 {
            data.macd_values.push(calculate_macd(kline_slice));
        }
        if kline_slice.len() > 14 {
            data.rsi14_values.push(calculate_rsi(kline_slice, 14));
        }
    }
    data
}

// --- API Fetchers ---

async fn get_klines(symbol: &str, interval: &str, limit: u16) -> Result<Vec<Kline>, MarketError> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol, interval, limit
    );
    let klines = reqwest::get(&url).await?.json::<Vec<Kline>>().await?;
    Ok(klines)
}

async fn get_open_interest_data(symbol: &str) -> Result<Option<OIData>, MarketError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OIResponse {
        open_interest: String,
    }
    let url = format!(
        "https://fapi.binance.com/fapi/v1/openInterest?symbol={}",
        symbol
    );

    let resp = reqwest::get(&url).await?;
    if !resp.status().is_success() {
        return Ok(None); // API might fail (e.g., for spot symbols), return None
    }

    let result = resp.json::<OIResponse>().await?;
    let oi = result.open_interest.parse::<f64>()?;

    Ok(Some(OIData {
        latest: oi,
        average: oi * 0.999, // Approximation from original code
    }))
}

async fn get_funding_rate(symbol: &str) -> Result<Option<f64>, MarketError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FundingResponse {
        last_funding_rate: String,
    }
    let url = format!(
        "https://fapi.binance.com/fapi/v1/premiumIndex?symbol={}",
        symbol
    );

    let resp = reqwest::get(&url).await?;
    if !resp.status().is_success() {
        return Ok(None);
    }

    let result = resp.json::<FundingResponse>().await?;
    let rate = result.last_funding_rate.parse::<f64>()?;
    Ok(Some(rate))
}

// --- Formatting & Helpers ---

/// Formats the market data into a human-readable string.
pub fn format(data: &Data) -> String {
    let mut s = String::new();

    let _ = writeln!(
        s,
        "current_price = {:.2}, current_ema20 = {:.3}, current_macd = {:.3}, current_rsi (7 period) = {:.3}\n",
        data.current_price, data.current_ema20, data.current_macd, data.current_rsi7
    );

    let _ = writeln!(
        s,
        "In addition, here is the latest {} open interest and funding rate for perps:\n",
        data.symbol
    );

    if let Some(oi) = &data.open_interest {
        let _ = writeln!(
            s,
            "Open Interest: Latest: {:.2} Average: {:.2}\n",
            oi.latest, oi.average
        );
    }

    let _ = writeln!(s, "Funding Rate: {:.2e}\n", data.funding_rate);

    let _ = writeln!(
        s,
        "Intraday series (3‑minute intervals, oldest → latest):\n"
    );
    match &data.intraday_series {
        Some(intraday_series) => {
            let _ = writeln!(
                s,
                "Mid prices: {}\n",
                format_float_slice(&intraday_series.mid_prices)
            );
            let _ = writeln!(
                s,
                "EMA indicators (20‑period): {}\n",
                format_float_slice(&intraday_series.ema20_values)
            );
            let _ = writeln!(
                s,
                "MACD indicators: {}\n",
                format_float_slice(&intraday_series.macd_values)
            );
            let _ = writeln!(
                s,
                "RSI indicators (7‑Period): {}\n",
                format_float_slice(&intraday_series.rsi7_values)
            );
            let _ = writeln!(
                s,
                "RSI indicators (14‑Period): {}\n",
                format_float_slice(&intraday_series.rsi14_values)
            );
        }
        None => (),
    }

    let _ = writeln!(s, "Longer‑term context (4‑hour timeframe):\n");
    let ltc = &data.longer_term_context;
    match ltc {
        Some(ltc) => {
            let _ = writeln!(
                s,
                "20‑Period EMA: {:.3} vs. 50‑Period EMA: {:.3}\n",
                &ltc.ema20, &ltc.ema50
            );
            let _ = writeln!(
                s,
                "3‑Period ATR: {:.3} vs. 14‑Period ATR: {:.3}\n",
                &ltc.atr3, &ltc.atr14
            );
            let _ = writeln!(
                s,
                "Current Volume: {:.3} vs. Average Volume: {:.3}\n",
                &ltc.current_volume, &ltc.average_volume
            );

            let _ = writeln!(
                s,
                "MACD indicators: {}\n",
                format_float_slice(&ltc.macd_values)
            );

            let _ = writeln!(
                s,
                "RSI indicators (14‑Period): {}\n",
                format_float_slice(&ltc.rsi14_values)
            );
        }
        None => (),
    }

    s
}

/// Formats a slice of f64 into a string like "[1.234, 5.678]".
fn format_float_slice(values: &[f64]) -> String {
    let parts: Vec<String> = values.iter().map(|v| format!("{:.3}", v)).collect();
    format!("[{}]", parts.join(", "))
}

/// Normalizes a symbol to its uppercase USDT pair format.
fn normalize(symbol: &str) -> String {
    let upper = symbol.to_uppercase();
    if upper.ends_with("USDT") {
        upper
    } else {
        format!("{}USDT", upper)
    }
}

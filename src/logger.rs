use std::error::Error;

use std::str::FromStr;
use std::time::{Duration, SystemTime};
use std::{fs, path::Path};

use anyhow::Result;
use chrono::{DateTime, Utc};
use glob::glob;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionRecord {
    timestamp: DateTime<Utc>,
    cycle_number: i32,
    system_prompt: String,
    input_prompt: String,
    cot_trace: String,
    decision_json: String,
    account_state: AccountSnapshot,
    positions: Vec<PositionSnapshot>,
    candidate_coins: Vec<String>,
    decisions: Vec<DecisionAction>,
    execution_log: Vec<String>,
    success: bool,
    error_message: String,
}

// AccountSnapshot 账户状态快照
#[derive(Debug, Serialize, Deserialize)]
struct AccountSnapshot {
    total_balance: f64,
    available_balance: f64,
    total_unrealized_profit: f64,
    position_count: i32,
    margin_used_pct: f64,
}

// PositionSnapshot 持仓快照
#[derive(Debug, Serialize, Deserialize)]
struct PositionSnapshot {
    symbol: String,
    side: String,
    position_amt: f64,
    entry_price: f64,
    mark_price: f64,
    unrealized_profit: f64,
    leverage: f64,
    liquidation_price: f64,
}

// DecisionAction 决策动作
#[derive(Debug, Serialize, Deserialize)]
struct DecisionAction {
    action: Action,
    symbol: String,
    quantity: f64,
    leverage: i32,
    price: f64,
    order_id: i64,
    timestamp: DateTime<Utc>,
    success: bool,
    error: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Action {
    #[serde(rename = "open_short")]
    OPENSHORT,
    #[serde(rename = "open_long")]
    OPENLONG,
    #[serde(rename = "close_short")]
    CLOSESHORT,
    #[serde(rename = "close_long")]
    CLOSELONG,
}

#[derive(Debug)]
struct DecisionLogger {
    log_dir: String,
    cycle_number: i32,
}

impl DecisionLogger {
    pub fn new(log_dir: &str) -> Self {
        let target_dir = if log_dir.is_empty() {
            "decision_logs"
        } else {
            log_dir
        };

        if let Err(e) = fs::create_dir_all(target_dir) {
            log::error!("⚠ 创建日志目录失败: {}", e);
        }

        DecisionLogger {
            log_dir: target_dir.to_string(),
            cycle_number: 0_i32,
        }
    }

    pub fn log_decision(&mut self, record: &mut DecisionRecord) -> Result<()> {
        self.cycle_number += 1;
        record.cycle_number = self.cycle_number;
        record.timestamp = Utc::now();

        // 生成文件名：decision_YYYYMMDD_HHMMSS_cycleN.json
        let time_str = record.timestamp.format("%Y%m%d_%H%M%S").to_string();
        let file_name = format!("decision_{}_cycle{}.json", time_str, record.cycle_number);
        let file_path = Path::new(&self.log_dir).join(&file_name);

        // 序列化为JSON（带缩进，方便阅读）
        let data = serde_json::to_string_pretty(record)?;

        // 写入文件
        fs::write(&file_path, data)?;

        log::info!("📝 决策记录已保存: {}", file_name);
        Ok(())
    }

    pub fn get_latest_records(&self, n: usize) -> Result<Vec<DecisionRecord>, Box<dyn Error>> {
        let read_dir =
            fs::read_dir(&self.log_dir).map_err(|e| format!("读取日志目录失败: {}", e))?;
        let mut entries: Vec<_> = read_dir
            .filter_map(|file| file.ok())
            .filter(|entry| match entry.file_type() {
                Ok(ft) => ft.is_file(),
                Err(_) => false,
            })
            .collect();
        entries.sort_by_key(|entry| entry.file_name());

        let start_index = entries.len().saturating_sub(n);
        let target_entry = &entries[start_index..];

        let mut records = Vec::new();
        for entry in target_entry {
            let path = entry.path();
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let record: DecisionRecord = match serde_json::from_str(&content) {
                Ok(r) => r,
                Err(_) => continue,
            };

            records.push(record);
        }

        Ok(records)
    }

    // 获取指定日期的所有记录
    pub fn get_record_by_date(
        &self,
        date: DateTime<Utc>,
    ) -> Result<Vec<DecisionRecord>, Box<dyn Error>> {
        let date_str = date.format("%Y%m%d_%H%M%S").to_string();
        let pattern = format!("{}/decision_{}_*.json", &self.log_dir, date_str);

        let mut records = Vec::new();
        for entry in glob(&pattern).map_err(|e| format!("Glob pattern error: {}", e))? {
            if let Ok(path) = entry {
                if let Ok(content) = fs::read_to_string(path) {
                    if let Ok(record) = serde_json::from_str(&content) {
                        records.push(record);
                    }
                }
            }
        }

        Ok(records)
    }

    // 清理N天前的旧记录
    pub fn clean_old_records(&self, days: u64) -> Result<(), Box<dyn Error>> {
        let cutoff_time = SystemTime::now()
            .checked_sub(Duration::from_secs(days * 24 * 60 * 60))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let read_dir =
            fs::read_dir(&self.log_dir).map_err(|e| format!("读取日志目录失败: {}", e))?;

        let mut removed_count = 0;

        for entry_res in read_dir {
            let entry = match entry_res {
                Ok(res) => res,
                Err(_) => continue,
            };

            let path = entry.path();
            if path.is_dir() {
                continue;
            }

            let metadata = match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let modtime = match metadata.modified() {
                Ok(m) => m,
                Err(_) => continue,
            };

            if modtime < cutoff_time {
                if let Err(e) = fs::remove_file(&path) {
                    let file_name = path
                        .file_name()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_else(|| "unknow".into());

                    log::error!("⚠ 删除旧记录失败 {}: {}\n", file_name, e);
                    continue;
                }
                removed_count += 1;
            }
        }

        if removed_count > 0 {
            log::info!("🗑️ 已清理 {} 条旧记录（{}天前）", removed_count, days);
        }

        Ok(())
    }

    // 获取统计信息
    pub fn get_statistics(&self) -> Result<Statistics, Box<dyn Error>> {
        let cur_dir =
            fs::read_dir(&self.log_dir).map_err(|e| format!("读取日志目录失败: {}", e))?;

        let mut stats = Statistics::default();

        for entry_res in cur_dir {
            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if path.is_dir() {
                continue;
            }

            let data = match fs::read(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let record: DecisionRecord = match serde_json::from_slice(&data) {
                Ok(dr) => dr,
                Err(_) => continue,
            };

            stats.total_cycles += 1;

            for action in &record.decisions {
                if action.success {
                    match action.action {
                        Action::OPENLONG | Action::OPENSHORT => stats.total_open_positions += 1,
                        Action::CLOSELONG | Action::CLOSESHORT => stats.total_close_positions += 1,
                    }
                }
            }

            if record.success {
                stats.successful_cycles += 1;
            } else {
                stats.failed_cycles += 1;
            }
        }

        Ok(stats)
    }

    pub fn analyze_performance(
        &self,
        lookback_cycles: usize,
    ) -> Result<PerformanceAnalysis, Box<dyn Error>> {
        let records = self
            .get_latest_records(lookback_cycles)
            .map_err(|e| format!("读取历史记录失败: {}", e))?;

        let mut analysis = PerformanceAnalysis::default();
        if records.len() == 0 {
            return Ok(analysis);
        }

        let mut open_positions: HashMap<String, HashMap<String, Value>> = HashMap::new();
        let all_records = self.get_latest_records(lookback_cycles * 3)?;
        if all_records.len() > records.len() {
            for record in &all_records {
                for action in &record.decisions {
                    if !action.success {
                        continue;
                    }

                    let mut side = "";
                    if action.action == Action::OPENLONG || action.action == Action::CLOSELONG {
                        side = "long";
                    } else if action.action == Action::OPENSHORT
                        || action.action == Action::CLOSESHORT
                    {
                        side = "short";
                    }

                    let pos_key = format!("{}_{}", &action.symbol, side);

                    match action.action {
                        Action::OPENLONG | Action::OPENSHORT => {
                            open_positions.insert(
                                pos_key,
                                HashMap::from([
                                    ("side".to_string(), json!(side)),
                                    ("open_price".to_string(), json!(action.price)),
                                    ("open_time".to_string(), json!(action.timestamp)),
                                    ("quantity".to_string(), json!(action.quantity)),
                                    ("leverage".to_string(), json!(action.leverage)),
                                ]),
                            );
                        }
                        Action::CLOSELONG | Action::CLOSESHORT => {
                            open_positions.remove(&pos_key);
                        }
                    }
                }
            }
        }

        for record in &records {
            for action in &record.decisions {
                if !action.success {
                    continue;
                }

                let mut side = "";
                if action.action == Action::OPENLONG || action.action == Action::OPENSHORT {
                    side = "long";
                } else if action.action == Action::OPENSHORT || action.action == Action::CLOSESHORT
                {
                    side = "short";
                }

                let pos_key = format!("{}_{}", &action.symbol, side);

                match action.action {
                    Action::OPENLONG | Action::OPENSHORT => {
                        open_positions.insert(
                            pos_key,
                            HashMap::from([
                                ("side".to_string(), json!(side)),
                                ("open_price".to_string(), json!(action.price)),
                                ("open_time".to_string(), json!(action.timestamp)),
                                ("quantity".to_string(), json!(action.quantity)),
                                ("leverage".to_string(), json!(action.leverage)),
                            ]),
                        );
                    }
                    Action::CLOSELONG | Action::CLOSESHORT => {
                        // 查找对应的开仓记录（可能来自预填充或当前窗口）
                        if let Some(open_pos) = open_positions.get(&pos_key) {
                            let open_price = open_pos["open_price"]
                                .as_f64()
                                .expect("open_price must be a float");

                            let open_time = open_pos["open_time"]
                                .as_str()
                                .expect("open_time must be a string")
                                .parse::<chrono::DateTime<chrono::Utc>>()
                                .expect("invalid time format");

                            let side = open_pos["side"]
                                .as_str()
                                .expect("side must be a string")
                                .to_string();

                            let quantity = open_pos["quantity"]
                                .as_f64()
                                .expect("quantity must be a float");

                            let leverage = open_pos["leverage"]
                                .as_i64()
                                .expect("leverage must be an integer")
                                as i32;

                            let mut pnl = 0_f64;
                            if side == "long" {
                                pnl = quantity * (action.price - open_price);
                            } else {
                                pnl = quantity * (open_price - action.price);
                            }

                            // 计算盈亏百分比（相对保证金）
                            let position_value = quantity * open_price;
                            let margin_used = position_value / f64::from(leverage);
                            let mut pnl_pct = 0_f64;
                            if margin_used > 0_f64 {
                                pnl_pct = (pnl / margin_used) * 100_f64;
                            }

                            let outcome = TradeOutcome {
                                symbol: action.symbol.to_string(),
                                side: Side::from_str(side.as_str()).unwrap(),
                                quantity: quantity,
                                leverage: leverage,
                                open_price: open_price,
                                close_price: action.price,
                                
                            }
                        }
                    }
                }
            }
        }

        {}
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Statistics {
    pub total_cycles: i32,
    pub successful_cycles: i32,
    pub failed_cycles: i32,
    pub total_open_positions: i32,
    pub total_close_positions: i32,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct TradeOutcome {
    symbol: String,
    side: Side,
    quantity: f64,
    leverage: i32,
    open_price: f64,
    close_price: f64,
    position_value: f64,
    margin_used: f64,
    pn_l: f64,
    pn_l_pct: f64,
    duration: String,
    open_time: DateTime<Utc>,
    close_time: DateTime<Utc>,
    was_stop_loss: bool,
}

#[derive(Debug, Deserialize, Serialize, Default)]
enum Side {
    #[default]
    SHORT,
    LONG,
}

impl FromStr for Side {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "short" => Ok(Side::SHORT),
            "long" => Ok(Side::LONG),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct PerformanceAnalysis {
    total_trades: i32,
    winning_trades: i32,
    losing_trades: i32,
    win_rate: f64,
    avg_win: f64,
    avg_loss: f64,
    profit_factor: f64,
    sharpe_ratio: f64,
    recent_trades: Vec<TradeOutcome>,
    symbol_stats: HashMap<String, SymbolPerformance>,
    best_symbol: String,
    worst_symbol: String,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct SymbolPerformance {
    symbol: String,
    total_trades: i32,
    winning_trades: i32,
    losing_trades: i32,
    win_rate: f64,
    total_pn_l: f64,
    avg_pn_l: f64,
}

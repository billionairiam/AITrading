use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row, SqlitePool, error::DatabaseError, sqlite::SqliteError};

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(db_path: &str) -> Result<Self> {
        let pool = SqlitePool::connect(db_path)
            .await
            .with_context(|| format!("Failed to open or create database at '{}'", db_path))?;

        let database = Self { pool };

        database.create_tables().await.context("创建表失败")?;
        database
            .init_default_data()
            .await
            .context("初始化默认数据失败")?;

        Ok(database)
    }

    pub async fn create_tables(&self) -> Result<()> {
        log::info!("Setting up database schema...");

        // A transaction ensures that all schema setup operations succeed or none do.
        let mut tx = self.pool.begin().await?;

        const queries: &[&str] = &[
            // AI模型配置表
            r#"
            CREATE TABLE IF NOT EXISTS ai_models (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL DEFAULT 'default',
                name TEXT NOT NULL,
                provider TEXT NOT NULL,
                enabled BOOLEAN DEFAULT 0,
                api_key TEXT DEFAULT '',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            )
            "#,
            // 交易所配置表
            r#"
            CREATE TABLE IF NOT EXISTS exchanges (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL DEFAULT 'default',
                name TEXT NOT NULL,
                type TEXT NOT NULL, -- 'cex' or 'dex'
                enabled BOOLEAN DEFAULT 0,
                api_key TEXT DEFAULT '',
                secret_key TEXT DEFAULT '',
                testnet BOOLEAN DEFAULT 0,
                -- Hyperliquid 特定字段
                hyperliquid_wallet_addr TEXT DEFAULT '',
                -- Aster 特定字段
                aster_user TEXT DEFAULT '',
                aster_signer TEXT DEFAULT '',
                aster_private_key TEXT DEFAULT '',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            )
            "#,
            // 用户信号源配置表
            r#"
            CREATE TABLE IF NOT EXISTS user_signal_sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT NOT NULL,
                coin_pool_url TEXT DEFAULT '',
                oi_top_url TEXT DEFAULT '',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                UNIQUE(user_id)
            )
            "#,
            // 交易员配置表
            r#"
            CREATE TABLE IF NOT EXISTS traders (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL DEFAULT 'default',
                name TEXT NOT NULL,
                ai_model_id TEXT NOT NULL,
                exchange_id TEXT NOT NULL,
                initial_balance REAL NOT NULL,
                scan_interval_minutes INTEGER DEFAULT 3,
                is_running BOOLEAN DEFAULT 0,
                btc_eth_leverage INTEGER DEFAULT 5,
                altcoin_leverage INTEGER DEFAULT 5,
                trading_symbols TEXT DEFAULT '',
                use_coin_pool BOOLEAN DEFAULT 0,
                use_oi_top BOOLEAN DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY (ai_model_id) REFERENCES ai_models(id),
                FOREIGN KEY (exchange_id) REFERENCES exchanges(id)
            )
            "#,
            // 用户表
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                otp_secret TEXT,
                otp_verified BOOLEAN DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
            // 系统配置表
            r#"
            CREATE TABLE IF NOT EXISTS system_config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
            // 内测码表
            r#"
            CREATE TABLE IF NOT EXISTS beta_codes (
                code TEXT PRIMARY KEY,
                used BOOLEAN DEFAULT 0,
                used_by TEXT DEFAULT '',
                used_at DATETIME DEFAULT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        ];

        // 触发器：自动更新 updated_at
        let triggers: &[&str] = &[
            r#"
            CREATE TRIGGER IF NOT EXISTS update_users_updated_at
			AFTER UPDATE ON users
			BEGIN
				UPDATE users SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
			END
            "#,
            r#"
            CREATE TRIGGER IF NOT EXISTS update_ai_models_updated_at
			AFTER UPDATE ON ai_models
			BEGIN
				UPDATE ai_models SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
			END
            "#,
            r#"
            CREATE TRIGGER IF NOT EXISTS update_exchanges_updated_at
			AFTER UPDATE ON exchanges
			BEGIN
				UPDATE exchanges SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
			END
            "#,
            r#"
            CREATE TRIGGER IF NOT EXISTS update_traders_updated_at
			AFTER UPDATE ON traders
			BEGIN
				UPDATE traders SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
			END
            "#,
            r#"
            CREATE TRIGGER IF NOT EXISTS update_user_signal_sources_updated_at
			AFTER UPDATE ON user_signal_sources
			BEGIN
				UPDATE user_signal_sources SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
			END
            "#,
            r#"
            CREATE TRIGGER IF NOT EXISTS update_system_config_updated_at
			AFTER UPDATE ON system_config
			BEGIN
				UPDATE system_config SET updated_at = CURRENT_TIMESTAMP WHERE key = NEW.key;
			END
            "#,
        ];

        for query in queries.iter().chain(triggers) {
            sqlx::query(query).execute(&mut *tx).await?;
        }

        tx.commit()
            .await
            .context("Failed to commit schema creation transaction")?;

        let alter_quries: &[&str] = &[
            r#"ALTER TABLE exchanges ADD COLUMN hyperliquid_wallet_addr TEXT DEFAULT ''"#,
            r#"ALTER TABLE exchanges ADD COLUMN aster_user TEXT DEFAULT ''"#,
            r#"ALTER TABLE exchanges ADD COLUMN aster_signer TEXT DEFAULT ''"#,
            r#"ALTER TABLE exchanges ADD COLUMN aster_private_key TEXT DEFAULT ''"#,
            r#"ALTER TABLE traders ADD COLUMN custom_prompt TEXT DEFAULT ''"#,
            r#"ALTER TABLE traders ADD COLUMN override_base_prompt BOOLEAN DEFAULT 0"#,
            r#"ALTER TABLE traders ADD COLUMN is_cross_margin BOOLEAN DEFAULT 1"#,
            r#"ALTER TABLE traders ADD COLUMN use_default_coins BOOLEAN DEFAULT 1"#,
            r#"ALTER TABLE traders ADD COLUMN custom_coins TEXT DEFAULT ''"#,
            r#"ALTER TABLE traders ADD COLUMN btc_eth_leverage INTEGER DEFAULT 5"#,
            r#"ALTER TABLE traders ADD COLUMN altcoin_leverage INTEGER DEFAULT 5"#,
            r#"ALTER TABLE traders ADD COLUMN trading_symbols TEXT DEFAULT ''"#,
            r#"ALTER TABLE traders ADD COLUMN use_coin_pool BOOLEAN DEFAULT 0"#,
            r#"ALTER TABLE traders ADD COLUMN use_oi_top BOOLEAN DEFAULT 0"#,
            r#"ALTER TABLE traders ADD COLUMN system_prompt_template TEXT DEFAULT 'default'"#,
            r#"ALTER TABLE ai_models ADD COLUMN custom_api_url TEXT DEFAULT ''"#,
            r#"ALTER TABLE ai_models ADD COLUMN custom_model_name TEXT DEFAULT ''"#,
        ];

        for query in alter_quries {
            match sqlx::query(&query).execute(&self.pool).await {
                Ok(_) => log::debug!("Successfully applied alteration: {}", query),
                Err(sqlx::Error::Database(db_err)) => {
                    let sqlite_err = db_err.downcast_ref::<SqliteError>();
                    // Now we know it's a SqliteError. Check the message.
                    if sqlite_err.message().contains("duplicate column name") {
                        log::trace!("Column already exists, skipping alteration: {}", query);
                    } else {
                        // It's a different SQLite error. We need to own the message
                        // before passing it to anyhow.
                        let error_message: String = sqlite_err.message().to_string(); // <-- THE FIX

                        return Err(anyhow::anyhow!(error_message)
                            .context(format!("Failed to execute alteration query: {}", query)));
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        if let Err(e) = self.migrate_exchange_table().await {
            log::warn!("⚠️ 迁移exchanges表失败: {e:?}");
        }

        Ok(())
    }

    pub async fn init_default_data(&self) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("Failed to begin transaction for default data initialization")?;

        const AI_MODELS: &[(&str, &str, &str)] = &[
            ("deepseek", "DeepSeek", "deepseek"),
            ("qwen", "Qwen", "qwen"),
        ];

        for &(id, name, provider) in AI_MODELS {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO ai_models (id, user_id, name, provider, enabled) 
                VALUES (?, 'default', ?, ?, 0)
            "#,
            )
            .bind(id)
            .bind(name)
            .bind(provider)
            .execute(&mut *tx)
            .await
            .context("Failed to initialize default AI models")?;
        }

        const EXCHANGES: &[(&str, &str, &str)] = &[
            ("binance", "Binance Futures", "binance"),
            ("hyperliquid", "Hyperliquid", "hyperliquid"),
            ("aster", "Aster DEX", "aster"),
        ];

        for &(id, name, typ) in EXCHANGES {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO exchanges (id, user_id, name, type, enabled) 
                VALUES (?, 'default', ?, ?, 0)
            "#,
            )
            .bind(id)
            .bind(name)
            .bind(typ)
            .execute(&mut *tx)
            .await
            .context("Failed to initialize default exchanges")?;
        }

        const SYSTEM_CONFIGS: &[(&str, &str)] = &[
            ("admin_mode", "true"),
            ("beta_mode", "false"),
            ("api_server_port", "8080"),
            ("use_default_coins", "true"),
            (
                "default_coins",
                r#"["BTCUSDT","ETHUSDT","SOLUSDT","BNBUSDT","XRPUSDT","DOGEUSDT","ADAUSDT","HYPEUSDT"]"#,
            ),
            ("max_daily_loss", "10.0"),
            ("max_drawdown", "20.0"),
            ("stop_trading_minutes", "60"),
            ("btc_eth_leverage", "5"),
            ("altcoin_leverage", "5"),
            ("jwt_secret", ""),
        ];

        for &(key, value) in SYSTEM_CONFIGS {
            sqlx::query("INSERT OR IGNORE INTO system_config (key, value) VALUES (?, ?)")
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await
                .context("Failed to initialize system configurations")?;
        }

        tx.commit()
            .await
            .context("Failed to commit transaction for default data")?;

        Ok(())
    }

    pub async fn migrate_exchange_table(&self) -> Result<()> {
        // 检查是否已经迁移过
        let pk_count: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('exchanges') WHERE pk > 0",
        )
        .fetch_one(&self.pool)
        .await
        {
            Ok(count) => count,
            Err(sqlx::Error::Database(db_err)) if db_err.message().contains("no such table") => {
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };

        // 如果已经迁移过，直接返回
        if pk_count >= 2 {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .context("Failed to begin migration transaction")?;

        log::info!("🔄 开始迁移exchanges表...");

        // 创建新的exchanges表，使用复合主键
        sqlx::query(
            r#"
            CREATE TABLE exchanges_new (
                id TEXT NOT NULL,
                user_id TEXT NOT NULL DEFAULT 'default',
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                enabled BOOLEAN DEFAULT 0,
                api_key TEXT DEFAULT '',
                secret_key TEXT DEFAULT '',
                testnet BOOLEAN DEFAULT 0,
                hyperliquid_wallet_addr TEXT DEFAULT '',
                aster_user TEXT DEFAULT '',
                aster_signer TEXT DEFAULT '',
                aster_private_key TEXT DEFAULT '',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (id, user_id),
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            )
        "#,
        )
        .execute(&mut *tx)
        .await
        .context("Failed to create new 'exchanges_new' table")?;

        // 复制数据到新表
        sqlx::query("INSERT INTO exchanges_new SELECT * FROM exchanges")
            .execute(&mut *tx)
            .await
            .context("Failed to copy data from 'exchanges' to 'exchanges_new'")?;

        // 删除旧表
        sqlx::query("DROP TABLE exchanges")
            .execute(&mut *tx)
            .await
            .context("Failed to drop old 'exchanges' table")?;

        // 重命名新表
        sqlx::query("ALTER TABLE exchanges_new RENAME TO exchanges")
            .execute(&mut *tx)
            .await
            .context("Failed to rename 'exchanges_new' to 'exchanges'")?;

        // 重新创建触发器
        sqlx::query(
            r#"
            CREATE TRIGGER IF NOT EXISTS update_exchanges_updated_at
                AFTER UPDATE ON exchanges
                BEGIN
                    UPDATE exchanges SET updated_at = CURRENT_TIMESTAMP 
                    WHERE id = NEW.id AND user_id = NEW.user_id;
                END
        "#,
        )
        .execute(&mut *tx)
        .await
        .context("Failed to recreate 'update_exchanges_updated_at' trigger")?;

        tx.commit()
            .await
            .context("Failed to commit migration transaction")?;

        log::info!("✅ exchanges表迁移完成");

        Ok(())
    }

    pub async fn create_user(&self, user: &User) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO users (id, email, password_hash, otp_secret, otp_verified)
            VALUES (?, ?, ?, ?, ?)"#,
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(&user.otp_secret)
        .bind(user.otp_verified)
        .execute(&self.pool)
        .await
        .context("failed to create user")?;

        Ok(())
    }

    pub async fn ensure_admin_user(&self) -> Result<()> {
        let result = sqlx::query(
            r#"
                INSERT OR IGNORE INTO users (id, email, password_hash, otp_secret, otp_verified)
                VALUES ('admin', 'admin@localhost', '', '', 1)
            "#,
        )
        .execute(&self.pool)
        .await
        .context("Failed to ensure admin user exists")?;

        if result.rows_affected() > 0 {
            log::info!("Admin user did not exist and was created.");
        } else {
            log::info!("Admin user already exists.");
        }

        Ok(())
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let user_result = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await;

        match user_result {
            Ok(user) => Ok(user),
            Err(e) => {
                // If an error occurs, wrap it with context for better debugging.
                Err(e).context(format!("Failed to fetch user with email: {}", email))
            }
        }
    }

    pub async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        let user_result = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await;

        match user_result {
            Ok(user) => Ok(user),
            Err(e) => {
                // If an error occurs, wrap it with context for better debugging.
                Err(e).context(format!("Failed to fetch user with id: {}", user_id))
            }
        }
    }

    pub async fn get_all_users_id(&self) -> Result<Vec<String>> {
        let user_ids = sqlx::query_scalar::<_, String>("SELECT id FROM users ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .context("Failed to fetch all user IDs from the database")?;

        Ok(user_ids)
    }

    // 更新用户OTP验证状态
    pub async fn update_user_ota_verified(&self, user_id: &str, verified: bool) -> Result<()> {
        let result = sqlx::query("UPDATE users SET otp_verified = ? WHERE id =?")
            .bind(user_id)
            .bind(verified)
            .execute(&self.pool)
            .await
            .context("Failed to update user OTP verification status")?;

        if result.rows_affected() == 0 {
            log::warn!(
                "Attempted to update OTP status for non-existent user_id: {}",
                user_id
            );
        }

        Ok(())
    }

    // 获取用户的AI模型配置
    pub async fn get_aimodels(&self, user_id: &str) -> Result<Vec<AIModelConfig>> {
        let results = sqlx::query_as::<_, AIModelConfig>(
            r#"SELECT id, user_id, name, provider, enabled, api_key,
		        COALESCE(custom_api_url, '') as custom_api_url,
		        COALESCE(custom_model_name, '') as custom_model_name,
		        created_at, updated_at
		    FROM ai_models WHERE user_id = ? ORDER BY id"#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await;

        match results {
            Ok(aimodels) => Ok(aimodels),
            Err(e) => Err(e).context(format!(
                "Failed to fetch aimodels with user_id: {}",
                user_id
            )),
        }
    }

    // 更新AI模型配置，如果不存在则创建用户特定配置
    pub async fn update_aimodel(
        &self,
        user_id: &str,
        id: &str,
        enabled: bool,
        api_key: &str,
        custom_api_url: &str,
        custom_model_name: &str,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("Failed to begin transaction")?;

        // 先尝试精确匹配 ID（新版逻辑，支持多个相同 provider 的模型）
        let maybe_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM ai_models WHERE user_id = ? AND id = ? LIMIT 1",
        )
        .bind(user_id)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;

        // 找到了现有配置（精确匹配 ID），更新它
        if let Some(existing_id) = maybe_id {
            sqlx::query(
                r#"UPDATE ai_models SET enabled = ?, api_key = ?, custom_api_url = ?, custom_model_name = ?, updated_at = datetime('now')
			        WHERE id = ? AND user_id = ?"#
            )
            .bind(enabled)
            .bind(api_key)
            .bind(custom_api_url)
            .bind(custom_model_name)
            .bind(&existing_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            return Ok(());
        }

        // ID 不存在，尝试兼容旧逻辑：将 id 作为 provider 查找
        let provider_as_id = id;
        let maybe_id_by_provider = sqlx::query_scalar::<_, String>(
            "SELECT id FROM ai_models WHERE user_id = ? AND provider = ? LIMIT 1",
        )
        .bind(user_id)
        .bind(provider_as_id)
        .fetch_optional(&mut *tx)
        .await?;

        // 找到了现有配置（通过 provider 匹配，兼容旧版），更新它
        if let Some(existing_id) = maybe_id_by_provider {
            log::info!(
                "⚠️  使用旧版 provider 匹配更新模型: {} -> {}",
                id,
                &existing_id
            );
            sqlx::query(r#"
                UPDATE ai_models 
                SET enabled = ?, api_key = ?, custom_api_url = ?, custom_model_name = ?, updated_at = datetime('now')
                WHERE id = ? AND user_id = ?
                "#,)
                .bind(enabled)
                .bind(api_key)
                .bind(custom_api_url)
                .bind(custom_model_name)
                .bind(&existing_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            return Ok(());
        }

        // 没有找到任何现有配置，创建新的
        // 推断 provider（从 id 中提取，或者直接使用 id）
        let provider = if id == "deepseek" || id == "qwen" {
            id.to_string()
        } else {
            id.split("_").last().unwrap_or(id).to_string()
        };

        // 获取模型的基本信息
        let maybe_name = sqlx::query_scalar::<_, String>(
            "SELECT name FROM ai_models WHERE provider = ? LIMIT 1",
        )
        .bind(&provider)
        .fetch_optional(&mut *tx)
        .await?;

        let name = match maybe_name {
            Some(n) => n,
            None => match provider.as_str() {
                "deepseek" => "Deepseek AI".to_string(),
                "qwen" => "Qwen AI".to_string(),
                p => format!("{} AI", p),
            },
        };

        let new_model_id = if id == provider {
            // If the input ID was just a provider, create a user-specific ID.
            format!("{}_{}", user_id, provider)
        } else {
            // Otherwise, use the provided ID as is.
            id.to_string()
        };

        log::info!(
            "✓ 创建新的 AI 模型配置: ID={}, Provider={}, Name={}",
            &new_model_id,
            provider,
            &name
        );

        sqlx::query(
            r#"
            INSERT INTO ai_models (id, user_id, name, provider, enabled, api_key, custom_api_url, custom_model_name, created_at, updated_at)
		    VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            "#
        )
        .bind(&new_model_id)
        .bind(user_id)
        .bind(&name)
        .bind(&provider)
        .bind(enabled)
        .bind(api_key)
        .bind(custom_api_url)
        .bind(custom_model_name)
        .execute(&mut *tx)
        .await?;

        tx.commit().await.context("failed to commit update model")?;

        Ok(())
    }

    pub async fn get_exchanges(&self, user_id: &str) -> Result<Vec<ExchangeConfig>> {
        let ecs = sqlx::query_as::<_, ExchangeConfig>(
            r#"
            SELECT id, user_id, name, type, enabled, api_key, secret_key, testnet, 
		       COALESCE(hyperliquid_wallet_addr, '') as hyperliquid_wallet_addr,
		       COALESCE(aster_user, '') as aster_user,
		       COALESCE(aster_signer, '') as aster_signer,
		       COALESCE(aster_private_key, '') as aster_private_key,
		       created_at, updated_at 
		FROM exchanges WHERE user_id = ? ORDER BY id
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(ecs)
    }

    pub async fn update_exchange(
        &self,
        user_id: &str,
        id: &str,
        enabled: bool,
        api_key: &str,
        secret_key: &str,
        testnet: bool,
        hyperliquid_wallet_addr: &str,
        aster_user: &str,
        aster_signer: &str,
        aster_private_key: &str,
    ) -> Result<()> {
        log::info!(
            "🔧 UpdateExchange: userID={}, id={}, enabled={}",
            user_id,
            id,
            enabled
        );

        let result = sqlx::query(
            r#"
            UPDATE exchanges SET enabled = ?, api_key = ?, secret_key = ?, testnet = ?, 
		       hyperliquid_wallet_addr = ?, aster_user = ?, aster_signer = ?, aster_private_key = ?, updated_at = datetime('now')
		    WHERE id = ? AND user_id = ?
            "#)
            .bind(enabled)
            .bind(api_key)
            .bind(secret_key)
            .bind(hyperliquid_wallet_addr)
            .bind(aster_user)
            .bind(aster_signer)
            .bind(aster_private_key)
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() > 0 {
            log::info!("📊 UpdateExchange: 影响行数 = {}", result.rows_affected());
        } else {
            let (name, typ) = match id {
                "binance" => ("Binance Futures", "cex"),
                "hyperliquid" => ("Hyperliquid", "dex"),
                "aster" => ("Aster DEX", "dex"),
                _ => ("-", "cex"),
            };

            let final_name = if name == "-" {
                format!("{} Exchange", id)
            } else {
                name.to_string()
            };

            log::info!(
                "🆕 UpdateExchange: 创建新记录 ID={}, name={}, type={}",
                id,
                name,
                typ
            );

            // 创建用户特定的配置，使用原始的交易所ID
            sqlx::query(
                r#"
                INSERT INTO exchanges (id, user_id, name, type, enabled, api_key, secret_key, testnet, 
			                       hyperliquid_wallet_addr, aster_user, aster_signer, aster_private_key, created_at, updated_at)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
                "#,
            )
            .bind(id)
            .bind(user_id)
            .bind(&final_name)
            .bind(typ)
            .bind(enabled)
            .bind(api_key)
            .bind(secret_key)
            .bind(testnet)
            .bind(hyperliquid_wallet_addr)
            .bind(aster_user)
            .bind(aster_signer)
            .bind(aster_private_key)
            .execute(&self.pool)
            .await
            .map(|_| {
                log::info!("✅ UpdateExchange: created record successfully");
            })
            .map_err(|e| {
                log::error!("❌ UpdateExchange: failed to create record: {}", e);
                e
            })?;
        }
        Ok(())
    }

    pub async fn create_ai_model(
        &self,
        user_id: &str,
        id: &str,
        name: &str,
        provider: &str,
        enabled: bool,
        api_key: &str,
        custom_api_url: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO ai_models (id, user_id, name, provider, enabled, api_key, custom_api_url) 
		    VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(provider)
        .bind(enabled)
        .bind(api_key)
        .bind(custom_api_url)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn create_exchange(
        &self,
        user_id: &str,
        id: &str,
        name: &str,
        typ: &str,
        enabled: bool,
        api_key: &str,
        secret_key: &str,
        testnet: bool,
        hyperliquid_wallet_addr: &str,
        aster_user: &str,
        aster_signer: &str,
        aster_private_key: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO exchanges (id, user_id, name, type, enabled, api_key, secret_key, testnet, hyperliquid_wallet_addr, aster_user, aster_signer, aster_private_key) 
		    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(typ)
        .bind(enabled)
        .bind(api_key)
        .bind(secret_key)
        .bind(testnet)
        .bind(hyperliquid_wallet_addr)
        .bind(aster_user)
        .bind(aster_signer)
        .bind(aster_private_key)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn create_trader(&self, trader: &TraderRecord) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO traders (id, user_id, name, ai_model_id, exchange_id, initial_balance, scan_interval_minutes, is_running, btc_eth_leverage, altcoin_leverage, trading_symbols, use_coin_pool, use_oi_top, custom_prompt, override_base_prompt, system_prompt_template, is_cross_margin)
		    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&trader.id)
        .bind(&trader.user_id)
        .bind(&trader.name)
        .bind(&trader.ai_model_id)
        .bind(&trader.exchange_id)
        .bind(&trader.initial_balance)
        .bind(&trader.scan_interval_minutes)
        .bind(&trader.is_running)
        .bind(&trader.btc_eth_leverage)
        .bind(&trader.altcoin_leverage)
        .bind(&trader.trading_symbols)
        .bind(&trader.use_coin_pool)
        .bind(&trader.use_oi_top)
        .bind(&trader.custom_prompt)
        .bind(&trader.override_base_prompt)
        .bind(&trader.system_prompt_template)
        .bind(&trader.is_cross_margin)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_traders(&self, user_id: &str) -> Result<Vec<TraderRecord>> {
        let trs = sqlx::query_as::<_, TraderRecord>(
            r#"
            SELECT id, user_id, name, ai_model_id, exchange_id, initial_balance, scan_interval_minutes, is_running,
		       COALESCE(btc_eth_leverage, 5) as btc_eth_leverage, COALESCE(altcoin_leverage, 5) as altcoin_leverage,
		       COALESCE(trading_symbols, '') as trading_symbols,
		       COALESCE(use_coin_pool, 0) as use_coin_pool, COALESCE(use_oi_top, 0) as use_oi_top,
		       COALESCE(custom_prompt, '') as custom_prompt, COALESCE(override_base_prompt, 0) as override_base_prompt,
		       COALESCE(system_prompt_template, 'default') as system_prompt_template,
		       COALESCE(is_cross_margin, 1) as is_cross_margin, created_at, updated_at
		    FROM traders WHERE user_id = ? ORDER BY created_at DESC
            "#
        ).bind(user_id).fetch_all(&self.pool).await?;

        Ok(trs)
    }

    pub async fn update_trader_status(&self, user_id: &str, is_running: bool) -> Result<()> {
        sqlx::query("UPDATE traders SET is_running = ? WHERE id = ? AND user_id = ?")
            .bind(user_id)
            .bind(is_running)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn update_trader(&self, trader: &TraderRecord) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE traders SET
			name = ?, ai_model_id = ?, exchange_id = ?, initial_balance = ?,
			scan_interval_minutes = ?, btc_eth_leverage = ?, altcoin_leverage = ?,
			trading_symbols = ?, custom_prompt = ?, override_base_prompt = ?,
			system_prompt_template = ?, is_cross_margin = ?, updated_at = CURRENT_TIMESTAMP
		    WHERE id = ? AND user_id = ?
            "#,
        )
        .bind(&trader.name)
        .bind(&trader.ai_model_id)
        .bind(&trader.exchange_id)
        .bind(&trader.initial_balance)
        .bind(&trader.scan_interval_minutes)
        .bind(&trader.btc_eth_leverage)
        .bind(&trader.altcoin_leverage)
        .bind(&trader.trading_symbols)
        .bind(&trader.custom_prompt)
        .bind(&trader.override_base_prompt)
        .bind(&trader.system_prompt_template)
        .bind(&trader.is_cross_margin)
        .bind(&trader.id)
        .bind(&trader.user_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn update_trader_custom_prompt(
        &self,
        user_id: &str,
        id: &str,
        custom_prompt: &str,
        override_base: bool,
    ) -> Result<()> {
        sqlx::query("UPDATE traders SET custom_prompt = ?, override_base_prompt = ? WHERE id = ? AND user_id = ?")
            .bind(user_id)
            .bind(id)
            .bind(custom_prompt)
            .bind(override_base)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn delete_trader(&self, user_id: &str, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM traders WHERE id = ? AND user_id = ?")
            .bind(user_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn get_trader_config(
        &self,
        user_id: &str,
        trader_id: &str,
    ) -> Result<(TraderRecord, AIModelConfig, ExchangeConfig)> {
        let row  = sqlx::query(
            r#"
            SELECT 
                t.id, t.user_id, t.name, t.ai_model_id, t.exchange_id, t.initial_balance, t.scan_interval_minutes, t.is_running, t.created_at, t.updated_at,
                a.id, a.user_id, a.name, a.provider, a.enabled, a.api_key, a.created_at, a.updated_at,
                e.id, e.user_id, e.name, e.type, e.enabled, e.api_key, e.secret_key, e.testnet,
                COALESCE(e.hyperliquid_wallet_addr, '') as hyperliquid_wallet_addr,
                COALESCE(e.aster_user, '') as aster_user,
                COALESCE(e.aster_signer, '') as aster_signer,
                COALESCE(e.aster_private_key, '') as aster_private_key,
                e.created_at, e.updated_at
            FROM traders t
            JOIN ai_models a ON t.ai_model_id = a.id AND t.user_id = a.user_id
            JOIN exchanges e ON t.exchange_id = e.id AND t.user_id = e.user_id
            WHERE t.id = ? AND t.user_id = ?
            "#,
        ).bind(trader_id).bind(user_id).fetch_one(&self.pool).await?;

        // Manually map the aliased columns to the structs
        let trader = TraderRecord {
            id: row.try_get("t_id")?,
            user_id: row.try_get("t_user_id")?,
            name: row.try_get("t_name")?,
            ai_model_id: row.try_get("ai_model_id")?,
            exchange_id: row.try_get("exchange_id")?,
            initial_balance: row.try_get("initial_balance")?,
            scan_interval_minutes: row.try_get("scan_interval_minutes")?,
            is_running: row.try_get("is_running")?,
            created_at: row.try_get("t_created_at")?,
            updated_at: row.try_get("t_updated_at")?,
            ..Default::default()
        };

        let ai_model = AIModelConfig {
            id: row.try_get("a_id")?,
            user_id: row.try_get("a_user_id")?,
            name: row.try_get("a_name")?,
            provider: row.try_get("provider")?,
            enabled: row.try_get("a_enabled")?,
            api_key: row.try_get("a_api_key")?,
            created_at: row.try_get("a_created_at")?,
            updated_at: row.try_get("a_updated_at")?,
            ..Default::default()
        };

        let exchange = ExchangeConfig {
            id: row.try_get("e_id")?,
            user_id: row.try_get("e_user_id")?,
            name: row.try_get("e_name")?,
            exchange_type: row.try_get("e_type")?,
            enabled: row.try_get("e_enabled")?,
            api_key: row.try_get("e_api_key")?,
            secret_key: row.try_get("secret_key")?,
            testnet: row.try_get("testnet")?,
            hyperliquid_wallet_addr: row.try_get("hyperliquid_wallet_addr")?,
            aster_user: row.try_get("aster_user")?,
            aster_signer: row.try_get("aster_signer")?,
            aster_private_key: row.try_get("aster_private_key")?,
            created_at: row.try_get("e_created_at")?,
            updated_at: row.try_get("e_updated_at")?,
            ..Default::default()
        };

        Ok((trader, ai_model, exchange))
    }

    pub async fn get_system_config(&self, key: &str) -> Result<String> {
        let config: String = sqlx::query_scalar("SELECT value FROM system_config WHERE key = ?")
            .bind(key)
            .fetch_one(&self.pool)
            .await?;

        Ok(config)
    }

    pub async fn set_system_config(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO system_config (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn create_user_signal_source(
        &self,
        user_id: &str,
        coin_pool_url: &str,
        oi_top_url: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO user_signal_sources (user_id, coin_pool_url, oi_top_url, updated_at)
		    VALUES (?, ?, ?, CURRENT_TIMESTAMP)
        "#)
        .bind(user_id)
        .bind(coin_pool_url)
        .bind(oi_top_url)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_user_signal_source(&self, user_id: &str) -> Result<UserSignalSource> {
        let usr = sqlx::query_as::<_, UserSignalSource>(
            r#"
            SELECT id, user_id, coin_pool_url, oi_top_url, created_at, updated_at
		    FROM user_signal_sources WHERE user_id = ?
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(usr)
    }

    pub async fn update_user_signal_source(
        &self,
        user_id: &str,
        coin_pool_url: &str,
        oi_top_url: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE user_signal_sources SET coin_pool_url = ?, oi_top_url = ?, updated_at = CURRENT_TIMESTAMP
		    WHERE user_id = ?
            "#
        )
        .bind(coin_pool_url)
        .bind(oi_top_url)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_custom_coins(&self) -> Result<Vec<String>> {
        let query =
            "SELECT GROUP_CONCAT(custom_coins SEPARATOR ',') FROM traders WHERE custom_coins != ''";

        let raw_result: Option<String> = match sqlx::query_scalar(query).fetch_one(&self.pool).await
        {
            Ok(res) => res, // Can be None (if NULL) or Some(String)
            Err(e) => {
                log::error!("Error fetching custom_coins: {:?}", e);
                None
            }
        };

        Ok(vec![])
    }
}

// User 用户配置
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, Default)]
pub struct User {
    pub id: String,
    pub email: String,

    #[serde(skip_serializing)]
    pub password_hash: String,

    #[serde(skip_serializing)]
    pub otp_secret: String,

    pub otp_verified: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

// AIModelConfig AI模型配置
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, Default)]
pub struct AIModelConfig {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub provider: String,
    pub enabled: bool,

    #[serde(rename = "apiKey")]
    pub api_key: String,

    #[serde(rename = "CustomAPIURL")]
    pub custom_api_url: String,

    #[serde(rename = "CustomModelName")]
    pub custom_model_name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

// ExchangeConfig 交易所配置
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, Default)]
pub struct ExchangeConfig {
    pub id: String,
    pub user_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub exchange_type: String,

    pub enabled: bool,

    #[serde(rename = "apiKey")]
    pub api_key: String,

    #[serde(rename = "SecretKey")]
    pub secret_key: String,
    pub testnet: bool,

    #[serde(rename = "hyperliquidWalletAddr")]
    pub hyperliquid_wallet_addr: String,

    #[serde(rename = "asterUser")]
    pub aster_user: String,

    #[serde(rename = "asterSigner")]
    pub aster_signer: String,

    #[serde(rename = "asterPrivateKey")]
    pub aster_private_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// TraderRecord 交易员配置（数据库实体）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, Default)]
pub struct TraderRecord {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub ai_model_id: String,
    pub exchange_id: String,
    pub initial_balance: f64,
    pub scan_interval_minutes: i32,
    pub is_running: bool,
    pub btc_eth_leverage: i32,          // BTC/ETH杠杆倍数
    pub altcoin_leverage: i32,          // 山寨币杠杆倍数
    pub trading_symbols: String,        // 交易币种，逗号分隔
    pub use_coin_pool: bool,            // 是否使用COIN POOL信号源
    pub use_oi_top: bool,               // 是否使用OI TOP信号源
    pub custom_prompt: String,          // 自定义交易策略prompt
    pub override_base_prompt: bool,     // 是否覆盖基础prompt
    pub system_prompt_template: String, // 是否为全仓模式（true=全仓，false=逐仓）
    pub is_cross_margin: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// UserSignalSource 用户信号源配置
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UserSignalSource {
    pub id: i32,
    pub user_id: String,
    pub coin_pool_url: String,
    pub oi_top_url: String,
    pub created_at: DateTime<Utc>,
    pub update_at: DateTime<Utc>,
}

pub fn generate_otp_secret() -> String {
    let mut secret_bytes = [0u8; 20];

    rand::thread_rng().fill_bytes(&mut secret_bytes);

    base32::encode(base32::Alphabet::RFC4648 { padding: true }, &secret_bytes)
}

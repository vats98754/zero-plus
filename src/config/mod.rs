use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;
use rust_decimal::Decimal;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub data: DataConfig,
    pub strategy: StrategyConfig,
    pub execution: ExecutionConfig,
    pub risk: RiskConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataConfig {
    pub nats_url: String,
    pub subject: String,
    pub market_data_sources: Vec<MarketDataSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDataSource {
    pub name: String,
    pub websocket_url: String,
    pub symbols: Vec<String>,
    pub source_type: SourceType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    CryptoL2,
    PennyStocks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    pub spread_target: Decimal,
    pub position_size: Decimal,
    pub max_inventory: Decimal,
    pub rebalance_threshold: Decimal,
    pub tick_size: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub exchange_urls: Vec<String>,
    pub api_keys: Vec<ApiKey>,
    pub order_timeout_ms: u64,
    pub max_orders_per_second: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub exchange: String,
    pub key: String,
    pub secret: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_position_size: Decimal,
    pub max_drawdown: Decimal,
    pub stop_loss_pct: Decimal,
    pub daily_loss_limit: Decimal,
    pub position_limits: Vec<PositionLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionLimit {
    pub symbol: String,
    pub max_position: Decimal,
    pub max_notional: Decimal,
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn default() -> Self {
        Config {
            data: DataConfig {
                nats_url: "nats://localhost:4222".to_string(),
                subject: "market_data".to_string(),
                market_data_sources: vec![
                    MarketDataSource {
                        name: "arbitrum_dex".to_string(),
                        websocket_url: "wss://arbitrum.example.com/ws".to_string(),
                        symbols: vec!["ARB/USDC".to_string(), "GMX/USDC".to_string()],
                        source_type: SourceType::CryptoL2,
                    },
                    MarketDataSource {
                        name: "penny_stocks".to_string(),
                        websocket_url: "wss://pennystocks.example.com/ws".to_string(),
                        symbols: vec!["AAPL".to_string(), "TSLA".to_string()],
                        source_type: SourceType::PennyStocks,
                    },
                ],
            },
            strategy: StrategyConfig {
                spread_target: Decimal::from_str("0.0001").unwrap(), // 1 basis point
                position_size: Decimal::from_str("100.0").unwrap(),
                max_inventory: Decimal::from_str("1000.0").unwrap(),
                rebalance_threshold: Decimal::from_str("500.0").unwrap(),
                tick_size: Decimal::from_str("0.0001").unwrap(),
            },
            execution: ExecutionConfig {
                exchange_urls: vec!["https://api.exchange.com".to_string()],
                api_keys: vec![],
                order_timeout_ms: 5000,
                max_orders_per_second: 10,
            },
            risk: RiskConfig {
                max_position_size: Decimal::from_str("10000.0").unwrap(),
                max_drawdown: Decimal::from_str("0.02").unwrap(), // 2%
                stop_loss_pct: Decimal::from_str("0.01").unwrap(), // 1%
                daily_loss_limit: Decimal::from_str("1000.0").unwrap(),
                position_limits: vec![],
            },
        }
    }
}
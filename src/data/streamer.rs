use crate::config::{DataConfig, MarketDataSource, SourceType};
use crate::data::{MarketData, MarketDataType, OrderBook, Trade, Quote, Side};
use anyhow::Result;
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use async_nats::Client;
use rust_decimal::Decimal;
use std::str::FromStr;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn, error, debug};
use url::Url;

pub struct DataStreamer {
    config: DataConfig,
    nats_client: Client,
}

impl DataStreamer {
    pub async fn new(config: DataConfig) -> Result<Self> {
        let nats_client = async_nats::connect(&config.nats_url).await?;
        info!("Connected to NATS at {}", config.nats_url);
        
        Ok(Self {
            config,
            nats_client,
        })
    }

    pub async fn start(&self) -> Result<mpsc::Receiver<MarketData>> {
        let (tx, rx) = mpsc::channel::<MarketData>(1000);
        
        // Start data sources
        for source in &self.config.market_data_sources {
            let source_clone = source.clone();
            let tx_clone = tx.clone();
            let nats_client = self.nats_client.clone();
            let subject = self.config.subject.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::start_websocket_feed(source_clone, tx_clone, nats_client, subject).await {
                    error!("WebSocket feed error: {}", e);
                }
            });
        }
        
        Ok(rx)
    }

    async fn start_websocket_feed(
        source: MarketDataSource,
        tx: mpsc::Sender<MarketData>,
        nats_client: Client,
        subject: String,
    ) -> Result<()> {
        info!("Starting WebSocket feed for {}", source.name);
        
        let url = Url::parse(&source.websocket_url)?;
        let (ws_stream, _) = connect_async(url).await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Send subscription message
        let subscription_msg = Self::create_subscription_message(&source)?;
        ws_sender.send(Message::Text(subscription_msg)).await?;
        
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    debug!("Received message: {}", text);
                    
                    if let Ok(market_data) = Self::parse_market_data(&text, &source) {
                        // Send to NATS JetStream
                        let data_json = serde_json::to_string(&market_data)?;
                        if let Err(e) = nats_client.publish(subject.clone(), data_json.into()).await {
                            warn!("Failed to publish to NATS: {}", e);
                        }
                        
                        // Send to local channel
                        if let Err(e) = tx.send(market_data).await {
                            warn!("Failed to send market data to channel: {}", e);
                            break;
                        }
                    }
                }
                Ok(Message::Binary(_)) => {
                    debug!("Received binary message (ignoring)");
                }
                Ok(Message::Close(_)) => {
                    info!("WebSocket connection closed for {}", source.name);
                    break;
                }
                Err(e) => {
                    error!("WebSocket error for {}: {}", source.name, e);
                    break;
                }
                _ => {}
            }
        }
        
        Ok(())
    }

    fn create_subscription_message(source: &MarketDataSource) -> Result<String> {
        match source.source_type {
            SourceType::CryptoL2 => {
                // Generic L2 crypto exchange subscription format
                let subscription = serde_json::json!({
                    "method": "SUBSCRIBE",
                    "params": source.symbols.iter().map(|symbol| format!("{}@depth", symbol.to_lowercase())).collect::<Vec<_>>(),
                    "id": 1
                });
                Ok(subscription.to_string())
            }
            SourceType::PennyStocks => {
                // Generic stock market data subscription format
                let subscription = serde_json::json!({
                    "action": "subscribe",
                    "symbols": source.symbols,
                    "type": "orderbook"
                });
                Ok(subscription.to_string())
            }
        }
    }

    fn parse_market_data(data: &str, source: &MarketDataSource) -> Result<MarketData> {
        let json: Value = serde_json::from_str(data)?;
        
        match source.source_type {
            SourceType::CryptoL2 => Self::parse_crypto_l2_data(json, source),
            SourceType::PennyStocks => Self::parse_penny_stock_data(json, source),
        }
    }

    fn parse_crypto_l2_data(json: Value, _source: &MarketDataSource) -> Result<MarketData> {
        // Parse generic L2 crypto exchange format
        let symbol = json["s"].as_str().unwrap_or("UNKNOWN").to_string();
        
        if let (Some(bids), Some(asks)) = (json["b"].as_array(), json["a"].as_array()) {
            let mut orderbook = OrderBook::new();
            
            // Parse bids
            for bid in bids {
                if let (Some(price_str), Some(qty_str)) = (bid[0].as_str(), bid[1].as_str()) {
                    if let (Ok(price), Ok(qty)) = (
                        Decimal::from_str(price_str),
                        Decimal::from_str(qty_str)
                    ) {
                        orderbook.update_bid(price, qty);
                    }
                }
            }
            
            // Parse asks
            for ask in asks {
                if let (Some(price_str), Some(qty_str)) = (ask[0].as_str(), ask[1].as_str()) {
                    if let (Ok(price), Ok(qty)) = (
                        Decimal::from_str(price_str),
                        Decimal::from_str(qty_str)
                    ) {
                        orderbook.update_ask(price, qty);
                    }
                }
            }
            
            return Ok(MarketData {
                symbol,
                timestamp: Utc::now(),
                data_type: MarketDataType::OrderBook(orderbook),
            });
        }
        
        // Parse trade data
        if let (Some(price_str), Some(qty_str), Some(side_str)) = (
            json["p"].as_str(),
            json["q"].as_str(),
            json["m"].as_bool(),
        ) {
            if let (Ok(price), Ok(qty)) = (
                Decimal::from_str(price_str),
                Decimal::from_str(qty_str)
            ) {
                let trade = Trade {
                    price,
                    quantity: qty,
                    side: if side_str { Side::Buy } else { Side::Sell },
                    trade_id: json["t"].as_u64().unwrap_or(0).to_string(),
                };
                
                return Ok(MarketData {
                    symbol,
                    timestamp: Utc::now(),
                    data_type: MarketDataType::Trade(trade),
                });
            }
        }
        
        Err(anyhow::anyhow!("Unable to parse crypto L2 data"))
    }

    fn parse_penny_stock_data(json: Value, _source: &MarketDataSource) -> Result<MarketData> {
        // Parse generic penny stock format
        let symbol = json["symbol"].as_str().unwrap_or("UNKNOWN").to_string();
        
        if let (Some(bid_str), Some(ask_str), Some(bid_size_str), Some(ask_size_str)) = (
            json["bid"].as_str(),
            json["ask"].as_str(),
            json["bidSize"].as_str(),
            json["askSize"].as_str(),
        ) {
            if let (Ok(bid), Ok(ask), Ok(bid_size), Ok(ask_size)) = (
                Decimal::from_str(bid_str),
                Decimal::from_str(ask_str),
                Decimal::from_str(bid_size_str),
                Decimal::from_str(ask_size_str),
            ) {
                let quote = Quote {
                    bid,
                    ask,
                    bid_size,
                    ask_size,
                };
                
                return Ok(MarketData {
                    symbol,
                    timestamp: Utc::now(),
                    data_type: MarketDataType::Quote(quote),
                });
            }
        }
        
        Err(anyhow::anyhow!("Unable to parse penny stock data"))
    }
}
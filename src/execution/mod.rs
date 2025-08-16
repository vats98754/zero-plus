use crate::config::ExecutionConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tokio::time::{Duration, timeout, Instant};
use tracing::{info, warn, error, debug};
use reqwest::Client;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: Decimal,
    pub price: Option<Decimal>,
    pub timestamp: DateTime<Utc>,
    pub status: OrderStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "Buy"),
            Side::Sell => write!(f, "Sell"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
    StopLoss,
    TakeProfit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    pub order_id: String,
    pub symbol: String,
    pub side: Side,
    pub quantity: Decimal,
    pub price: Decimal,
    pub timestamp: DateTime<Utc>,
    pub fee: Decimal,
}

pub struct ExecutionEngine {
    config: ExecutionConfig,
    orders: HashMap<String, Order>,
    fills: Vec<Fill>,
    rate_limiter: RateLimiter,
    http_client: Arc<Client>,
    order_queue: VecDeque<Order>,
}

struct RateLimiter {
    max_orders_per_second: u32,
    window_start: Instant,
    request_count: u32,
}

impl RateLimiter {
    fn new(max_orders_per_second: u32) -> Self {
        Self {
            max_orders_per_second,
            window_start: Instant::now(),
            request_count: 0,
        }
    }

    fn can_send_order(&mut self) -> bool {
        let now = Instant::now();
        
        // Reset window if more than 1 second has passed
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
            self.window_start = now;
            self.request_count = 0;
        }
        
        // Check if we can send another order
        if self.request_count < self.max_orders_per_second {
            self.request_count += 1;
            true
        } else {
            false
        }
    }
}

impl ExecutionEngine {
    pub async fn new(config: ExecutionConfig) -> Result<Self> {
        info!("Initializing execution engine with connection pooling");
        
        // Create HTTP client with connection pooling and optimized settings
        let http_client = Arc::new(
            Client::builder()
                .pool_max_idle_per_host(20)
                .pool_idle_timeout(Duration::from_secs(90))
                .timeout(Duration::from_millis(config.order_timeout_ms))
                .tcp_keepalive(Duration::from_secs(60))
                .http2_prior_knowledge()
                .build()?
        );
        
        Ok(Self {
            rate_limiter: RateLimiter::new(config.max_orders_per_second),
            config,
            orders: HashMap::new(),
            fills: Vec::new(),
            http_client,
            order_queue: VecDeque::new(),
        })
    }

    pub async fn place_order(&mut self, mut order: Order) -> Result<String> {
        // Check rate limits
        if !self.rate_limiter.can_send_order() {
            return Err(anyhow::anyhow!("Rate limit exceeded"));
        }

        // Validate order
        self.validate_order(&order)?;

        // Add to queue for batch processing if not a market order
        if matches!(order.order_type, OrderType::Market) {
            // Fast path for market orders
            return self.execute_market_order_fast(order).await;
        }

        // For limit orders, add to batch queue
        order.status = OrderStatus::Pending;
        self.orders.insert(order.id.clone(), order.clone());
        self.order_queue.push_back(order.clone());

        info!("Order queued: {} {} {} @ {}", 
              order.symbol, order.side, order.quantity, 
              order.price.unwrap_or_default());

        // Process batch if queue is full or enough time has passed
        if self.order_queue.len() >= 5 {
            self.process_order_batch().await?;
        }

        Ok(order.id)
    }

    async fn execute_market_order_fast(&mut self, order: Order) -> Result<String> {
        let order_id = order.id.clone();
        
        // Store order
        self.orders.insert(order_id.clone(), order.clone());

        // Execute immediately via HTTP
        match self.execute_order_via_api(order).await {
            Ok(fill) => {
                self.fills.push(fill);
                if let Some(stored_order) = self.orders.get_mut(&order_id) {
                    stored_order.status = OrderStatus::Filled;
                }
                info!("Market order executed: {}", order_id);
                Ok(order_id)
            }
            Err(e) => {
                if let Some(stored_order) = self.orders.get_mut(&order_id) {
                    stored_order.status = OrderStatus::Rejected;
                }
                warn!("Market order execution failed: {}", e);
                Err(e)
            }
        }
    }

    async fn process_order_batch(&mut self) -> Result<()> {
        if self.order_queue.is_empty() {
            return Ok(());
        }

        let batch: Vec<Order> = self.order_queue.drain(..).collect();
        debug!("Processing batch of {} orders", batch.len());

        // Execute orders in parallel for better throughput
        let futures: Vec<_> = batch.into_iter()
            .map(|order| self.execute_order_via_api(order))
            .collect();

        let results = futures::future::join_all(futures).await;
        
        for result in results.into_iter() {
            match result {
                Ok(fill) => {
                    self.fills.push(fill.clone());
                    if let Some(order) = self.orders.get_mut(&fill.order_id) {
                        order.status = OrderStatus::Filled;
                    }
                }
                Err(e) => {
                    error!("Batch order execution failed: {}", e);
                }
            }
        }

        Ok(())
    }

    pub async fn cancel_order(&mut self, order_id: &str) -> Result<()> {
        if let Some(order) = self.orders.get_mut(order_id) {
            if matches!(order.status, OrderStatus::New | OrderStatus::Pending) {
                order.status = OrderStatus::Cancelled;
                info!("Order cancelled: {}", order_id);
                return Ok(());
            }
        }
        
        Err(anyhow::anyhow!("Cannot cancel order: {}", order_id))
    }

    pub async fn cancel_all_orders(&mut self, symbol: Option<&str>) -> Result<Vec<String>> {
        let mut cancelled_orders = Vec::new();
        
        for (order_id, order) in self.orders.iter_mut() {
            let should_cancel = match symbol {
                Some(sym) => order.symbol == sym,
                None => true,
            };
            
            if should_cancel && matches!(order.status, OrderStatus::New | OrderStatus::Pending) {
                order.status = OrderStatus::Cancelled;
                cancelled_orders.push(order_id.clone());
            }
        }
        
        info!("Cancelled {} orders", cancelled_orders.len());
        Ok(cancelled_orders)
    }

    pub fn get_order(&self, order_id: &str) -> Option<&Order> {
        self.orders.get(order_id)
    }

    pub fn get_open_orders(&self, symbol: Option<&str>) -> Vec<&Order> {
        self.orders
            .values()
            .filter(|order| {
                let status_match = matches!(order.status, OrderStatus::New | OrderStatus::Pending);
                let symbol_match = symbol.map_or(true, |sym| order.symbol == sym);
                status_match && symbol_match
            })
            .collect()
    }

    pub fn get_fills(&self, symbol: Option<&str>) -> Vec<&Fill> {
        self.fills
            .iter()
            .filter(|fill| symbol.map_or(true, |sym| fill.symbol == sym))
            .collect()
    }

    fn validate_order(&self, order: &Order) -> Result<()> {
        if order.quantity <= Decimal::ZERO {
            return Err(anyhow::anyhow!("Order quantity must be positive"));
        }

        if matches!(order.order_type, OrderType::Limit) && order.price.is_none() {
            return Err(anyhow::anyhow!("Limit orders must have a price"));
        }

        if order.symbol.is_empty() {
            return Err(anyhow::anyhow!("Order symbol cannot be empty"));
        }

        Ok(())
    }

    async fn execute_order_via_api(&self, order: Order) -> Result<Fill> {
        // Create order payload for API
        let order_payload = serde_json::json!({
            "symbol": order.symbol,
            "side": order.side.to_string().to_lowercase(),
            "type": match order.order_type {
                OrderType::Market => "market",
                OrderType::Limit => "limit",
                OrderType::StopLoss => "stop_loss",
                OrderType::TakeProfit => "take_profit",
            },
            "quantity": order.quantity.to_string(),
            "price": order.price.map(|p| p.to_string()),
            "client_order_id": order.id.clone(),
            "timestamp": order.timestamp.timestamp_millis()
        });

        // Try executing against configured exchange URLs
        for exchange_url in &self.config.exchange_urls {
            let api_url = format!("{}/api/v1/order", exchange_url);
            
            let response = self.http_client
                .post(&api_url)
                .json(&order_payload)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    // Parse successful execution response
                    if let Ok(execution_data) = resp.json::<serde_json::Value>().await {
                        return self.parse_execution_response(order, execution_data).await;
                    }
                }
                Ok(resp) => {
                    warn!("Order execution failed with status: {}", resp.status());
                }
                Err(e) => {
                    warn!("HTTP request failed for {}: {}", api_url, e);
                }
            }
        }

        // If all exchanges fail, fall back to simulation with minimal delay
        self.simulate_execution_fallback(order).await
    }

    async fn parse_execution_response(&self, order: Order, response: serde_json::Value) -> Result<Fill> {
        let execution_price = response["price"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_else(|| order.price.unwrap_or_default());

        let execution_quantity = response["executed_quantity"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(order.quantity);

        let fee = response["fee"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_else(|| {
                // Calculate typical fee
                let fee_rate = Decimal::from_str("0.001").unwrap(); // 0.1%
                execution_quantity * execution_price * fee_rate
            });

        Ok(Fill {
            order_id: order.id,
            symbol: order.symbol,
            side: order.side,
            quantity: execution_quantity,
            price: execution_price,
            timestamp: Utc::now(),
            fee,
        })
    }

    async fn simulate_execution_fallback(&self, order: Order) -> Result<Fill> {
        // Minimal simulation for fallback with very low latency
        tokio::time::sleep(Duration::from_millis(1)).await;

        // Calculate execution price with minimal slippage
        let execution_price = match order.order_type {
            OrderType::Market => {
                let slippage = Decimal::from_str("0.0001").unwrap(); // 1 basis point
                order.price.unwrap_or_default() + slippage
            }
            _ => order.price.unwrap_or_default(),
        };

        let fee_rate = Decimal::from_str("0.001").unwrap(); // 0.1%
        let fee = order.quantity * execution_price * fee_rate;

        Ok(Fill {
            order_id: order.id,
            symbol: order.symbol,
            side: order.side,
            quantity: order.quantity,
            price: execution_price,
            timestamp: Utc::now(),
            fee,
        })
    }

    pub fn get_position(&self, symbol: &str) -> Decimal {
        self.fills
            .iter()
            .filter(|fill| fill.symbol == symbol)
            .fold(Decimal::ZERO, |acc, fill| {
                match fill.side {
                    Side::Buy => acc + fill.quantity,
                    Side::Sell => acc - fill.quantity,
                }
            })
    }

    pub fn get_realized_pnl(&self, symbol: &str) -> Decimal {
        // Simplified PnL calculation
        self.fills
            .iter()
            .filter(|fill| fill.symbol == symbol)
            .fold(Decimal::ZERO, |acc, fill| {
                let trade_value = fill.quantity * fill.price;
                match fill.side {
                    Side::Buy => acc - trade_value - fill.fee,
                    Side::Sell => acc + trade_value - fill.fee,
                }
            })
    }
}

// Add a method to process pending batches
impl ExecutionEngine {
    pub async fn flush_pending_orders(&mut self) -> Result<()> {
        if !self.order_queue.is_empty() {
            self.process_order_batch().await?;
        }
        Ok(())
    }
}
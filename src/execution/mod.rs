use crate::config::ExecutionConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::time::{Duration, timeout};
use tracing::{info, warn};

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
}

struct RateLimiter {
    max_orders_per_second: u32,
    orders_sent: Vec<DateTime<Utc>>,
}

impl RateLimiter {
    fn new(max_orders_per_second: u32) -> Self {
        Self {
            max_orders_per_second,
            orders_sent: Vec::new(),
        }
    }

    fn can_send_order(&mut self) -> bool {
        let now = Utc::now();
        
        // Remove orders older than 1 second
        self.orders_sent.retain(|&timestamp| {
            now.signed_duration_since(timestamp).num_seconds() < 1
        });
        
        // Check if we can send another order
        if self.orders_sent.len() < self.max_orders_per_second as usize {
            self.orders_sent.push(now);
            true
        } else {
            false
        }
    }
}

impl ExecutionEngine {
    pub async fn new(config: ExecutionConfig) -> Result<Self> {
        info!("Initializing execution engine");
        
        Ok(Self {
            rate_limiter: RateLimiter::new(config.max_orders_per_second),
            config,
            orders: HashMap::new(),
            fills: Vec::new(),
        })
    }

    pub async fn place_order(&mut self, mut order: Order) -> Result<String> {
        // Check rate limits
        if !self.rate_limiter.can_send_order() {
            return Err(anyhow::anyhow!("Rate limit exceeded"));
        }

        // Validate order
        self.validate_order(&order)?;

        // For simulation, we'll mock the order placement
        order.status = OrderStatus::Pending;
        self.orders.insert(order.id.clone(), order.clone());

        info!("Order placed: {} {} {} @ {}", 
              order.symbol, order.side, order.quantity, 
              order.price.unwrap_or_default());

        // Simulate order processing
        let order_id = order.id.clone();
        let timeout_duration = Duration::from_millis(self.config.order_timeout_ms);
        
        match timeout(timeout_duration, self.simulate_order_execution(order)).await {
            Ok(Ok(fill)) => {
                self.fills.push(fill);
                if let Some(order) = self.orders.get_mut(&order_id) {
                    order.status = OrderStatus::Filled;
                }
                info!("Order filled: {}", order_id);
            }
            Ok(Err(e)) => {
                if let Some(order) = self.orders.get_mut(&order_id) {
                    order.status = OrderStatus::Rejected;
                }
                warn!("Order rejected: {}: {}", order_id, e);
            }
            Err(_) => {
                if let Some(order) = self.orders.get_mut(&order_id) {
                    order.status = OrderStatus::Cancelled;
                }
                warn!("Order timed out: {}", order_id);
            }
        }

        Ok(order_id)
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

    async fn simulate_order_execution(&self, order: Order) -> Result<Fill> {
        // Simulate network latency
        tokio::time::sleep(Duration::from_millis(10 + (rand::random::<u64>() % 50))).await;

        // Simulate different execution scenarios
        let success_rate = 0.95; // 95% success rate
        
        if (rand::random::<u64>() % 100) as f64 / 100.0 > success_rate {
            return Err(anyhow::anyhow!("Simulated execution failure"));
        }

        // Calculate execution price (with some slippage)
        let execution_price = match order.order_type {
            OrderType::Market => {
                // Market orders get some slippage
                let slippage = Decimal::from_str("0.0001").unwrap(); // 1 basis point
                order.price.unwrap_or_default() + slippage
            }
            OrderType::Limit => {
                // Limit orders execute at limit price
                order.price.unwrap_or_default()
            }
            _ => order.price.unwrap_or_default(),
        };

        // Calculate fee (typical maker fee)
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

// Simple random number generation for simulation
mod rand {
    use std::cell::RefCell;
    
    thread_local! {
        static RNG: RefCell<u64> = RefCell::new(1);
    }
    
    pub fn random<T>() -> T 
    where
        T: From<u64>
    {
        RNG.with(|rng| {
            let mut state = rng.borrow_mut();
            *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            T::from(*state)
        })
    }
}
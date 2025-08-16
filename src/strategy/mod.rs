use crate::config::StrategyConfig;
use crate::data::{MarketData, MarketDataType, OrderBookManager};
use crate::execution::{ExecutionEngine, Order, OrderType, Side};
use crate::risk::RiskManager;
use anyhow::Result;
use chrono::Utc;
use rust_decimal::Decimal;
use std::collections::HashMap;
use tracing::{info, warn, debug};
use uuid::Uuid;

#[cfg(test)]
mod tests;

pub struct ZeroPlusStrategy {
    config: StrategyConfig,
    risk_manager: RiskManager,
    execution_engine: ExecutionEngine,
    orderbook_manager: OrderBookManager,
    positions: HashMap<String, Decimal>, // symbol -> net position
    active_orders: HashMap<String, Vec<Order>>, // symbol -> orders
    pnl: HashMap<String, Decimal>, // symbol -> unrealized PnL
}

impl ZeroPlusStrategy {
    pub fn new(
        config: StrategyConfig,
        risk_manager: RiskManager,
        execution_engine: ExecutionEngine,
    ) -> Self {
        Self {
            config,
            risk_manager,
            execution_engine,
            orderbook_manager: OrderBookManager::new(),
            positions: HashMap::new(),
            active_orders: HashMap::new(),
            pnl: HashMap::new(),
        }
    }

    pub async fn process_market_data(&mut self, market_data: MarketData) -> Result<()> {
        debug!("Processing market data for {}", market_data.symbol);
        
        match market_data.data_type {
            MarketDataType::OrderBook(orderbook) => {
                self.orderbook_manager.update_orderbook(&market_data.symbol, orderbook);
                self.update_strategy(&market_data.symbol).await?;
            }
            MarketDataType::Trade(trade) => {
                // Update last trade price for PnL calculation
                self.update_pnl(&market_data.symbol, trade.price);
                debug!("Trade: {} {} @ {}", market_data.symbol, trade.quantity, trade.price);
            }
            MarketDataType::Quote(quote) => {
                // Use quote data to update our view
                debug!("Quote: {} bid: {} ask: {}", market_data.symbol, quote.bid, quote.ask);
            }
        }
        
        Ok(())
    }

    async fn update_strategy(&mut self, symbol: &str) -> Result<()> {
        // Get current market state
        let Some((bid, ask)) = self.orderbook_manager.get_best_bid_ask(symbol) else {
            warn!("No bid/ask available for {}", symbol);
            return Ok(());
        };

        let mid_price = (bid + ask) / Decimal::from(2);
        let spread = ask - bid;
        let spread_bps = self.orderbook_manager.get_spread_bps(symbol).unwrap_or_default();
        
        debug!("Market state for {}: bid={}, ask={}, mid={}, spread_bps={}", 
               symbol, bid, ask, mid_price, spread_bps);

        // Check if we should trade (spread must be profitable)
        if spread < self.config.spread_target {
            debug!("Spread too tight for {}: {} < {}", symbol, spread, self.config.spread_target);
            return Ok(());
        }

        // Get current position
        let current_position = self.positions.get(symbol).copied().unwrap_or_default();
        
        // Check risk limits
        if !self.risk_manager.can_trade(symbol, self.config.position_size, current_position) {
            warn!("Risk limits exceeded for {}", symbol);
            return Ok(());
        }

        // Cancel existing orders if market has moved significantly
        self.cancel_stale_orders(symbol, mid_price).await?;

        // Implement 0+ strategy logic
        self.place_market_making_orders(symbol, bid, ask, mid_price, current_position).await?;

        Ok(())
    }

    async fn place_market_making_orders(
        &mut self,
        symbol: &str,
        bid: Decimal,
        ask: Decimal,
        mid_price: Decimal,
        current_position: Decimal,
    ) -> Result<()> {
        let mut orders_to_place = Vec::new();

        // Calculate optimal bid/ask prices based on current inventory
        let inventory_skew = self.calculate_inventory_skew(current_position);
        let skewed_mid = mid_price + (inventory_skew * self.config.tick_size);

        // Place bid order (buy side)
        let bid_price = skewed_mid - (self.config.spread_target / Decimal::from(2));
        let bid_size = self.calculate_order_size(current_position, Side::Buy);
        
        if bid_size > Decimal::ZERO && bid_price < bid {
            let bid_order = Order {
                id: Uuid::new_v4().to_string(),
                symbol: symbol.to_string(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                quantity: bid_size,
                price: Some(bid_price),
                timestamp: Utc::now(),
                status: crate::execution::OrderStatus::New,
            };
            orders_to_place.push(bid_order);
        }

        // Place ask order (sell side)
        let ask_price = skewed_mid + (self.config.spread_target / Decimal::from(2));
        let ask_size = self.calculate_order_size(current_position, Side::Sell);
        
        if ask_size > Decimal::ZERO && ask_price > ask {
            let ask_order = Order {
                id: Uuid::new_v4().to_string(),
                symbol: symbol.to_string(),
                side: Side::Sell,
                order_type: OrderType::Limit,
                quantity: ask_size,
                price: Some(ask_price),
                timestamp: Utc::now(),
                status: crate::execution::OrderStatus::New,
            };
            orders_to_place.push(ask_order);
        }

        // Execute orders
        for order in orders_to_place {
            match self.execution_engine.place_order(order.clone()).await {
                Ok(_) => {
                    info!("Placed order: {} {} {} @ {}", 
                          order.symbol, order.side, order.quantity, 
                          order.price.unwrap_or_default());
                    
                    // Track active order
                    self.active_orders
                        .entry(symbol.to_string())
                        .or_insert_with(Vec::new)
                        .push(order);
                }
                Err(e) => {
                    warn!("Failed to place order: {}", e);
                }
            }
        }

        Ok(())
    }

    fn calculate_inventory_skew(&self, position: Decimal) -> Decimal {
        // Skew orders based on current inventory to encourage mean reversion
        let max_skew = Decimal::from(10); // Max skew in ticks
        let skew_factor = position / self.config.max_inventory;
        (skew_factor * max_skew).min(max_skew).max(-max_skew)
    }

    fn calculate_order_size(&self, current_position: Decimal, side: Side) -> Decimal {
        let base_size = self.config.position_size;
        
        match side {
            Side::Buy => {
                // Reduce buy size if we're already long
                if current_position > Decimal::ZERO {
                    base_size * (Decimal::ONE - (current_position / self.config.max_inventory))
                } else {
                    base_size
                }
            }
            Side::Sell => {
                // Reduce sell size if we're already short
                if current_position < Decimal::ZERO {
                    base_size * (Decimal::ONE - (current_position.abs() / self.config.max_inventory))
                } else {
                    base_size
                }
            }
        }
    }

    async fn cancel_stale_orders(&mut self, symbol: &str, current_mid: Decimal) -> Result<()> {
        let active_orders = self.active_orders.get(symbol).cloned().unwrap_or_default();
        let mut orders_to_cancel = Vec::new();

        for order in &active_orders {
            if let Some(order_price) = order.price {
                let price_diff = (order_price - current_mid).abs();
                let max_diff = self.config.tick_size * Decimal::from(5); // 5 ticks away
                
                if price_diff > max_diff {
                    orders_to_cancel.push(order.id.clone());
                }
            }
        }

        for order_id in orders_to_cancel {
            if let Err(e) = self.execution_engine.cancel_order(&order_id).await {
                warn!("Failed to cancel order {}: {}", order_id, e);
            } else {
                // Remove from active orders
                if let Some(orders) = self.active_orders.get_mut(symbol) {
                    orders.retain(|o| o.id != order_id);
                }
            }
        }

        Ok(())
    }

    fn update_pnl(&mut self, symbol: &str, current_price: Decimal) {
        if let Some(position) = self.positions.get(symbol) {
            // This is a simplified PnL calculation
            // In reality, you'd track the average entry price
            let unrealized_pnl = *position * current_price;
            self.pnl.insert(symbol.to_string(), unrealized_pnl);
        }
    }

    pub fn get_position(&self, symbol: &str) -> Decimal {
        self.positions.get(symbol).copied().unwrap_or_default()
    }

    pub fn get_pnl(&self, symbol: &str) -> Decimal {
        self.pnl.get(symbol).copied().unwrap_or_default()
    }

    pub fn update_position(&mut self, symbol: &str, trade_quantity: Decimal, side: Side) {
        let current_position = self.positions.get(symbol).copied().unwrap_or_default();
        let position_change = match side {
            Side::Buy => trade_quantity,
            Side::Sell => -trade_quantity,
        };
        
        let new_position = current_position + position_change;
        self.positions.insert(symbol.to_string(), new_position);
        
        info!("Position update for {}: {} -> {}", symbol, current_position, new_position);
    }
}
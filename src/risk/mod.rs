use crate::config::RiskConfig;
use rust_decimal::Decimal;
use std::str::FromStr;
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};
use tracing::{warn, info};

pub struct RiskManager {
    config: RiskConfig,
    positions: HashMap<String, Decimal>, // symbol -> net position
    daily_pnl: HashMap<String, Decimal>, // symbol -> daily PnL
    drawdown: Decimal,
    max_drawdown_today: Decimal,
    daily_trades: HashMap<String, u32>, // symbol -> trade count
    last_reset: DateTime<Utc>,
}

impl RiskManager {
    pub fn new(config: RiskConfig) -> Self {
        Self {
            config,
            positions: HashMap::new(),
            daily_pnl: HashMap::new(),
            drawdown: Decimal::ZERO,
            max_drawdown_today: Decimal::ZERO,
            daily_trades: HashMap::new(),
            last_reset: Utc::now(),
        }
    }

    pub fn can_trade(&mut self, symbol: &str, trade_size: Decimal, current_position: Decimal) -> bool {
        self.reset_daily_stats_if_needed();

        // Check overall daily loss limit
        if self.get_total_daily_pnl() <= -self.config.daily_loss_limit {
            warn!("Daily loss limit exceeded: {}", self.get_total_daily_pnl());
            return false;
        }

        // Check maximum drawdown
        if self.max_drawdown_today >= self.config.max_drawdown {
            warn!("Maximum drawdown exceeded: {}", self.max_drawdown_today);
            return false;
        }

        // Check position size limits
        let new_position = current_position + trade_size;
        if new_position.abs() > self.config.max_position_size {
            warn!("Maximum position size exceeded for {}: {} > {}", 
                  symbol, new_position.abs(), self.config.max_position_size);
            return false;
        }

        // Check symbol-specific limits
        if let Some(limit) = self.get_position_limit(symbol) {
            if new_position.abs() > limit.max_position {
                warn!("Symbol position limit exceeded for {}: {} > {}", 
                      symbol, new_position.abs(), limit.max_position);
                return false;
            }

            let notional_value = new_position.abs() * self.get_estimated_price(symbol);
            if notional_value > limit.max_notional {
                warn!("Symbol notional limit exceeded for {}: {} > {}", 
                      symbol, notional_value, limit.max_notional);
                return false;
            }
        }

        // Check trade frequency limits (prevent overtrading)
        let daily_trade_count = self.daily_trades.get(symbol).copied().unwrap_or(0);
        if daily_trade_count > 100 { // Max 100 trades per symbol per day
            warn!("Daily trade limit exceeded for {}: {}", symbol, daily_trade_count);
            return false;
        }

        true
    }

    pub fn update_position(&mut self, symbol: &str, new_position: Decimal) {
        let old_position = self.positions.insert(symbol.to_string(), new_position);
        
        if let Some(old_pos) = old_position {
            info!("Position updated for {}: {} -> {}", symbol, old_pos, new_position);
        } else {
            info!("New position for {}: {}", symbol, new_position);
        }
    }

    pub fn update_pnl(&mut self, symbol: &str, pnl: Decimal) {
        self.daily_pnl.insert(symbol.to_string(), pnl);
        
        // Update drawdown tracking
        let total_pnl = self.get_total_daily_pnl();
        if total_pnl < Decimal::ZERO && total_pnl.abs() > self.max_drawdown_today {
            self.max_drawdown_today = total_pnl.abs();
        }
    }

    pub fn record_trade(&mut self, symbol: &str) {
        let count = self.daily_trades.entry(symbol.to_string()).or_insert(0);
        *count += 1;
    }

    pub fn should_stop_loss(&self, symbol: &str, current_price: Decimal) -> bool {
        if let Some(position) = self.positions.get(symbol) {
            if position.is_zero() {
                return false;
            }

            // Calculate unrealized PnL (simplified)
            let estimated_entry_price = self.get_estimated_entry_price(symbol);
            let price_diff = current_price - estimated_entry_price;
            let unrealized_pnl_pct = if position.is_sign_positive() {
                price_diff / estimated_entry_price
            } else {
                -price_diff / estimated_entry_price
            };

            if unrealized_pnl_pct <= -self.config.stop_loss_pct {
                warn!("Stop loss triggered for {}: {}%", symbol, unrealized_pnl_pct * Decimal::from(100));
                return true;
            }
        }

        false
    }

    pub fn check_circuit_breaker(&self) -> bool {
        // Circuit breaker conditions
        let total_pnl = self.get_total_daily_pnl();
        
        // Stop trading if daily loss exceeds limit
        if total_pnl <= -self.config.daily_loss_limit {
            warn!("Circuit breaker triggered: daily loss limit exceeded");
            return true;
        }

        // Stop trading if drawdown exceeds limit
        if self.max_drawdown_today >= self.config.max_drawdown {
            warn!("Circuit breaker triggered: max drawdown exceeded");
            return true;
        }

        false
    }

    pub fn get_position(&self, symbol: &str) -> Decimal {
        self.positions.get(symbol).copied().unwrap_or_default()
    }

    pub fn get_daily_pnl(&self, symbol: &str) -> Decimal {
        self.daily_pnl.get(symbol).copied().unwrap_or_default()
    }

    pub fn get_total_daily_pnl(&self) -> Decimal {
        self.daily_pnl.values().sum()
    }

    pub fn get_max_drawdown(&self) -> Decimal {
        self.max_drawdown_today
    }

    pub fn get_risk_metrics(&self) -> RiskMetrics {
        RiskMetrics {
            total_daily_pnl: self.get_total_daily_pnl(),
            max_drawdown: self.max_drawdown_today,
            total_positions: self.positions.len(),
            risk_utilization: self.calculate_risk_utilization(),
            circuit_breaker_active: self.check_circuit_breaker(),
        }
    }

    fn reset_daily_stats_if_needed(&mut self) {
        let now = Utc::now();
        let time_since_reset = now.signed_duration_since(self.last_reset);
        
        // Reset stats at the start of each day
        if time_since_reset > Duration::hours(24) {
            info!("Resetting daily risk statistics");
            self.daily_pnl.clear();
            self.daily_trades.clear();
            self.max_drawdown_today = Decimal::ZERO;
            self.last_reset = now;
        }
    }

    fn get_position_limit(&self, symbol: &str) -> Option<&crate::config::PositionLimit> {
        self.config.position_limits.iter().find(|limit| limit.symbol == symbol)
    }

    fn get_estimated_price(&self, _symbol: &str) -> Decimal {
        // In a real implementation, this would fetch the current market price
        // For now, we'll use a default value
        Decimal::from(100)
    }

    fn get_estimated_entry_price(&self, _symbol: &str) -> Decimal {
        // In a real implementation, this would track the average entry price
        // For now, we'll use a default value
        Decimal::from(100)
    }

    fn calculate_risk_utilization(&self) -> Decimal {
        let total_notional: Decimal = self.positions
            .iter()
            .map(|(symbol, position)| {
                position.abs() * self.get_estimated_price(symbol)
            })
            .sum();
        
        let max_total_notional = self.config.max_position_size * Decimal::from(100); // Simplified
        
        if max_total_notional.is_zero() {
            Decimal::ZERO
        } else {
            total_notional / max_total_notional
        }
    }
}

#[derive(Debug, Clone)]
pub struct RiskMetrics {
    pub total_daily_pnl: Decimal,
    pub max_drawdown: Decimal,
    pub total_positions: usize,
    pub risk_utilization: Decimal,
    pub circuit_breaker_active: bool,
}

impl RiskMetrics {
    pub fn is_healthy(&self) -> bool {
        !self.circuit_breaker_active && 
        self.risk_utilization < Decimal::from_str("0.8").unwrap() // 80% max utilization
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RiskConfig;

    fn create_test_risk_config() -> RiskConfig {
        RiskConfig {
            max_position_size: Decimal::from(1000),
            max_drawdown: Decimal::from_str("0.02").unwrap(),
            stop_loss_pct: Decimal::from_str("0.01").unwrap(),
            daily_loss_limit: Decimal::from(500),
            position_limits: vec![],
        }
    }

    #[test]
    fn test_position_size_limit() {
        let config = create_test_risk_config();
        let mut risk_manager = RiskManager::new(config);
        
        // Should allow normal trade
        assert!(risk_manager.can_trade("BTCUSD", Decimal::from(100), Decimal::ZERO));
        
        // Should reject trade that exceeds position limit
        assert!(!risk_manager.can_trade("BTCUSD", Decimal::from(2000), Decimal::ZERO));
    }

    #[test]
    fn test_daily_loss_limit() {
        let config = create_test_risk_config();
        let mut risk_manager = RiskManager::new(config);
        
        // Simulate large loss
        risk_manager.update_pnl("BTCUSD", Decimal::from(-600));
        
        // Should reject new trades after loss limit exceeded
        assert!(!risk_manager.can_trade("BTCUSD", Decimal::from(100), Decimal::ZERO));
    }
}
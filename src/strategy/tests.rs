#[cfg(test)]
mod tests {
    use super::super::ZeroPlusStrategy;
    use crate::config::*;
    use crate::data::{OrderBookManager, OrderBook};
    use crate::execution::{ExecutionEngine, Order, OrderType, Side as ExecSide, OrderStatus};
    use crate::risk::RiskManager;
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use chrono::Utc;

    #[tokio::test]
    async fn test_strategy_initialization() {
        let config = StrategyConfig {
            spread_target: Decimal::from_str("0.001").unwrap(),
            position_size: Decimal::from_str("100.0").unwrap(),
            max_inventory: Decimal::from_str("1000.0").unwrap(),
            rebalance_threshold: Decimal::from_str("500.0").unwrap(),
            tick_size: Decimal::from_str("0.0001").unwrap(),
        };

        let risk_config = RiskConfig {
            max_position_size: Decimal::from_str("10000.0").unwrap(),
            max_drawdown: Decimal::from_str("0.02").unwrap(),
            stop_loss_pct: Decimal::from_str("0.01").unwrap(),
            daily_loss_limit: Decimal::from_str("1000.0").unwrap(),
            position_limits: vec![],
        };

        let execution_config = ExecutionConfig {
            exchange_urls: vec!["https://test.com".to_string()],
            api_keys: vec![],
            order_timeout_ms: 5000,
            max_orders_per_second: 10,
        };

        let risk_manager = RiskManager::new(risk_config);
        let execution_engine = ExecutionEngine::new(execution_config).await.unwrap();
        let strategy = ZeroPlusStrategy::new(config, risk_manager, execution_engine);

        // Test that strategy initializes correctly
        assert_eq!(strategy.get_position("BTC/USD"), Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_orderbook_management() {
        let manager = OrderBookManager::new();
        
        let mut orderbook = OrderBook::new();
        orderbook.update_bid(Decimal::from_str("100.0").unwrap(), Decimal::from_str("10.0").unwrap());
        orderbook.update_ask(Decimal::from_str("101.0").unwrap(), Decimal::from_str("10.0").unwrap());
        
        manager.update_orderbook("BTC/USD", orderbook);
        
        let (bid, ask) = manager.get_best_bid_ask("BTC/USD").unwrap();
        assert_eq!(bid, Decimal::from_str("100.0").unwrap());
        assert_eq!(ask, Decimal::from_str("101.0").unwrap());
        
        let spread_bps = manager.get_spread_bps("BTC/USD").unwrap();
        assert!(spread_bps > Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_execution_engine() {
        let config = ExecutionConfig {
            exchange_urls: vec!["https://test.com".to_string()],
            api_keys: vec![],
            order_timeout_ms: 1000,
            max_orders_per_second: 10,
        };

        let mut engine = ExecutionEngine::new(config).await.unwrap();

        let order = Order {
            id: "test-order-1".to_string(),
            symbol: "BTC/USD".to_string(),
            side: ExecSide::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::from_str("1.0").unwrap(),
            price: Some(Decimal::from_str("50000.0").unwrap()),
            timestamp: Utc::now(),
            status: OrderStatus::New,
        };

        let order_id = engine.place_order(order).await.unwrap();
        assert_eq!(order_id, "test-order-1");
    }

    #[test]
    fn test_risk_management() {
        let config = RiskConfig {
            max_position_size: Decimal::from_str("1000.0").unwrap(),
            max_drawdown: Decimal::from_str("0.02").unwrap(),
            stop_loss_pct: Decimal::from_str("0.01").unwrap(),
            daily_loss_limit: Decimal::from_str("500.0").unwrap(),
            position_limits: vec![],
        };

        let mut risk_manager = RiskManager::new(config);

        // Test normal trade approval
        assert!(risk_manager.can_trade("BTC/USD", Decimal::from_str("100.0").unwrap(), Decimal::ZERO));

        // Test position size limit
        assert!(!risk_manager.can_trade("BTC/USD", Decimal::from_str("2000.0").unwrap(), Decimal::ZERO));

        // Test daily loss limit
        risk_manager.update_pnl("BTC/USD", Decimal::from_str("-600.0").unwrap());
        assert!(!risk_manager.can_trade("BTC/USD", Decimal::from_str("100.0").unwrap(), Decimal::ZERO));
    }

    #[test]
    fn test_config_loading() {
        let config = Config::default();
        
        assert_eq!(config.data.nats_url, "nats://localhost:4222");
        assert!(config.data.market_data_sources.len() > 0);
        assert!(config.strategy.spread_target > Decimal::ZERO);
        assert!(config.risk.max_position_size > Decimal::ZERO);
    }
}
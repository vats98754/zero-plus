use crate::execution::{ExecutionEngine, Order, OrderType, Side, OrderStatus};
use crate::config::ExecutionConfig;
use crate::data::{OrderBook, OrderBookManager};
use rust_decimal::Decimal;
use std::str::FromStr;
use chrono::Utc;
use uuid::Uuid;
use std::time::Instant;
use anyhow::Result;

#[cfg(test)]
mod performance_tests {
    use super::*;

    #[tokio::test]
    async fn test_execution_engine_throughput() {
        let config = ExecutionConfig {
            exchange_urls: vec!["https://test.example.com".to_string()],
            api_keys: vec![],
            order_timeout_ms: 1000,
            max_orders_per_second: 100,
        };

        let mut engine = ExecutionEngine::new(config).await.unwrap();
        
        let start = Instant::now();
        let num_orders = 50;
        
        // Test batch processing of limit orders
        for i in 0..num_orders {
            let order = Order {
                id: Uuid::new_v4().to_string(),
                symbol: "BTC/USDC".to_string(),
                side: if i % 2 == 0 { Side::Buy } else { Side::Sell },
                order_type: OrderType::Limit,
                quantity: Decimal::from_str("1.0").unwrap(),
                price: Some(Decimal::from_str(&format!("{}.0", 50000 + i)).unwrap()),
                timestamp: Utc::now(),
                status: OrderStatus::New,
            };
            
            engine.place_order(order).await.unwrap();
        }
        
        // Flush any remaining batched orders
        engine.flush_pending_orders().await.unwrap();
        
        let duration = start.elapsed();
        println!("Processed {} orders in {:?}", num_orders, duration);
        println!("Throughput: {:.2} orders/sec", num_orders as f64 / duration.as_secs_f64());
        
        // Should process orders quickly with batching
        assert!(duration.as_millis() < 1000, "Order processing took too long: {:?}", duration);
    }

    #[tokio::test]
    async fn test_market_order_fast_path() {
        let config = ExecutionConfig {
            exchange_urls: vec!["https://test.example.com".to_string()],
            api_keys: vec![],
            order_timeout_ms: 100,
            max_orders_per_second: 100,
        };

        let mut engine = ExecutionEngine::new(config).await.unwrap();
        
        let start = Instant::now();
        
        // Test fast path for market orders
        let order = Order {
            id: Uuid::new_v4().to_string(),
            symbol: "BTC/USDC".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::from_str("1.0").unwrap(),
            price: Some(Decimal::from_str("50000.0").unwrap()),
            timestamp: Utc::now(),
            status: OrderStatus::New,
        };
        
        engine.place_order(order).await.unwrap();
        
        let duration = start.elapsed();
        println!("Market order processed in {:?}", duration);
        
        // Market orders should be very fast (< 50ms including fallback simulation)
        assert!(duration.as_millis() < 50, "Market order took too long: {:?}", duration);
    }

    #[test]
    fn test_orderbook_performance() {
        let manager = OrderBookManager::new();
        let start = Instant::now();
        
        // Test orderbook operations with new Decimal-based structure
        let mut orderbook = OrderBook::new();
        
        // Add many price levels
        for i in 0..1000 {
            let bid_price = Decimal::from_str(&format!("{}.{:02}", 50000 - i, i % 100)).unwrap();
            let ask_price = Decimal::from_str(&format!("{}.{:02}", 50001 + i, i % 100)).unwrap();
            let quantity = Decimal::from_str(&format!("{}.0", i + 1)).unwrap();
            
            orderbook.update_bid(bid_price, quantity);
            orderbook.update_ask(ask_price, quantity);
        }
        
        manager.update_orderbook("BTC/USDC", orderbook);
        
        // Test many lookups
        for _ in 0..1000 {
            manager.get_best_bid_ask("BTC/USDC");
            manager.get_spread_bps("BTC/USDC");
        }
        
        let duration = start.elapsed();
        println!("Orderbook operations completed in {:?}", duration);
        
        // Orderbook operations should be very fast with Decimal keys
        assert!(duration.as_millis() < 100, "Orderbook operations took too long: {:?}", duration);
    }

    #[test]
    fn test_rate_limiter_performance() {
        let config = ExecutionConfig {
            exchange_urls: vec![],
            api_keys: vec![],
            order_timeout_ms: 1000,
            max_orders_per_second: 100,
        };
        
        let start = Instant::now();
        
        // Create multiple engines to test rate limiter efficiency
        let _engines: Vec<_> = (0..10).map(|_| {
            tokio_test::block_on(ExecutionEngine::new(config.clone()))
        }).collect::<Result<Vec<_>, _>>().unwrap();
        
        let duration = start.elapsed();
        println!("Created 10 execution engines in {:?}", duration);
        
        // Engine creation should be reasonable (HTTP client creation takes time)
        assert!(duration.as_millis() < 1000, "Engine creation took too long: {:?}", duration);
    }
}
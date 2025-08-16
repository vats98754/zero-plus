use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::collections::BTreeMap;
use dashmap::DashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketData {
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub data_type: MarketDataType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketDataType {
    OrderBook(OrderBook),
    Trade(Trade),
    Quote(Quote),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub bids: BTreeMap<Decimal, Decimal>, // price -> quantity (sorted desc for bids)
    pub asks: BTreeMap<Decimal, Decimal>, // price -> quantity (sorted asc for asks)
    pub last_update: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub price: Decimal,
    pub quantity: Decimal,
    pub side: Side,
    pub trade_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub bid: Decimal,
    pub ask: Decimal,
    pub bid_size: Decimal,
    pub ask_size: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

pub struct OrderBookManager {
    books: DashMap<String, OrderBook>,
}

impl OrderBookManager {
    pub fn new() -> Self {
        Self {
            books: DashMap::new(),
        }
    }

    pub fn update_orderbook(&self, symbol: &str, orderbook: OrderBook) {
        self.books.insert(symbol.to_string(), orderbook);
    }

    pub fn get_orderbook(&self, symbol: &str) -> Option<OrderBook> {
        self.books.get(symbol).map(|entry| entry.clone())
    }

    pub fn get_best_bid_ask(&self, symbol: &str) -> Option<(Decimal, Decimal)> {
        self.books.get(symbol).and_then(|book| {
            // Bids are sorted in ascending order, so we want the last (highest)
            let best_bid = book.bids.keys().last().copied()?;
            // Asks are sorted in ascending order, so we want the first (lowest)
            let best_ask = book.asks.keys().next().copied()?;
            Some((best_bid, best_ask))
        })
    }

    pub fn get_mid_price(&self, symbol: &str) -> Option<Decimal> {
        self.get_best_bid_ask(symbol).map(|(bid, ask)| {
            (bid + ask) / Decimal::from(2)
        })
    }

    pub fn get_spread(&self, symbol: &str) -> Option<Decimal> {
        self.get_best_bid_ask(symbol).map(|(bid, ask)| {
            ask - bid
        })
    }

    pub fn get_spread_bps(&self, symbol: &str) -> Option<Decimal> {
        let (bid, ask) = self.get_best_bid_ask(symbol)?;
        let mid = (bid + ask) / Decimal::from(2);
        let spread = ask - bid;
        Some((spread / mid) * Decimal::from(10000)) // basis points
    }
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_update: Utc::now(),
        }
    }

    pub fn update_bid(&mut self, price: Decimal, quantity: Decimal) {
        if quantity > Decimal::ZERO {
            self.bids.insert(price, quantity);
        } else {
            self.bids.remove(&price);
        }
        self.last_update = Utc::now();
    }

    pub fn update_ask(&mut self, price: Decimal, quantity: Decimal) {
        if quantity > Decimal::ZERO {
            self.asks.insert(price, quantity);
        } else {
            self.asks.remove(&price);
        }
        self.last_update = Utc::now();
    }

    pub fn best_bid(&self) -> Option<(Decimal, Decimal)> {
        self.bids.iter().next_back().map(|(&price, &qty)| (price, qty))
    }

    pub fn best_ask(&self) -> Option<(Decimal, Decimal)> {
        self.asks.iter().next().map(|(&price, &qty)| (price, qty))
    }
}
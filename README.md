# Zero Plus HFT Strategy

A high-frequency trading (HFT) implementation of the "0+" market making strategy in Rust, designed for very small volume penny stocks and Layer 2 crypto coins.

## Overview

The 0+ strategy is a market making approach that aims to maintain near-zero inventory while profiting from bid-ask spreads. This implementation uses:

- **NATS JetStream** for ultra-fast message streaming and data distribution
- **Rust** for maximum performance and memory safety
- **Async/await** for concurrent processing of multiple market feeds
- **WebSocket** connections for real-time market data

## Target Markets

- **Layer 2 Crypto**: Arbitrum, Optimism, Polygon DEX tokens
- **Penny Stocks**: Low volume traditional market securities

## Architecture

### Core Components

1. **Data Streaming** (`src/data/`)
   - NATS JetStream integration for message streaming
   - WebSocket connections to market data sources
   - Order book management and price aggregation

2. **Strategy Engine** (`src/strategy/`)
   - 0+ market making logic
   - Inventory management and position rebalancing
   - Dynamic spread calculation based on market conditions

3. **Execution Engine** (`src/execution/`)
   - Order placement and management
   - Rate limiting and latency optimization
   - Simulated execution for testing

4. **Risk Management** (`src/risk/`)
   - Position size limits
   - Stop-loss mechanisms
   - Daily loss limits and circuit breakers

5. **Configuration** (`src/config/`)
   - JSON-based configuration system
   - Market-specific parameters
   - Risk and execution settings

## Quick Start

### Prerequisites

- Rust 1.70+
- NATS server (for message streaming)
- Access to market data feeds

### Installation

```bash
git clone <repository-url>
cd zero-plus
cargo build --release
```

### Configuration

Edit `config.json` to configure:

```json
{
  "data": {
    "nats_url": "nats://localhost:4222",
    "market_data_sources": [
      {
        "name": "arbitrum_uniswap",
        "websocket_url": "wss://arb-mainnet.g.alchemy.com/v2/YOUR_API_KEY",
        "symbols": ["ARB/USDC", "GMX/USDC"],
        "source_type": "CryptoL2"
      }
    ]
  },
  "strategy": {
    "spread_target": "0.0005",
    "position_size": "100.0",
    "max_inventory": "1000.0"
  },
  "risk": {
    "max_position_size": "10000.0",
    "daily_loss_limit": "1000.0"
  }
}
```

### Running

```bash
# Start with default config
cargo run

# Specify custom config and market type
cargo run -- --config my_config.json --market crypto
```

## Strategy Details

### 0+ Market Making

The strategy places simultaneous buy and sell orders around the current market price:

1. **Spread Targeting**: Maintains target spread width to ensure profitability
2. **Inventory Skewing**: Adjusts order prices based on current position to encourage mean reversion
3. **Dynamic Sizing**: Reduces order sizes as inventory approaches limits
4. **Risk Controls**: Cancels stale orders and enforces position limits

### Performance Optimizations

- **Lock-free data structures** (DashMap) for order book management
- **Minimal allocations** using pre-allocated buffers
- **Batch processing** of market data updates
- **Async I/O** for concurrent data processing

## Data Sources

### Crypto L2 (Example formats supported)
- Uniswap V3 on Arbitrum
- Velodrome on Optimism  
- QuickSwap on Polygon

### Penny Stocks (Example formats supported)
- Alpaca Markets data feed
- IEX Cloud market data
- Polygon.io stock feeds

## Risk Management

- **Position Limits**: Per-symbol and total exposure limits
- **Stop Losses**: Automatic position closure on adverse moves
- **Circuit Breakers**: Trading halt on excessive losses
- **Daily Limits**: Maximum daily loss thresholds

## Testing

Run unit tests:
```bash
cargo test
```

Run integration tests:
```bash
cargo test --test integration
```

## Monitoring

The strategy provides real-time metrics including:
- Current positions and PnL
- Order fill rates and latencies
- Risk utilization percentages
- Market data feed health

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass
5. Submit a pull request

## License

MIT License - see LICENSE file for details

## Disclaimer

This software is for educational and research purposes. Trading financial instruments carries risk of loss. Use at your own risk.

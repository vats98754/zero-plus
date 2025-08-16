#!/usr/bin/env bash
set -e

echo "Starting NATS server..."
# Start NATS server if not running
if ! pgrep -f "nats-server" > /dev/null; then
    echo "Please install and start NATS server first:"
    echo "  curl -L https://github.com/nats-io/nats-server/releases/download/v2.10.5/nats-server-v2.10.5-linux-amd64.zip -o nats-server.zip"
    echo "  unzip nats-server.zip"
    echo "  ./nats-server-v2.10.5-linux-amd64/nats-server &"
    echo ""
fi

echo "Building zero-plus HFT strategy..."
cargo build --release

echo "Running zero-plus with sample configuration..."
echo "Note: This will use simulated data and execution"

# Run the strategy
cargo run --release -- --config config.json --market crypto

echo "Strategy stopped."
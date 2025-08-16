use anyhow::Result;
use clap::{Arg, Command};
use tracing::{info, error};
use tracing_subscriber;

mod config;
mod data;
mod strategy;
mod execution;
mod risk;

use config::Config;
use data::DataStreamer;
use strategy::ZeroPlusStrategy;
use execution::ExecutionEngine;
use risk::RiskManager;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Parse command line arguments
    let matches = Command::new("zero-plus")
        .version("0.1.0")
        .about("0+ HFT Strategy for penny stocks and L2 crypto markets")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("FILE")
                .help("Configuration file path")
                .default_value("config.json")
        )
        .arg(
            Arg::new("market")
                .short('m')
                .long("market")
                .value_name("TYPE")
                .help("Market type: crypto or stocks")
                .default_value("crypto")
        )
        .get_matches();

    let config_path = matches.get_one::<String>("config").unwrap();
    let market_type = matches.get_one::<String>("market").unwrap();

    info!("Starting 0+ HFT Strategy");
    info!("Config: {}, Market: {}", config_path, market_type);

    // Load configuration
    let config = Config::load(config_path)?;
    
    // Initialize components
    let risk_manager = RiskManager::new(config.risk.clone());
    let execution_engine = ExecutionEngine::new(config.execution.clone()).await?;
    let data_streamer = DataStreamer::new(config.data.clone()).await?;
    let mut strategy = ZeroPlusStrategy::new(
        config.strategy.clone(),
        risk_manager,
        execution_engine,
    );

    // Start data streaming
    let mut data_receiver = data_streamer.start().await?;
    
    info!("Strategy started, processing market data...");

    // Main strategy loop
    while let Some(market_data) = data_receiver.recv().await {
        match strategy.process_market_data(market_data).await {
            Ok(_) => {},
            Err(e) => {
                error!("Error processing market data: {}", e);
            }
        }
    }

    info!("Strategy stopped");
    Ok(())
}
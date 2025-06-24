use anyhow::Result;
use clap::{App, Arg, SubCommand};
use log::{info, error};
use serde_json;
use std::fs;
use std::path::Path;
use solana_sdk::signature::{Keypair, Signer};
use dotenv;
use std::env;

use solana_sniper_bot::{DexConfig, DexMonitor, MonitoringConfig, SafetyConfig, SniperConfig, SolanaSniperBot, TelegramConfig, TradingConfig};

fn create_default_config() -> SniperConfig {
    SniperConfig {
        rpc_url: env::var("RPC_URL").expect("RPC_URL must be set"),
        wallets: env::var("WALLETS").unwrap_or_else(|_| "wallets/wallet1.json".to_string())
            .split(',').map(|s| s.trim().to_string()).collect(),
        min_sol_balance: env::var("MIN_SOL_BALANCE").unwrap_or_else(|_| "0.1".to_string()).parse().unwrap(),
        max_sol_per_trade: env::var("MAX_SOL_PER_TRADE").unwrap_or_else(|_| "0.05".to_string()).parse().unwrap(),
        slippage_tolerance: env::var("SLIPPAGE_TOLERANCE").unwrap_or_else(|_| "0.1".to_string()).parse().unwrap(),
        auto_sell: env::var("AUTO_SELL").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
        anti_rug_check: env::var("ANTI_RUG_CHECK").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
        dex_config: DexConfig {
            raydium_api_url: env::var("RAYDIUM_API_URL").expect("RAYDIUM_API_URL must be set"),
            orca_api_url: env::var("ORCA_API_URL").expect("ORCA_API_URL must be set"),
            jupiter_api_url: env::var("JUPITER_API_URL").expect("JUPITER_API_URL must be set"),
            birdeye_api_url: env::var("BIRDEYE_API_URL").expect("BIRDEYE_API_URL must be set"),
            solscan_api_url: env::var("SOLSCAN_API_URL").expect("SOLSCAN_API_URL must be set"),
        },
        monitoring: MonitoringConfig {
            enable_raydium: env::var("ENABLE_RAYDIUM").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
            enable_orca: env::var("ENABLE_ORCA").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
            enable_jupiter: env::var("ENABLE_JUPITER").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
            check_interval_ms: env::var("CHECK_INTERVAL_MS").unwrap_or_else(|_| "1000".to_string()).parse().unwrap(),
            websocket_reconnect_delay_ms: env::var("WEBSOCKET_RECONNECT_DELAY_MS").unwrap_or_else(|_| "5000".to_string()).parse().unwrap(),
        },
        safety: SafetyConfig {
            min_liquidity_sol: env::var("MIN_LIQUIDITY_SOL").unwrap_or_else(|_| "1.0".to_string()).parse().unwrap(),
            max_creator_holdings_percent: env::var("MAX_CREATOR_HOLDINGS_PERCENT").unwrap_or_else(|_| "50.0".to_string()).parse().unwrap(),
            blacklist_check: env::var("BLACKLIST_CHECK").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
            honeypot_check: env::var("HONEYPOT_CHECK").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
            min_market_cap: env::var("MIN_MARKET_CAP").unwrap_or_else(|_| "10000.0".to_string()).parse().unwrap(),
            min_holders: env::var("MIN_HOLDERS").unwrap_or_else(|_| "100".to_string()).parse().unwrap(),
            min_volume_24h: env::var("MIN_VOLUME_24H").unwrap_or_else(|_| "1000.0".to_string()).parse().unwrap(),
        },
        trading: TradingConfig {
            max_price_impact: env::var("MAX_PRICE_IMPACT").unwrap_or_else(|_| "0.05".to_string()).parse().unwrap(),
            min_slippage: env::var("MIN_SLIPPAGE").unwrap_or_else(|_| "0.001".to_string()).parse().unwrap(),
            max_slippage: env::var("MAX_SLIPPAGE").unwrap_or_else(|_| "0.1".to_string()).parse().unwrap(),
            gas_priority: env::var("GAS_PRIORITY").unwrap_or_else(|_| "medium".to_string()),
            retry_failed_trades: env::var("RETRY_FAILED_TRADES").unwrap_or_else(|_| "true".to_string()).parse().unwrap(),
            max_retries: env::var("MAX_RETRIES").unwrap_or_else(|_| "3".to_string()).parse().unwrap(),
        },
        telegram: TelegramConfig {
            bot_token: env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN must be set"),
            chat_id: env::var("TELEGRAM_CHAT_ID").expect("TELEGRAM_CHAT_ID must be set"),
        },
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();
    
    let matches = App::new("Solana Sniper Bot CLI")
        .version("1.0")
        .author("Your Name")
        .about("CLI tool for managing Solana Sniper Bot")
        .subcommand(SubCommand::with_name("start")
            .about("Start the sniper bot")
            .arg(Arg::with_name("config")
                .short('c')
                .long("config")
                .value_name("FILE")
                .help("Configuration file path")
                .default_value("config.json")))
        .subcommand(SubCommand::with_name("status")
            .about("Show bot status"))
        .subcommand(SubCommand::with_name("create-wallets")
            .about("Create multiple wallets")
            .arg(Arg::with_name("count")
                .short('n')
                .long("count")
                .value_name("NUMBER")
                .help("Number of wallets to create")
                .default_value("10")))
        .subcommand(SubCommand::with_name("check-balance")
            .about("Check wallet balance")
            .arg(Arg::with_name("wallet")
                .short('w')
                .long("wallet")
                .value_name("INDEX")
                .help("Wallet index (0-based)")
                .required(true)))
        .subcommand(SubCommand::with_name("feed-wallet")
            .about("Feed wallet with small amount")
            .arg(Arg::with_name("from")
                .short('f')
                .long("from")
                .value_name("INDEX")
                .help("Source wallet index")
                .required(true))
            .arg(Arg::with_name("to")
                .short('t')
                .long("to")
                .value_name("INDEX")
                .help("Target wallet index")
                .required(true))
            .arg(Arg::with_name("amount")
                .short('a')
                .long("amount")
                .value_name("SOL")
                .help("Amount in SOL")
                .default_value("0.01")))
        .subcommand(SubCommand::with_name("snipe")
            .about("Manually snipe a token")
            .arg(Arg::with_name("token")
                .short('t')
                .long("token")
                .value_name("ADDRESS")
                .help("Token address")
                .required(true))
            .arg(Arg::with_name("wallet")
                .short('w')
                .long("wallet")
                .value_name("INDEX")
                .help("Wallet index")
                .default_value("0"))
            .arg(Arg::with_name("amount")
                .short('a')
                .long("amount")
                .value_name("SOL")
                .help("Amount in SOL")
                .default_value("0.01")))
        .get_matches();

    match matches.subcommand() {
        Some(("start", args)) => {
            let config_file = args.value_of("config").unwrap();
            start_bot(config_file).await?;
        }
        Some(("status", _)) => {
            show_status().await?;
        }
        Some(("create-wallets", args)) => {
            let count: usize = args.value_of("count").unwrap().parse()?;
            create_wallets(count)?;
        }
        Some(("check-balance", args)) => {
            let wallet_index: usize = args.value_of("wallet").unwrap().parse()?;
            check_balance(wallet_index).await?;
        }
        Some(("feed-wallet", args)) => {
            let from_index: usize = args.value_of("from").unwrap().parse()?;
            let to_index: usize = args.value_of("to").unwrap().parse()?;
            let amount: f64 = args.value_of("amount").unwrap().parse()?;
            feed_wallet(from_index, to_index, amount).await?;
        }
        Some(("snipe", args)) => {
            let token_address = args.value_of("token").unwrap();
            let wallet_index: usize = args.value_of("wallet").unwrap().parse()?;
            let amount: f64 = args.value_of("amount").unwrap().parse()?;
            snipe_token(token_address, wallet_index, amount).await?;
        }
        _ => {
            println!("Use --help for usage information");
        }
    }

    Ok(())
}

async fn start_bot(config_file: &str) -> Result<()> {
    info!("Starting Solana Sniper Bot with config: {}", config_file);
    
    // Load config
    let config_data = fs::read_to_string(config_file)?;
    let config: SniperConfig = serde_json::from_str(&config_data)?;
    
    // Create bot
    let bot = SolanaSniperBot::new(config.clone())?;
    
    // Start monitoring
    let telegram_config = Some((config.telegram.bot_token.clone(), config.telegram.chat_id.clone()));
    let dex_config = solana_sniper_bot::dex_monitor::DexConfig {
        raydium_api_url: config.dex_config.raydium_api_url.clone(),
        orca_api_url: config.dex_config.orca_api_url.clone(),
        jupiter_api_url: config.dex_config.jupiter_api_url.clone(),
        birdeye_api_url: config.dex_config.birdeye_api_url.clone(),
        solscan_api_url: config.dex_config.solscan_api_url.clone(),
    };
    let _dex_monitor = DexMonitor::new(
        dex_config,
        config.rpc_url.clone(),
        telegram_config,
    );
    bot.start().await?;
    
    Ok(())
}

async fn show_status() -> Result<()> {
    info!("Bot status: Running");
    // TODO: Implement status checking
    Ok(())
}

fn create_wallets(count: usize) -> Result<()> {
    if !Path::new("wallets").exists() {
        fs::create_dir("wallets")?;
    }
    
    info!("Creating {} wallets...", count);
    
    for i in 1..=count {
        let keypair = Keypair::new();
        let wallet_name = format!("wallet{}", i);
        let wallet_data = serde_json::to_string_pretty(&keypair.to_bytes().to_vec())?;
        
        fs::write(format!("wallets/{}.json", wallet_name), wallet_data)?;
        info!("Created wallet {}: {}", wallet_name, keypair.pubkey());
    }
    
    info!("Successfully created {} wallets", count);
    Ok(())
}

async fn check_balance(wallet_index: usize) -> Result<()> {
    let config = create_default_config();
    
    let bot = SolanaSniperBot::new(config)?;
    let balance = bot.check_balance(wallet_index)?;
    
    info!("Wallet {} balance: {} SOL", wallet_index, balance);
    Ok(())
}

async fn feed_wallet(from_index: usize, to_index: usize, amount: f64) -> Result<()> {
    let config = create_default_config();
    
    let bot = SolanaSniperBot::new(config)?;
    let signature = bot.feed_wallet(from_index, to_index, amount).await?;
    
    info!("Fed wallet {} with {} SOL: {}", to_index, amount, signature);
    Ok(())
}

async fn snipe_token(token_address: &str, wallet_index: usize, amount: f64) -> Result<()> {
    let config = create_default_config();
    
    let bot = SolanaSniperBot::new(config)?;
    
    match bot.snipe_token(wallet_index, token_address, amount).await {
        Ok(signature) => {
            info!("Successfully sniped token: {}", signature);
        }
        Err(e) => {
            error!("Failed to snipe token: {}", e);
        }
    }
    
    Ok(())
}
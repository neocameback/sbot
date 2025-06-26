use anyhow::{anyhow, Result};
use log::{info, warn, error};
use serde::{Deserialize, Serialize};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
    native_token::LAMPORTS_PER_SOL,
};
use solana_sdk::signature::read_keypair_file;
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod dex_monitor;
pub mod websocket_monitor;
pub mod telegram;

// use dex_monitor::{DexMonitor, TokenMetadata};
use websocket_monitor::{DexWebSocketManager, WebSocketMessage};

pub use crate::dex_monitor::{DexMonitor, TokenMetadata};
pub use crate::telegram::{TelegramConfig, TelegramSender};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SniperConfig {
    pub rpc_url: String,
    pub wallets: Vec<String>,
    pub min_sol_balance: f64,
    pub max_sol_per_trade: f64,
    pub slippage_tolerance: f64,
    pub auto_sell: bool,
    pub anti_rug_check: bool,
    pub dex_config: DexConfig,
    pub monitoring: MonitoringConfig,
    pub safety: SafetyConfig,
    pub trading: TradingConfig,
    pub telegram: TelegramConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexConfig {
    pub raydium_api_url: String,
    pub orca_api_url: String,
    pub jupiter_api_url: String,
    pub birdeye_api_url: String,
    pub solscan_api_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    pub enable_raydium: bool,
    pub enable_orca: bool,
    pub enable_jupiter: bool,
    pub check_interval_ms: u64,
    pub websocket_reconnect_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyConfig {
    pub min_liquidity_sol: f64,
    pub max_creator_holdings_percent: f64,
    pub blacklist_check: bool,
    pub honeypot_check: bool,
    pub min_market_cap: f64,
    pub min_holders: u64,
    pub min_volume_24h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub max_price_impact: f64,
    pub min_slippage: f64,
    pub max_slippage: f64,
    pub gas_priority: String,
    pub retry_failed_trades: bool,
    pub max_retries: u32,
}

// Enhanced token information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
    pub total_supply: u64,
    pub pool_address: Option<String>,
    pub created_at: u64,
    pub metadata: Option<TokenMetadata>,
}

// Enhanced sniper state
#[derive(Debug, Clone)]
pub struct SniperState {
    pub is_running: bool,
    pub total_trades: u64,
    pub successful_trades: u64,
    pub failed_trades: u64,
    pub total_profit: f64,
    pub last_snipe_time: u64,
    pub active_monitors: u32,
}

pub struct SolanaSniperBot {
    config: SniperConfig,
    client: RpcClient,
    wallets: Vec<Keypair>,
    state: Arc<Mutex<SniperState>>,
    dex_monitor: DexMonitor,
    websocket_manager: DexWebSocketManager,
}

impl SolanaSniperBot {
    pub fn new(config: SniperConfig) -> Result<Self> {
        let client = RpcClient::new_with_commitment(
            config.rpc_url.clone(),
            CommitmentConfig::confirmed(),
        );

        let mut wallets = Vec::new();
        for wallet_path in &config.wallets {
            let keypair = read_keypair_file(wallet_path)
                .map_err(|e| anyhow!("Failed to read wallet {}: {}", wallet_path, e))?;
            wallets.push(keypair);
        }

        let state = Arc::new(Mutex::new(SniperState {
            is_running: false,
            total_trades: 0,
            successful_trades: 0,
            failed_trades: 0,
            total_profit: 0.0,
            last_snipe_time: 0,
            active_monitors: 0,
        }));

        // Convert our DexConfig to dex_monitor::DexConfig
        let dex_config = dex_monitor::DexConfig {
            raydium_api_url: config.dex_config.raydium_api_url.clone(),
            orca_api_url: config.dex_config.orca_api_url.clone(),
            jupiter_api_url: config.dex_config.jupiter_api_url.clone(),
            birdeye_api_url: config.dex_config.birdeye_api_url.clone(),
            solscan_api_url: config.dex_config.solscan_api_url.clone(),
        };

        let telegram_sender = TelegramSender::new(config.telegram.clone());
        let dex_monitor = DexMonitor::new(dex_config, config.rpc_url.clone(), telegram_sender);
        let mut websocket_manager = DexWebSocketManager::new();

        // Add WebSocket monitors based on configuration
        if config.monitoring.enable_raydium {
            websocket_manager.add_monitor(
                config.dex_config.raydium_api_url.clone(),
                config.monitoring.websocket_reconnect_delay_ms,
            );
        }

        Ok(SolanaSniperBot {
            config,
            client,
            wallets,
            state,
            dex_monitor,
            websocket_manager,
        })
    }

    // Create new wallet
    pub fn create_wallet(&self, wallet_name: &str) -> Result<()> {
        let keypair = Keypair::new();
        let wallet_data = serde_json::to_string_pretty(&keypair.to_bytes().to_vec())?;
        
        std::fs::write(format!("wallets/{}.json", wallet_name), wallet_data)?;
        info!("Created new wallet: {} at {}", wallet_name, keypair.pubkey());
        
        Ok(())
    }

    // Check wallet balance
    pub fn check_balance(&self, wallet_index: usize) -> Result<f64> {
        if wallet_index >= self.wallets.len() {
            return Err(anyhow!("Invalid wallet index"));
        }

        let pubkey = self.wallets[wallet_index].pubkey();
        let balance = self.client.get_balance(&pubkey)?;
        
        Ok(balance as f64 / LAMPORTS_PER_SOL as f64)
    }

    // Feed wallet with SOL
    pub async fn feed_wallet(&self, from_index: usize, to_index: usize, amount: f64) -> Result<String> {
        if from_index >= self.wallets.len() || to_index >= self.wallets.len() {
            return Err(anyhow!("Invalid wallet index"));
        }

        let from_keypair = &self.wallets[from_index];
        let to_pubkey = self.wallets[to_index].pubkey();
        let lamports = (amount * LAMPORTS_PER_SOL as f64) as u64;

        let tx = system_instruction::transfer(
            &from_keypair.pubkey(),
            &to_pubkey,
            lamports,
        );

        let recent_blockhash = self.client.get_latest_blockhash()?;
        let transaction = Transaction::new_signed_with_payer(
            &[tx],
            Some(&from_keypair.pubkey()),
            &[from_keypair],
            recent_blockhash,
        );

        let signature = self.client.send_and_confirm_transaction(&transaction)?;
        info!("Fed wallet {} with {} SOL: {}", to_index, amount, signature);
        
        Ok(signature.to_string())
    }

    // Enhanced token analysis with real data
    pub async fn analyze_token(&self, token_address: &str) -> Result<bool> {
        info!("Analyzing token safety: {}", token_address);
        
        // Get token metadata from Birdeye
        let metadata = self.dex_monitor.get_token_metadata(token_address).await?;
        
        // Check verification status
        if !metadata.is_verified {
            warn!("Token {} is not verified", token_address);
            return Ok(false);
        }
        
        // Check market cap
        if metadata.market_cap < self.config.safety.min_market_cap {
            warn!("Token {} has low market cap: {}", token_address, metadata.market_cap);
            return Ok(false);
        }
        
        // Check volume
        if metadata.volume_24h < self.config.safety.min_volume_24h {
            warn!("Token {} has low 24h volume: {}", token_address, metadata.volume_24h);
            return Ok(false);
        }
        
        // Check holders
        if metadata.holders < self.config.safety.min_holders {
            warn!("Token {} has few holders: {}", token_address, metadata.holders);
            return Ok(false);
        }
        
        // Check liquidity
        if let Ok(has_liquidity) = self.dex_monitor.check_token_liquidity(token_address).await {
            if !has_liquidity {
                warn!("Token {} has insufficient liquidity", token_address);
                return Ok(false);
            }
        }
        
        // Check honeypot (using public method)
        if self.config.safety.honeypot_check {
            if let Ok(is_honeypot) = self.dex_monitor.analyze_token_safety(token_address).await {
                if !is_honeypot {
                    warn!("Token {} appears to be a honeypot", token_address);
                    return Ok(false);
                }
            }
        }
        
        info!("Token {} passed all safety checks", token_address);
        Ok(true)
    }

    // Real token sniping with Jupiter
    pub async fn snipe_token(&self, wallet_index: usize, token_address: &str, amount_sol: f64) -> Result<String> {
        if wallet_index >= self.wallets.len() {
            return Err(anyhow!("Invalid wallet index"));
        }

        // Check balance
        let balance = self.check_balance(wallet_index)?;
        if balance < amount_sol {
            return Err(anyhow!("Insufficient balance: {} SOL, need {} SOL", balance, amount_sol));
        }

        // Enhanced token analysis
        if self.config.anti_rug_check {
            let is_safe = self.analyze_token(token_address).await?;
            if !is_safe {
                return Err(anyhow!("Token failed safety check"));
            }
        }

        let keypair = &self.wallets[wallet_index];
        let sol_mint = "So11111111111111111111111111111111111111112";
        let amount_lamports = (amount_sol * LAMPORTS_PER_SOL as f64) as u64;

        // Get Jupiter quote
        let swap_info = self.dex_monitor.get_jupiter_quote(sol_mint, token_address, amount_lamports).await?;
        
        // Check price impact
        if swap_info.price_impact > self.config.trading.max_price_impact {
            return Err(anyhow!("Price impact too high: {}%", swap_info.price_impact * 100.0));
        }

        // Execute swap
        let signature = self.dex_monitor.execute_jupiter_swap(keypair, &swap_info).await?;
        
        // Update state
        let mut state = self.state.lock().await;
        state.total_trades += 1;
        state.successful_trades += 1;
        state.last_snipe_time = chrono::Utc::now().timestamp() as u64;
        
        info!("Successfully sniped token {} with {} SOL: {}", token_address, amount_sol, signature);
        
        Ok(signature)
    }

    // Sell token
    pub async fn sell_token(&self, wallet_index: usize, token_address: &str, amount: f64) -> Result<String> {
        if wallet_index >= self.wallets.len() {
            return Err(anyhow!("Invalid wallet index"));
        }

        let keypair = &self.wallets[wallet_index];
        let sol_mint = "So11111111111111111111111111111111111111112";
        
        // Get sell quote
        let swap_info = self.dex_monitor.get_jupiter_quote(token_address, sol_mint, amount as u64).await?;
        
        // Execute sell
        let signature = self.dex_monitor.execute_jupiter_swap(keypair, &swap_info).await?;
        
        info!("Successfully sold token {}: {}", token_address, signature);
        Ok(signature)
    }

    // Start real-time monitoring
    pub async fn start_monitoring(&self) -> Result<()> {
        info!("Starting real-time DEX monitoring...");
        
        let mut state = self.state.lock().await;
        state.is_running = true;
        state.active_monitors = 1; // Placeholder
        drop(state);

        // Start on-chain monitoring for Raydium
        if self.config.monitoring.enable_raydium {
            let dex_config = dex_monitor::DexConfig {
                raydium_api_url: self.config.dex_config.raydium_api_url.clone(),
                orca_api_url: self.config.dex_config.orca_api_url.clone(),
                jupiter_api_url: self.config.dex_config.jupiter_api_url.clone(),
                birdeye_api_url: self.config.dex_config.birdeye_api_url.clone(),
                solscan_api_url: self.config.dex_config.solscan_api_url.clone(),
            };
            let rpc_url = self.config.rpc_url.clone();
            let message_tx = self.websocket_manager.get_message_sender();
            
            let telegram_config = TelegramConfig {
                bot_token: self.config.telegram.bot_token.clone(),
                chat_id: self.config.telegram.chat_id.clone() 
            };
            let telegram_sender = TelegramSender::new(telegram_config);
            tokio::spawn(async move {
                let dex_monitor = crate::dex_monitor::DexMonitor::new(dex_config, rpc_url, telegram_sender);
                if let Err(e) = dex_monitor.monitor_raydium_onchain(message_tx).await {
                    error!("Raydium on-chain monitoring failed: {}", e);
                }
            });
        }

        // Process messages from all monitors
        loop {
            let message = self.websocket_manager.receive_message().await;
            
            if let Some(message) = message {
                match message {
                    WebSocketMessage::TokenListing(listing) => {
                        info!("New token listing: {} ({})", listing.symbol, listing.token_address);
                        
                        // Auto snipe if enabled
                        if self.config.auto_sell {
                            for (i, _) in self.wallets.iter().enumerate() {
                                if let Err(e) = self.snipe_token(i, &listing.token_address, self.config.max_sol_per_trade).await {
                                    error!("Failed to snipe token {}: {}", listing.token_address, e);
                                }
                            }
                        }
                    }
                    WebSocketMessage::PoolUpdate(pool) => {
                        info!("Pool update: {} - {} <-> {}", pool.pool_address, pool.token_a, pool.token_b);
                    }
                    WebSocketMessage::PriceUpdate(price) => {
                        info!("Price update: {} = ${}", price.token_address, price.price);
                    }
                    WebSocketMessage::Error(error_msg) => {
                        error!("WebSocket error: {}", error_msg);
                    }
                }
            } else {
                // Channel closed, break the loop
                break;
            }
        }

        Ok(())
    }

    // Get current status
    pub async fn get_status(&self) -> SniperState {
        self.state.lock().await.clone()
    }

    // Start the bot
    pub async fn start(&self) -> Result<()> {
        info!("Starting Solana Sniper Bot...");
        
        // Start monitoring
        self.start_monitoring().await?;
        
        Ok(())
    }

    // Stop the bot
    pub async fn stop(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.is_running = false;
        state.active_monitors = 0;
        
        info!("Stopping Solana Sniper Bot...");
        Ok(())
    }
}

use anyhow::Result;
use log::{info, error};
use serde::{Deserialize, Serialize};
use solana_client::{
    rpc_client::RpcClient,
    pubsub_client::PubsubClient,
    rpc_config::{RpcTransactionLogsFilter, RpcTransactionLogsConfig},
};
use solana_sdk::signature::{Keypair, Signature};
use solana_transaction_status::UiTransactionEncoding;
use std::str::FromStr;
use tokio::sync::mpsc;
use crate::websocket_monitor::{WebSocketMessage, PoolUpdate};
use crate::send_telegram_message;

const RAYDIUM_LIQUIDITY_POOL_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeNSfTFE9yPZVvTZ6Qko";

#[derive(Debug, Clone)]
pub struct DexConfig {
    pub raydium_api_url: String,
    pub orca_api_url: String,
    pub jupiter_api_url: String,
    pub birdeye_api_url: String,
    pub solscan_api_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolInfo {
    pub pool_address: String,
    pub token_a: String,
    pub token_b: String,
    pub token_a_amount: u64,
    pub token_b_amount: u64,
    pub fee_rate: f64,
    pub created_at: u64,
    pub liquidity: f64,
    pub volume_24h: f64,
    pub price_change_24h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapInfo {
    pub input_token: String,
    pub output_token: String,
    pub input_amount: u64,
    pub output_amount: u64,
    pub slippage: f64,
    pub route: Vec<String>,
    pub price_impact: f64,
    pub fee_amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
    pub total_supply: u64,
    pub market_cap: f64,
    pub price: f64,
    pub volume_24h: f64,
    pub holders: u64,
    pub creator: String,
    pub is_verified: bool,
}

pub struct DexMonitor {
    client: RpcClient,
    telegram_config: Option<(String, String)>, // (bot_token, chat_id)
}

impl DexMonitor {
    pub fn new(_config: DexConfig, rpc_url: String, telegram_config: Option<(String, String)>) -> Self {
        let client = RpcClient::new(rpc_url);
        DexMonitor { client, telegram_config }
    }

    pub async fn monitor_raydium_onchain(&self, tx: mpsc::Sender<WebSocketMessage>) -> Result<()> {
        let rpc_ws_url = self.client.url().replace("http", "ws");
        info!("Starting on-chain monitoring for Raydium pools at {}", rpc_ws_url);
        let sender = tx.clone();
        let rpc_url = self.client.url();
        let telegram_config = self.telegram_config.clone();
        std::thread::spawn(move || {
            loop {
                match PubsubClient::logs_subscribe(
                    &rpc_ws_url,
                    RpcTransactionLogsFilter::Mentions(vec![RAYDIUM_LIQUIDITY_POOL_V4_PROGRAM_ID.to_string()]),
                    RpcTransactionLogsConfig { commitment: None },
                ) {
                    Ok((_client, receiver)) => {
                        let rpc_client = RpcClient::new(rpc_url.clone());
                        while let Ok(log_info) = receiver.recv() {
                            if log_info.value.logs.iter().any(|log| log.contains("initialize2")) {
                                info!("Detected potential new Raydium pool: {}", log_info.value.signature);
                                if let Ok(tx_signature) = Signature::from_str(&log_info.value.signature) {
                                    if let Ok(fetched_tx) = rpc_client.get_transaction(&tx_signature, UiTransactionEncoding::JsonParsed) {
                                        if let Some(transaction) = fetched_tx.transaction.transaction.decode() {
                                            let account_keys = transaction.message.static_account_keys();
                                            for instruction in transaction.message.instructions() {
                                                let program_id = account_keys[instruction.program_id_index as usize];
                                                if program_id.to_string() == RAYDIUM_LIQUIDITY_POOL_V4_PROGRAM_ID {
                                                    if instruction.accounts.len() > 9 {
                                                        let pool_addr = account_keys[instruction.accounts[4] as usize].to_string();
                                                        let token_a = account_keys[instruction.accounts[8] as usize].to_string();
                                                        let token_b = account_keys[instruction.accounts[9] as usize].to_string();
                                                        let pool_update = WebSocketMessage::PoolUpdate(PoolUpdate {
                                                            pool_address: pool_addr.clone(),
                                                            token_a: token_a.clone(),
                                                            token_b: token_b.clone(),
                                                            liquidity: 0.0, volume_24h: 0.0,
                                                            timestamp: chrono::Utc::now().timestamp() as u64,
                                                        });
                                                        let sender_clone = sender.clone();
                                                        let telegram_config = telegram_config.clone();
                                                        tokio::spawn(async move {
                                                            if let Err(e) = sender_clone.send(pool_update).await {
                                                                error!("Failed to send pool update: {}", e);
                                                            }
                                                            if let Some((bot_token, chat_id)) = telegram_config {
                                                                let msg = format!(
                                                                    "*New Raydium Pool Detected!*
Pool: `{}`
Token A: `{}`
Token B: `{}`",
                                                                    pool_addr, token_a, token_b
                                                                );
                                                                let _ = send_telegram_message(&bot_token, &chat_id, &msg).await;
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        error!("Raydium on-chain monitor: subscription ended, reconnecting in 5s...");
                    }
                    Err(e) => {
                        error!("Raydium on-chain monitor: failed to subscribe: {}. Retrying in 5s...", e);
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        });
        Ok(())
    }

    pub async fn monitor_jupiter(&self, _tx: mpsc::Sender<WebSocketMessage>) -> Result<()> {
        Ok(())
    }

    pub async fn get_jupiter_quote(&self, _input_token: &str, _output_token: &str, _amount: u64) -> Result<SwapInfo> {
        unimplemented!()
    }

    pub async fn execute_jupiter_swap(&self, _keypair: &Keypair, _swap_info: &SwapInfo) -> Result<String> {
        unimplemented!()
    }

    pub async fn get_token_metadata(&self, _token_address: &str) -> Result<TokenMetadata> {
        unimplemented!()
    }

    pub async fn analyze_token_safety(&self, _token_address: &str) -> Result<bool> {
        unimplemented!()
    }

    pub async fn check_token_liquidity(&self, _token_address: &str) -> Result<bool> {
        unimplemented!()
    }
} 
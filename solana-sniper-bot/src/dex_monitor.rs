use anyhow::Result;
use log::{debug, info, error, warn};
use serde::{Deserialize, Serialize};
use solana_client::{
    rpc_client::RpcClient,
    pubsub_client::PubsubClient,
    rpc_config::{RpcTransactionLogsFilter, RpcTransactionLogsConfig, RpcTransactionConfig},
};
use solana_sdk::{signature::{Keypair, Signature}};
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding};
use std::{str::FromStr};
use tokio::sync::mpsc;
use tokio::runtime::Runtime;
use crate::websocket_monitor::{WebSocketMessage, PoolUpdate};
use crate::telegram::TelegramSender;

// const RAYDIUM_LIQUIDITY_POOL_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeNSfTFE9yPZVvTZ6Qko";
const RAYDIUM_LIQUIDITY_POOL_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";

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
    telegram_sender: TelegramSender,
}

impl DexMonitor {
    pub fn new(_config: DexConfig, rpc_url: String, telegram_sender: TelegramSender) -> Self {
        let client = RpcClient::new(rpc_url);
        DexMonitor { client, telegram_sender }
    }

    pub async fn monitor_raydium_onchain(&self, tx: mpsc::Sender<WebSocketMessage>) -> Result<()> {
        let rpc_ws_url = self.client.url().replace("http", "ws");
        info!("Starting on-chain monitoring for Raydium pools at {}", rpc_ws_url);
        let sender = tx.clone();
        let telegram_sender = self.telegram_sender.clone();
        let rpc_url = self.client.url();
        let rpc_client = RpcClient::new(rpc_url.clone());

        tokio::spawn(async move {
            loop {
                match PubsubClient::logs_subscribe(
                    &rpc_ws_url,
                    RpcTransactionLogsFilter::Mentions(vec![RAYDIUM_LIQUIDITY_POOL_V4_PROGRAM_ID.to_string()]),
                    RpcTransactionLogsConfig { commitment: None },
                ) {
                    Ok((_client, receiver)) => {
                        
                        while let Ok(log_info) = receiver.recv() {
                            if log_info.value.logs.iter().any(|log| log.contains("initialize2")) {
                                info!("Detected potential new Raydium pool: {}", log_info.value.signature);
                                if let Ok(tx_signature) = Signature::from_str(&log_info.value.signature) {
                                    info!("tx_signature: {:?}", tx_signature);
                                    
                                    // Retry mechanism for fetching transaction
                                    let mut retry_count = 0;
                                    let max_retries = 3;
                                    let mut fetched_tx = None;
                                    
                                    while retry_count < max_retries {
                                        // Get transaction with retries
                                        let tx_signature_clone = tx_signature;
                                        match get_transaction(&rpc_client, &tx_signature_clone) {
                                            Ok(tx) => {
                                                fetched_tx = Some(tx);
                                                break;
                                            }
                                            Err(e) => {
                                                retry_count += 1;
                                                if retry_count >= max_retries {
                                                    error!("Failed to get transaction after {} retries: {} (Error: {})", 
                                                           max_retries, log_info.value.signature, e);
                                                } else {
                                                    warn!("Failed to get transaction (attempt {}/{}): {} (Error: {}). Retrying in {}ms...", 
                                                          retry_count, max_retries, log_info.value.signature, e, 
                                                          (2_u64.pow(retry_count as u32) * 100));
                                                    std::thread::sleep(std::time::Duration::from_millis(2_u64.pow(retry_count as u32) * 100));
                                                }
                                            }
                                        }
                                    }
                                    
                                    if let Some(fetched_tx) = fetched_tx {
                                        // Decode transaction
                                        match decode_transaction(&fetched_tx) {
                                            Ok(pull_updates) => {
                                                info!("===> Pull updates: {:?}", pull_updates);
                                                for pull_update in pull_updates.clone() {
                                                    if let WebSocketMessage::PoolUpdate(pool_update) = pull_update.clone() {
                                                        info!("===> Sending info about new pool: {:?}", pool_update);
                                                        let sender_clone = sender.clone();
                                                        let telegram_sender_clone = telegram_sender.clone();
                                                        if let Err(e) = sender_clone.send(pull_update).await {
                                                            error!("Failed to send pool update: {}", e);
                                                        }
                                                        if let Err(e) = telegram_sender_clone.send_new_pool_telegram_message(&log_info.value.signature, &pool_update.pool_address, &pool_update.token_a, &pool_update.token_b).await {
                                                            error!("{}", e);
                                                        }
                                                        // tokio::spawn(async move {
                                                            // if let Err(e) = sender_clone.send(pull_update).await {
                                                            //     error!("Failed to send pool update: {}", e);
                                                            // }
                                                            // if let Err(e) = telegram_sender_clone.send_new_pool_telegram_message(&pool_update.pool_address, &pool_update.token_a, &pool_update.token_b).await {
                                                            //     error!("{}", e);
                                                            // }
                                                        // });
                                                    } else {
                                                        error!("Failed to get pool update: {:?}", pull_update);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                error!("Failed to decode transaction: {}", e);
                                            }
                                        }
                                    } else {
                                        // Transaction fetching failed after all retries, skip this transaction
                                        debug!("Skipping transaction {} due to fetch failure", log_info.value.signature);
                                    }
                                } else {
                                    error!("Failed to get transaction signature: {}", log_info.value.signature);
                                }
                            } else {
                                debug!("====> log_info: {:?}", log_info);
                                debug!("No new Raydium pool detected");
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

fn get_transaction(client: &RpcClient, tx_signature: &Signature) -> Result<EncodedConfirmedTransactionWithStatusMeta> {
    info!("===> Getting transaction: {:?}", tx_signature);
    // let rpc_url = client.url();
    let config = RpcTransactionConfig {
        encoding: Some(UiTransactionEncoding::JsonParsed),
        commitment: None,
        max_supported_transaction_version: Some(0),
    };
    // let rpc_client = RpcClient::new(rpc_url.clone());
    match client.get_transaction_with_config(tx_signature, config) {
        Ok(tx) => {
            Ok(tx)
        }
        Err(e) => {
            error!("Failed to get transaction: {}", e);
            Err(anyhow::anyhow!("Failed to get transaction: {}", e))
        }
    }
}

fn decode_transaction(fetched_tx: &EncodedConfirmedTransactionWithStatusMeta) -> Result<Vec<WebSocketMessage>> {
    // List of pull updates
    let mut pull_updates = Vec::new();
    
    match &fetched_tx.transaction.transaction {
        solana_transaction_status::EncodedTransaction::Json(ui_transaction) => {
            info!("===> Processing JsonParsed transaction: {:?}", ui_transaction.signatures);
            
            // Extract account keys from the UiParsedMessage
            match &ui_transaction.message.clone() {
                solana_transaction_status::UiMessage::Parsed(parsed_message) => {
                    let account_keys: Vec<String> = parsed_message.account_keys.iter()
                        .map(|account| account.pubkey.clone())
                        .collect();
                    // Process each instruction
                    for instruction in parsed_message.instructions.iter() {
                        match instruction {
                            solana_transaction_status::UiInstruction::Parsed(parsed_instruction) => {
                                match parsed_instruction {
                                    solana_transaction_status::UiParsedInstruction::PartiallyDecoded(partially_decoded) => {
                                        // Handle partially decoded instructions (like Raydium instructions)
                                        if partially_decoded.program_id == RAYDIUM_LIQUIDITY_POOL_V4_PROGRAM_ID && partially_decoded.accounts.len() > 9 {
                                            // For partially decoded instructions, we need to use the account indices
                                                let pool_addr = partially_decoded.accounts[4].clone();
                                                let token_a = partially_decoded.accounts[8].clone();
                                                let token_b = partially_decoded.accounts[9].clone();
                                                
                                                let pool_update = WebSocketMessage::PoolUpdate(PoolUpdate {
                                                    pool_address: pool_addr.clone(),
                                                    token_a: token_a.clone(),
                                                    token_b: token_b.clone(),
                                                    liquidity: 0.0, 
                                                    volume_24h: 0.0,
                                                    timestamp: chrono::Utc::now().timestamp() as u64,
                                                });
                                                pull_updates.push(pool_update);
                                                info!("===> Created pool update for pool: {}, token_a: {}, token_b: {}", 
                                                      pool_addr, token_a, token_b);
                                        } else {
                                            info!("===> Partially decoded instruction is not a Raydium instruction: {:?}", partially_decoded.program_id);
                                        }
                                    }
                                    _ => {
                                        error!("Instruction is not PartiallyDecoded type: {:?}", instruction);
                                    }
                                }
                            }
                            // solana_transaction_status::UiInstruction::Compiled(compiled_instruction) => {
                            //     info!("===> Compiled instruction: {:?}", compiled_instruction);
                            // }
                            // solana_transaction_status::UiInstruction::PartiallyDecoded(partially_decoded) => {
                            //     info!("===> Partially decoded instruction: {:?}", partially_decoded);
                            // }
                            _ => {
                                error!("Instruction is not Parsed type: {:?}", instruction);
                            }
                        }
                    }
                }
                _ => {
                    error!("Message is not a UiParsedMessage: {:?}", ui_transaction.message);
                }
            }
            
            // Process each instruction
            Ok(pull_updates)
        }
        _ => {
            Err(anyhow::anyhow!("Transaction is not in JsonParsed format"))
        }
    }
}
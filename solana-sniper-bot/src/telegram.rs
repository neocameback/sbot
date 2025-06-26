use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use anyhow::{Result};
use log::{info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
}

#[derive(Clone)]

pub struct TelegramSender {
  telegram_config: TelegramConfig,
}

impl TelegramSender {
  pub fn new(telegram_config: TelegramConfig) -> Self {
    // Check if the bot token and chat id are valid
    if telegram_config.bot_token.is_empty() || telegram_config.chat_id.is_empty() {
      // Throw error
      panic!("Telegram config is invalid: {:?}", telegram_config);
    }

    TelegramSender { telegram_config }
  }

  pub async fn send_new_pool_telegram_message(&self, tx_hash: &str, pool_addr: &str, token_a: &str, token_b: &str) -> Result<()> {
    let bot_token = self.telegram_config.bot_token.clone();
    // Pool, token link in explorer is https://explorer.solana.com/address/xx
    let pool_link = format!("https://explorer.solana.com/address/{}", pool_addr);
    let token_a_link = format!("https://explorer.solana.com/address/{}", token_a);
    let token_b_link = format!("https://explorer.solana.com/address/{}", token_b);
    let tx_link = format!("https://explorer.solana.com/tx/{}", tx_hash);
    let chat_id = self.telegram_config.chat_id.clone();
      let msg = format!(
        "*New Raydium Pool Detected!*
        Tx Hash: `{}`
        Pool: `{}`
        Token A: `{}`
        Token B: `{}`",
        tx_link, pool_link, token_a_link, token_b_link
      );
      self.send_telegram_message(&bot_token, &chat_id, &msg).await.map_err(|e| anyhow::anyhow!("Failed to send Telegram message: {}", e))
  }

  async fn send_telegram_message(&self,bot_token: &str, chat_id: &str, text: &str) -> Result<()> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
    // let payload = [
    //     ("chat_id", chat_id),
    //     ("text", text),
    //     ("parse_mode", "MarkdownV2"),
    // ];
    let mut payload = HashMap::new();
    payload.insert("chat_id", chat_id);
    payload.insert("text", text);
    payload.insert("parse_mode", "MarkdownV2");
    info!("Sending Telegram message: {:?}", payload);
    reqwest::Client::new().post(&url).header("Content-Type", "application/json").json(&payload).send().await?;
    Ok(())
  } 
}
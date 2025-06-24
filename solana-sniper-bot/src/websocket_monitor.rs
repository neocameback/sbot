use anyhow::Result;
use log::{info, warn, error};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{SinkExt, StreamExt};
use url::Url;
use std::time::Duration;

// WebSocket message types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WebSocketMessage {
    PoolUpdate(PoolUpdate),
    TokenListing(TokenListing),
    PriceUpdate(PriceUpdate),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolUpdate {
    pub pool_address: String,
    pub token_a: String,
    pub token_b: String,
    pub liquidity: f64,
    pub volume_24h: f64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenListing {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub initial_liquidity: f64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceUpdate {
    pub token_address: String,
    pub price: f64,
    pub price_change_24h: f64,
    pub volume_24h: f64,
    pub timestamp: u64,
}

#[derive(Clone)]
pub struct WebSocketMonitor {
    url: String,
    reconnect_delay: Duration,
}

impl WebSocketMonitor {
    pub fn new(url: String, reconnect_delay_ms: u64) -> Self {
        WebSocketMonitor {
            url,
            reconnect_delay: Duration::from_millis(reconnect_delay_ms),
        }
    }

    pub async fn start_monitoring(&self, tx: mpsc::Sender<WebSocketMessage>) -> Result<()> {
        info!("Starting WebSocket monitoring for: {}", self.url);
        let url = Url::parse(&self.url)?;
        loop {
            match connect_async(url.clone()).await {
                Ok((ws_stream, _)) => {
                    info!("Connected to WebSocket: {}", self.url);
                    let (mut write, mut read) = ws_stream.split();
                    if let Err(e) = Self::send_subscription(&mut write).await {
                        error!("Failed to send subscription: {}", e);
                        tokio::time::sleep(self.reconnect_delay).await;
                        continue;
                    }
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                if let Err(e) = Self::process_message(&text, &tx).await {
                                    error!("Failed to process message: {}", e);
                                }
                            }
                            Ok(Message::Close(_)) => {
                                info!("WebSocket connection closed: {}", self.url);
                                break;
                            }
                            Err(e) => {
                                error!("WebSocket error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to connect to WebSocket {}: {}", self.url, e);
                }
            }
            tokio::time::sleep(self.reconnect_delay).await;
        }
    }

    async fn send_subscription<S>(write: &mut S) -> Result<()> 
    where
        S: SinkExt<Message> + Unpin,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
        let subscribe_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "subscribe",
            "params": ["poolUpdates", "tokenListings", "priceUpdates"]
        });
        write.send(Message::Text(subscribe_msg.to_string())).await
            .map_err(|e| anyhow::anyhow!("Failed to send subscription: {}", e))?;
        Ok(())
    }

    async fn process_message(text: &str, tx: &mpsc::Sender<WebSocketMessage>) -> Result<()> {
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(params) = data.get("params") {
                if let Some(result) = params.get("result") {
                    if let Some(method) = params.get("method").and_then(|v| v.as_str()) {
                        match method {
                            "poolUpdate" => {
                                if let Ok(pool_update) = serde_json::from_value::<PoolUpdate>(result.clone()) {
                                    if let Err(e) = tx.send(WebSocketMessage::PoolUpdate(pool_update)).await {
                                        error!("Failed to send pool update: {}", e);
                                    }
                                }
                            }
                            "tokenListing" => {
                                if let Ok(token_listing) = serde_json::from_value::<TokenListing>(result.clone()) {
                                    if let Err(e) = tx.send(WebSocketMessage::TokenListing(token_listing)).await {
                                        error!("Failed to send token listing: {}", e);
                                    }
                                }
                            }
                            "priceUpdate" => {
                                if let Ok(price_update) = serde_json::from_value::<PriceUpdate>(result.clone()) {
                                    if let Err(e) = tx.send(WebSocketMessage::PriceUpdate(price_update)).await {
                                        error!("Failed to send price update: {}", e);
                                    }
                                }
                            }
                            _ => {
                                warn!("Unknown WebSocket method: {}", method);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct DexWebSocketManager {
    monitors: Vec<WebSocketMonitor>,
    message_tx: mpsc::Sender<WebSocketMessage>,
    message_rx: std::sync::Arc<tokio::sync::Mutex<mpsc::Receiver<WebSocketMessage>>>,
}

impl DexWebSocketManager {
    pub fn new() -> Self {
        let (message_tx, message_rx) = mpsc::channel(1000);
        DexWebSocketManager {
            monitors: Vec::new(),
            message_tx,
            message_rx: std::sync::Arc::new(tokio::sync::Mutex::new(message_rx)),
        }
    }

    pub fn add_monitor(&mut self, url: String, reconnect_delay_ms: u64) {
        let monitor = WebSocketMonitor::new(url, reconnect_delay_ms);
        self.monitors.push(monitor);
    }

    pub async fn start_all_monitors(&self) -> Result<()> {
        let mut handles = Vec::new();
        for monitor in &self.monitors {
            let tx = self.message_tx.clone();
            let monitor_clone = monitor.clone();
            let handle = tokio::spawn(async move {
                if let Err(e) = monitor_clone.start_monitoring(tx).await {
                    error!("Monitor failed: {}", e);
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            if let Err(e) = handle.await {
                error!("Monitor task failed: {}", e);
            }
        }
        Ok(())
    }

    pub fn get_message_receiver(&self) -> std::sync::Arc<tokio::sync::Mutex<mpsc::Receiver<WebSocketMessage>>> {
        self.message_rx.clone()
    }

    pub async fn receive_message(&self) -> Option<WebSocketMessage> {
        let mut rx_guard = self.message_rx.lock().await;
        rx_guard.recv().await
    }

    pub fn get_message_sender(&self) -> mpsc::Sender<WebSocketMessage> {
        self.message_tx.clone()
    }
} 
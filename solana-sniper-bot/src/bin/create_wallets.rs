use anyhow::Result;
use log::info;
use solana_sdk::signature::{Keypair, Signer};
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    env_logger::init();
    
    // Create wallets directory if it doesn't exist
    if !Path::new("wallets").exists() {
        fs::create_dir("wallets")?;
    }
    
    let num_wallets = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);
    
    info!("Creating {} wallets...", num_wallets);
    
    for i in 1..=num_wallets {
        let keypair = Keypair::new();
        let wallet_name = format!("wallet{}", i);
        let wallet_data = serde_json::to_string_pretty(&keypair.to_bytes().to_vec())?;
        
        fs::write(format!("wallets/{}.json", wallet_name), wallet_data)?;
        info!("Created wallet {}: {}", wallet_name, keypair.pubkey());
    }
    
    info!("Successfully created {} wallets in wallets/ directory", num_wallets);
    Ok(())
} 
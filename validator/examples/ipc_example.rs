//! IPC module usage example
//! 
//! This example demonstrates how to use the IPC module to send Solana transactions via Unix domain socket

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use solana_sdk::{
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
    pubkey::Pubkey,
};
use agave_validator::bridge::{
    bridge::Bridge,
    config::MultivmConfig,
    ipc::{IpcServer, IpcServerConfig, IpcClient},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    println!("Starting IPC example...");

    // Configuration parameters
    let (rpc_url, websocket_url) = MultivmConfig::urls();
    let socket_path = "/tmp/solana_bridge.sock".to_string();

    println!("Starting IPC example...");

    // Create Bridge instance
    let bridge = match Bridge::new(rpc_url, websocket_url) {
        Ok(bridge) => Arc::new(bridge),
        Err(e) => {
            eprintln!("Unable to create Bridge: {}", e);
            eprintln!("Please ensure Solana node is running on {}", MultivmConfig::RPC_URL);
            return Ok(());
        }
    };

    // Create IPC server configuration
    let config = IpcServerConfig {
        socket_path: socket_path.clone(),
        bridge: Arc::clone(&bridge),
    };

    // Start IPC server
    let mut server = IpcServer::new(config);
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start().await {
            eprintln!("IPC server error: {}", e);
        }
    });

    // Wait for server to start
    sleep(Duration::from_millis(500)).await;
    println!("IPC server started, socket path: {}", socket_path);

    // Create test transactions
    println!("Creating test transactions...");
    let from_keypair = Keypair::new();
    let to_keypair = Keypair::new();
    
    println!("Sender address: {}", from_keypair.pubkey());
    println!("Receiver address: {}", to_keypair.pubkey());

    // First request airdrop for sender (if on test network)
    println!("Requesting airdrop for sender...");
    match bridge.airdrop(&from_keypair.pubkey(), 1_000_000_000) {
        Ok(signature) => {
            println!("Airdrop transaction signature: {}", signature);
            // Wait for confirmation
            if let Some(result) = bridge.confirm_transaction(&signature) {
                match result {
                    Ok(()) => println!("Airdrop confirmed successfully"),
                    Err(e) => println!("Airdrop failed: {:?}", e),
                }
            } else {
                println!("Airdrop confirmation timeout");
            }
        }
        Err(e) => {
            println!("Airdrop request failed: {}", e);
            println!("Continuing with existing balance for testing...");
        }
    }

    // Create transfer transaction
    let lamports = 100_000; // 0.0001 SOL
    let recent_blockhash = bridge.rpc_client.get_latest_blockhash()
        .map_err(|e| format!("Failed to get latest blockhash: {}", e))?;

    let instruction = system_instruction::transfer(
        &from_keypair.pubkey(),
        &to_keypair.pubkey(),
        lamports,
    );

    let mut transaction = Transaction::new_with_payer(&[instruction], Some(&from_keypair.pubkey()));
    transaction.sign(&[&from_keypair], recent_blockhash);

    let transactions = vec![transaction];
    let signers = vec![from_keypair];

    // Create IPC client and send transactions
    println!("Sending transactions via IPC...");
    let client = IpcClient::new(socket_path.clone());
    
    match client.send_batch_transactions(transactions, signers).await {
        Ok(success) => {
            if success {
                println!("✅ Transactions sent successfully via IPC!");
            } else {
                println!("❌ Transaction sending failed");
            }
        }
        Err(e) => {
            println!("❌ IPC communication error: {}", e);
        }
    }

    // Wait for server to finish processing
    sleep(Duration::from_secs(2)).await;

    // Stop server
    server_handle.abort();
    println!("Example completed");

    Ok(())
}
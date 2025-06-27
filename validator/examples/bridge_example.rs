//! Bridge module usage example
//! 
//! This example demonstrates how to use the Bridge module directly to interact with Solana blockchain.
//! It showcases core Bridge functionality including transfers, airdrops, and batch transaction processing.

use std::sync::Arc;
use solana_sdk::{
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
    pubkey::Pubkey,
};
use agave_validator::bridge::{
    bridge::Bridge,
    config::MultivmConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🌉 Starting Bridge example...");

    // Get configuration URLs
    let (rpc_url, websocket_url) = MultivmConfig::urls();
    println!("📡 Connecting to Solana node:");
    println!("   RPC URL: {}", rpc_url);
    println!("   WebSocket URL: {}", websocket_url);

    // Create Bridge instance
    let bridge = match Bridge::new(rpc_url, websocket_url) {
        Ok(bridge) => Arc::new(bridge),
        Err(e) => {
            eprintln!("❌ Unable to create Bridge: {}", e);
            eprintln!("💡 Please ensure Solana node is running on {}", MultivmConfig::RPC_URL);
            eprintln!("💡 You may need to update the URLs in validator/src/bridge/config.rs");
            return Ok(());
        }
    };

    println!("✅ Bridge created successfully!");

    // Example 1: Airdrop functionality
    println!("\n🪂 Example 1: Airdrop functionality");
    let alice = Keypair::new();
    println!("👤 Alice's address: {}", alice.pubkey());

    let airdrop_amount = 2_000_000_000; // 2 SOL
    match bridge.airdrop(&alice.pubkey(), airdrop_amount) {
        Ok(signature) => {
            println!("📤 Airdrop transaction sent: {}", signature);
            
            // Wait for confirmation
            match bridge.confirm_transaction(&signature) {
                Some(Ok(())) => {
                    println!("✅ Airdrop confirmed successfully!");
                    
                    // Check balance
                    if let Ok(balance) = bridge.rpc_client.get_balance(&alice.pubkey()) {
                        println!("💰 Alice's balance: {} lamports ({:.2} SOL)", 
                               balance, balance as f64 / 1_000_000_000.0);
                    }
                }
                Some(Err(e)) => {
                    println!("❌ Airdrop failed: {:?}", e);
                }
                None => {
                    println!("⏰ Airdrop confirmation timeout");
                }
            }
        }
        Err(e) => {
            println!("❌ Airdrop request failed: {}", e);
            println!("💡 This might be expected on mainnet or if airdrop limits are reached");
        }
    }

    // Example 2: Simple transfer
    println!("\n💸 Example 2: Simple transfer");
    let bob = Keypair::new();
    println!("👤 Bob's address: {}", bob.pubkey());

    let transfer_amount = 100_000_000; // 0.1 SOL
    match bridge.transfer(&alice, &bob.pubkey(), transfer_amount) {
        Ok(signature) => {
            println!("📤 Transfer transaction sent: {}", signature);
            
            // Wait for confirmation
            match bridge.confirm_transaction(&signature) {
                Some(Ok(())) => {
                    println!("✅ Transfer confirmed successfully!");
                    
                    // Check both balances
                    if let (Ok(alice_balance), Ok(bob_balance)) = (
                        bridge.rpc_client.get_balance(&alice.pubkey()),
                        bridge.rpc_client.get_balance(&bob.pubkey())
                    ) {
                        println!("💰 Alice's balance: {} lamports ({:.2} SOL)", 
                               alice_balance, alice_balance as f64 / 1_000_000_000.0);
                        println!("💰 Bob's balance: {} lamports ({:.2} SOL)", 
                               bob_balance, bob_balance as f64 / 1_000_000_000.0);
                    }
                }
                Some(Err(e)) => {
                    println!("❌ Transfer failed: {:?}", e);
                }
                None => {
                    println!("⏰ Transfer confirmation timeout");
                }
            }
        }
        Err(e) => {
            println!("❌ Transfer request failed: {}", e);
        }
    }

    // Example 3: Batch transaction processing
    println!("\n📦 Example 3: Batch transaction processing");
    
    // Create multiple recipients
    let recipients: Vec<Keypair> = (0..3).map(|_| Keypair::new()).collect();
    println!("👥 Created {} recipients", recipients.len());
    
    // Create transactions for each recipient
    let mut transactions = Vec::new();
    let recent_blockhash = match bridge.rpc_client.get_latest_blockhash() {
        Ok(blockhash) => blockhash,
        Err(e) => {
            println!("❌ Failed to get latest blockhash: {}", e);
            return Ok(());
        }
    };

    let amount_per_recipient = 50_000_000; // 0.05 SOL each
    for (i, recipient) in recipients.iter().enumerate() {
        let instruction = system_instruction::transfer(
            &alice.pubkey(),
            &recipient.pubkey(),
            amount_per_recipient,
        );

        let mut transaction = Transaction::new_with_payer(&[instruction], Some(&alice.pubkey()));
        transaction.sign(&[&alice], recent_blockhash);
        transactions.push(transaction);
        
        println!("📝 Transaction {} created for recipient: {}", i + 1, recipient.pubkey());
    }

    // Send transactions sequentially using Bridge's batch method
    println!("🚀 Sending {} transactions sequentially...", transactions.len());
    match bridge.send_and_confirm_transactions_sequentially(&mut transactions, &[&alice]) {
        Ok(()) => {
            println!("✅ All batch transactions confirmed successfully!");
            
            // Check recipient balances
            for (i, recipient) in recipients.iter().enumerate() {
                if let Ok(balance) = bridge.rpc_client.get_balance(&recipient.pubkey()) {
                    println!("💰 Recipient {} balance: {} lamports ({:.3} SOL)", 
                           i + 1, balance, balance as f64 / 1_000_000_000.0);
                }
            }
        }
        Err(e) => {
            println!("❌ Batch transaction processing failed: {}", e);
        }
    }

    println!("\n🎉 Bridge example completed successfully!");
    println!("💡 This example demonstrated:");
    println!("   • Creating a Bridge instance");
    println!("   • Requesting airdrops");
    println!("   • Performing simple transfers");
    println!("   • Processing batch transactions");
    println!("   • Transaction confirmation mechanisms");

    Ok(())
}
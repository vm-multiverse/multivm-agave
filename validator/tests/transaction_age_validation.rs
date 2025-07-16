use log::{debug, error, info, warn};
use solana_sdk::commitment_config::CommitmentLevel;
use solana_sdk::signature::Signature;
use {
    assert_cmd::prelude::*,
    solana_client::{
        rpc_client::RpcClient,
        rpc_config::RpcSendTransactionConfig,
        client_error::ClientError,
    },
    solana_sdk::{
        clock::MAX_PROCESSING_AGE,
        commitment_config::CommitmentConfig,
        hash::Hash,
        message::Message,
        pubkey::Pubkey,
        signature::{Keypair, Signer},
        system_instruction,
        transaction::Transaction,
        native_token::LAMPORTS_PER_SOL,
    },
    std::{
        process::{Child, Command},
        thread,
        time::Duration,
    },
};

use agave_validator::bridge::ipc::IpcClient;
#[test]
#[ignore] // Requires manual execution with validator running
fn test_transaction_age_validation() {
    let rpc_url = "http://127.0.0.1:8899".to_string();
    let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::processed());
    let ipc_client = IpcClient::new("/tmp/solana-private-validator".to_string());
    let nb_block_number = MAX_PROCESSING_AGE + 10;

    let expire_block_hash = rpc_client.get_latest_blockhash().unwrap();
    println!("Current block hash: {}, height: {}", expire_block_hash, rpc_client.get_block_height().unwrap());
    
    // Create test accounts
    let from = Keypair::new();
    let to = Pubkey::new_unique();

    // Airdrop SOL to sender account
    println!("Requesting airdrop for account: {}", from.pubkey());
    ipc_client.tick().unwrap();
    thread::sleep(Duration::from_secs(1));
    let airdrop_amount = 2 * LAMPORTS_PER_SOL; // Airdrop 2 SOL
    let airdrop_signature = rpc_client
        .request_airdrop(&from.pubkey(), airdrop_amount)
        .expect("Failed to request airdrop");

    // Wait for airdrop confirmation with longer timeout and better error handling
    println!("Waiting for airdrop confirmation...");
    ipc_client.tick().unwrap();
    thread::sleep(Duration::from_secs(1));
    ipc_client.tick().unwrap();
    thread::sleep(Duration::from_secs(1));
    ipc_client.tick().unwrap();
    thread::sleep(Duration::from_secs(1));
    ipc_client.tick().unwrap();
    thread::sleep(Duration::from_secs(1));
    ipc_client.tick().unwrap();
    thread::sleep(Duration::from_secs(1));

    let mut airdrop_confirmed = false;
    let max_attempts = 120; // Increased from 30 to 120 (60 seconds total)

    for attempt in 1..=max_attempts {
        if attempt % 10 == 0 {
            println!("Airdrop confirmation attempt {}/{}", attempt, max_attempts);
        }

        match rpc_client.get_signature_status(&airdrop_signature) {
            Ok(Some(Ok(_))) => {
                airdrop_confirmed = true;
                println!("Airdrop confirmed successfully");
                break;
            }
            Ok(Some(Err(e))) => {
                panic!("Airdrop failed with error: {}", e);
            }
            Ok(None) => {
                // Transaction not yet processed, continue waiting
                thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                println!("Warning: Error checking airdrop status (attempt {}): {}", attempt, e);
                thread::sleep(Duration::from_millis(500));
            }
        }
    }

    if !airdrop_confirmed {
        // Try to check balance anyway in case the signature status check failed
        let balance = rpc_client.get_balance(&from.pubkey()).unwrap_or(0);
        if balance >= airdrop_amount {
            println!("Airdrop appears successful based on balance check: {} lamports", balance);
        } else {
            panic!("Airdrop confirmation timeout after {} attempts. Current balance: {} lamports",
                   max_attempts, balance);
        }
    }
    
    let balance = rpc_client.get_balance(&from.pubkey()).unwrap();
    println!("Airdrop successful, account balance: {} lamports", balance);
    
    // Use ipc_client.tick() to advance nb_block_number slots
    println!("Starting to advance {} blocks...", nb_block_number);
    let initial_height = rpc_client.get_block_height().unwrap();
    
    for _ in 0..nb_block_number {
        // 2 ticks per block - each tick() call is synchronous and blocks until complete
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));

    }

    let final_height = rpc_client.get_block_height().unwrap();
    let blocks_advanced = final_height - initial_height;
    println!("Advancement complete, current block hash: {}, height: {} (advanced {} blocks)",
             rpc_client.get_latest_blockhash().unwrap(),
             final_height,
             blocks_advanced);
             
    // Verify we actually advanced enough blocks
    if blocks_advanced < nb_block_number as u64 {
        println!("Warning: Expected to advance {} blocks but only advanced {} blocks",
                 nb_block_number, blocks_advanced);
    }

    let valid_block_hash  = rpc_client.get_latest_blockhash().unwrap();
    let invalid_block_hash = solana_sdk::hash::Hash::from([0u8; 32]);  // invalid
    // Construct a transaction using expired blockhash
    let transfer_amount = 1000; // Transfer 1000 lamports
    let transfer_instruction =
        system_instruction::transfer(&from.pubkey(), &to, transfer_amount);

    // Create message and transaction using expired blockhash
    let message = Message::new(&[transfer_instruction], Some(&from.pubkey()));
    // test expire block hash
    {
        let mut expire_transaction = Transaction::new_unsigned(message.clone());
        expire_transaction.sign(&[&from], expire_block_hash);
        println!("Attempting to send transaction with expired blockhash...");
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        // Send transaction, expecting it to be rejected
        let send_result = rpc_client.send_transaction(&expire_transaction);
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));

        match send_result {
            Ok(signature) => {
                thread::sleep(Duration::from_secs(2));
                match rpc_client.get_signature_status_with_commitment(
                    &signature,
                    CommitmentConfig {
                        commitment: CommitmentLevel::Processed,
                    },
                ) {
                    Ok(Some(Ok(_))) => {
                        println!("✅ TEST PASSED: Transaction with expired blockhash should be confirmed!");
                    }
                    Ok(Some(Err(e))) => {
                        panic!("Transaction was eventually rejected, error: {}", e);
                    }
                    Ok(None) => {
                        panic!("Transaction was not processed");
                    }
                    Err(e) => {
                        panic!("Error checking transaction status: {}", e);
                    }
                }
            }
            Err(e) => {
                // This is the expected result - transaction should be rejected at send time
                panic!("{}", format!("TEST FAILED: Transaction with expired blockhash was correctly rejected,Rejection reason: {}", e));
            }
        }
    }
    println!();
    println!();
    // test invalid transaction
    {
        let mut invalid_transaction = Transaction::new_unsigned(message.clone());
        invalid_transaction.sign(&[&from], invalid_block_hash);
        println!("Attempting to send transaction with invalid blockhash...");
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        // Send transaction, expecting it to be rejected
        let send_result = rpc_client.send_transaction(&invalid_transaction);
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));

        match send_result {
            Ok(signature) => {
                thread::sleep(Duration::from_secs(2));
                match rpc_client.get_signature_status_with_commitment(
                    &signature,
                    CommitmentConfig {
                        commitment: CommitmentLevel::Processed,
                    },
                ) {
                    Ok(Some(Ok(_))) => {
                        panic!("TEST FAILED: Transaction with invalid blockhash should not be confirmed!");
                    }
                    Ok(Some(Err(e))) => {
                        println!("Transaction was eventually rejected, error: {}", e);
                    }
                    Ok(None) => {
                        println!("Transaction was not processed");
                    }
                    Err(e) => {
                        println!("Error checking transaction status: {}", e);
                    }
                }
            }
            Err(e) => {
                // This is the expected result - transaction should be rejected at send time
                println!("✅ TEST PASSED: Transaction with invalid blockhash was correctly rejected");
                println!("Rejection reason: {}", e);
            }
        }
    }
    println!();
    println!();
    // test valid transaction
    {
        let mut invalid_transaction = Transaction::new_unsigned(message.clone());
        invalid_transaction.sign(&[&from], valid_block_hash);
        println!("Attempting to send transaction with valid blockhash...");
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        // Send transaction, expecting it to be rejected
        let send_result = rpc_client.send_transaction(&invalid_transaction);
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_secs(1));

        match send_result {
            Ok(signature) => {
                thread::sleep(Duration::from_secs(2));
                match rpc_client.get_signature_status_with_commitment(
                    &signature,
                    CommitmentConfig {
                        commitment: CommitmentLevel::Processed,
                    },
                ) {
                    Ok(Some(Ok(_))) => {
                        println!("✅ TEST PASSED: Transaction with valid blockhash is confirmed!");
                    }
                    Ok(Some(Err(e))) => {
                        panic!("TEST FAILED: Valid Transaction was eventually rejected, error: {}", e);
                    }
                    Ok(None) => {
                        panic!("TEST FAILED: Valid Transaction was not processed");
                    }
                    Err(e) => {
                        panic!("TEST FAILED: Error checking valid transaction status: {}", e);
                    }
                }
            }
            Err(e) => {
                // This is the expected result - transaction should be rejected at send time
                panic!("TEST FAILED: Transaction with valid blockhash was rejected");
                println!("Rejection reason: {}", e);
            }
        }
    }
    
    println!("Test completed");
}



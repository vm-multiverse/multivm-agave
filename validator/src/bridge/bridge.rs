use std::time::{Duration, Instant};
use std::{iter::repeat_with, sync::Arc};

use solana_client::connection_cache::ConnectionCache;
use solana_connection_cache::connection_cache::NewConnectionConfig;
use solana_quic_client::{QuicConfig, QuicConnectionManager, QuicPool};
use solana_rpc_client::rpc_client::RpcClient;
use solana_rpc_client_api::client_error::Result as ClientResult;
use solana_sdk::hash::Hash;
use solana_sdk::system_transaction;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    message::Message,
    pubkey::{self, Pubkey},
    signature::{Keypair, Signature, Signer},
    system_instruction,
    transaction::Transaction,
};
use solana_tpu_client::tpu_client::{TpuClient, TpuClientConfig};
use solana_transaction_error::TransactionResult;
use tokio::time::timeout;

pub struct Bridge {
    pub tpu_client: Arc<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>>,
    pub rpc_client: Arc<RpcClient>,
}

impl Bridge {
    pub fn new(rpc_url: String, websocket_url: String) -> Result<Self, String> {
        let rpc_client = Arc::new(RpcClient::new_with_commitment(
            rpc_url,
            CommitmentConfig::processed(),
        ));

        let connection_cache = ConnectionCache::new_quic("bridge_connection_cache", 1);
        let cache = if let ConnectionCache::Quic(cache) = connection_cache {
            cache
        } else {
            return Err("Expected a Quic connection cache, but got something else.".to_string());
        };
        let tpu_client = TpuClient::new_with_connection_cache(
            Arc::clone(&rpc_client),
            websocket_url.as_str(),
            TpuClientConfig::default(),
            cache,
        )
        .map_err(|e| format!("Failed to build TpuClient: {}", e))?;
        Ok(Self {
            tpu_client: Arc::new(tpu_client),
            rpc_client,
        })
    }

    pub fn transfer(
        &self,
        from_keypair: &Keypair,
        to_pubkey: &Pubkey,
        lamports: u64,
    ) -> ClientResult<Signature> {
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let transaction =
            system_transaction::transfer(from_keypair, to_pubkey, lamports, recent_blockhash);
        self.tpu_client.send_transaction(&transaction);
        Ok(transaction.signatures[0])
    }

    pub fn airdrop(&self, to_pubkey: &Pubkey, lamports: u64) -> ClientResult<Signature> {
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let signature = self.rpc_client.request_airdrop_with_blockhash(
            to_pubkey,
            lamports,
            &recent_blockhash,
        )?;
        Ok(signature)
    }

    pub fn confirm_transaction(&self, signature: &Signature) -> Option<TransactionResult<()>> {
        let now = Instant::now();
        // Wait up to 10 seconds for confirmation.
        let timeout = Duration::from_secs(10);
        loop {
            if now.elapsed() > timeout {
                return None;
            }

            if let Ok(status) = self
                .rpc_client
                .get_signature_status_with_commitment(signature, CommitmentConfig::processed())
            {
                if status.is_some() {
                    return status;
                }
            }
            // On RPC error or status is None, sleep and retry.
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn send_and_confirm_transactions_sequentially(
        &self,
        transactions: &mut [Transaction],
        signers: &[&Keypair],
    ) -> Result<(), String> {
        for transaction in transactions {
            let recent_blockhash = self
                .rpc_client
                .get_latest_blockhash()
                .map_err(|e| e.to_string())?;
            transaction.sign(signers, recent_blockhash);
            self.tpu_client.send_transaction(transaction);
            let signature = &transaction.signatures[0];
            match self.confirm_transaction(signature) {
                Some(Ok(())) => {
                    // Transaction confirmed successfully, continue to the next one.
                }
                Some(Err(e)) => {
                    // Transaction failed to process.
                    return Err(format!("Transaction {} failed: {:?}", signature, e));
                }
                None => {
                    // Transaction confirmation timed out.
                    return Err(format!(
                        "Confirmation timed out for transaction {}",
                        signature
                    ));
                }
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
mod tests {
    use rand::Rng;

    use super::*;

    const RPC_URL: &str = "http://100.68.83.77:8899";
    const WEBSOCKET_URL: &str = "ws://100.68.83.77:8900";

    #[test]
    fn test_no_fee() {
        let bridge = Bridge::new(RPC_URL.to_string(), WEBSOCKET_URL.to_string()).unwrap();
        let alice = tmp_keypair();

        // Airdrop 1000 SOL to Alice
        let lamports = 1000_000_000_000;
        let signature = bridge.airdrop(&alice.pubkey(), lamports).unwrap();
        let status = bridge.confirm_transaction(&signature).unwrap();
        assert_eq!(status, Ok(()));

        // Create 10 transactions to send SOL to random addresses
        let mut total_send = 0;
        let mut transactions = Vec::new();
        let recent_blockhash = bridge.rpc_client.get_latest_blockhash().unwrap();
        for _ in 0..10 {
            let to = tmp_keypair();
            let rand_lamports = rand::thread_rng().gen_range(1..=1000);
            let transaction =
                system_transaction::transfer(&alice, &to.pubkey(), rand_lamports, recent_blockhash);
            transactions.push(transaction);
            total_send += rand_lamports;
        }

        // Send and confirm all transactions at once.
        assert_eq!(
            bridge.send_and_confirm_transactions_sequentially(&mut transactions, &[&alice]),
            Ok(())
        );

        // Check Alice's balance
        let balance = bridge.rpc_client.get_balance(&alice.pubkey()).unwrap();
        assert_eq!(balance, lamports - total_send);
    }

    #[test]
    fn test_request_send() {
        let bridge = Bridge::new(RPC_URL.to_string(), WEBSOCKET_URL.to_string()).unwrap();
        let alice = alice();
        let bob = bob();

        // Get balances before the transaction
        let alice_balance_before = bridge.rpc_client.get_balance(&alice.pubkey()).unwrap();
        let bob_balance_before = bridge.rpc_client.get_balance(&bob.pubkey()).unwrap();

        let lamports = 0_010_000_000;
        let signature = bridge.transfer(&alice, &bob.pubkey(), lamports).unwrap();
        let status = bridge.confirm_transaction(&signature).unwrap();
        assert_eq!(status, Ok(()), "Signature: {}", signature);

        // Check final balances
        let alice_balance_after = bridge.rpc_client.get_balance(&alice.pubkey()).unwrap();
        let bob_balance_after = bridge.rpc_client.get_balance(&bob.pubkey()).unwrap();

        assert_eq!(alice_balance_after, alice_balance_before - lamports);
        assert_eq!(bob_balance_after, bob_balance_before + lamports);
    }

    #[test]
    fn test_request_airdrop() {
        let bridge = Bridge::new(RPC_URL.to_string(), WEBSOCKET_URL.to_string()).unwrap();
        let alice = alice();
        let lamports = 1_000_000_000;
        let signature = bridge.airdrop(&alice.pubkey(), lamports).unwrap();
        let status = bridge.confirm_transaction(&signature).unwrap();
        assert_eq!(status, Ok(()), "Signature: {}", signature);
    }

    fn alice() -> Keypair {
        Keypair::from_bytes(&[
            182, 66, 221, 204, 169, 194, 132, 75, 137, 215, 189, 243, 67, 178, 228, 32, 139, 231,
            102, 191, 0, 115, 156, 92, 17, 9, 92, 204, 163, 255, 248, 12, 139, 243, 50, 97, 252,
            102, 133, 250, 20, 225, 37, 44, 11, 194, 65, 202, 183, 253, 150, 254, 16, 171, 151, 23,
            106, 46, 176, 21, 49, 64, 120, 56,
        ])
        .unwrap()
    }

    fn bob() -> Keypair {
        Keypair::from_bytes(&[
            165, 63, 205, 72, 74, 49, 76, 51, 203, 81, 5, 101, 135, 76, 240, 152, 12, 13, 94, 149,
            99, 174, 220, 74, 239, 147, 71, 156, 128, 42, 134, 125, 170, 77, 134, 241, 250, 18,
            128, 159, 183, 32, 90, 139, 60, 115, 23, 34, 159, 7, 99, 151, 74, 20, 25, 115, 104, 5,
            197, 65, 110, 147, 199, 134,
        ])
        .unwrap()
    }

    fn tmp_keypair() -> Keypair {
        Keypair::new()
    }
}

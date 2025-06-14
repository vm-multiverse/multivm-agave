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
use tokio::time::timeout;

fn request_and_confirm_airdrop(
    rpc_client: Arc<RpcClient>,
    to_pubkey: &Pubkey,
    lamports: u64,
) -> ClientResult<Signature> {
    let recent_blockhash = rpc_client.get_latest_blockhash()?;
    let signature =
        rpc_client.request_airdrop_with_blockhash(to_pubkey, lamports, &recent_blockhash)?;
    rpc_client.confirm_transaction_with_spinner(
        &signature,
        &recent_blockhash,
        CommitmentConfig::processed(),
    )?;
    println!("{}", signature);
    Ok(signature)
}

fn init_airdrop(
    rpc_client: Arc<RpcClient>,
    to_pubkey: &Pubkey,
    lamports: u64,
) -> ClientResult<Signature> {
    request_and_confirm_airdrop(rpc_client, to_pubkey, lamports)
}

#[test]
fn tmp_keypair() {
    let keypair = Keypair::new();
    println!("{:?}", keypair.to_bytes());
}

fn rpc_client() -> Arc<RpcClient> {
    let rpc_url = "http://100.68.83.77:8899";
    let websocket_url = "ws://100.68.83.77:8900";
    let rpc_client = Arc::new(RpcClient::new_with_commitment(
        rpc_url.to_string(),
        CommitmentConfig::processed(),
    ));
    return rpc_client;
}

fn tpu_client() -> (
    Arc<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>>,
    Arc<RpcClient>,
) {
    let rpc_url = "http://100.68.83.77:8899";
    let websocket_url = "ws://100.68.83.77:8900";
    let rpc_client = Arc::new(RpcClient::new_with_commitment(
        rpc_url.to_string(),
        CommitmentConfig::processed(),
    ));

    let connection_cache = ConnectionCache::new_quic("connection_cache_test", 1);
    let tpu_client = match connection_cache {
        ConnectionCache::Quic(cache) => TpuClient::new_with_connection_cache(
            Arc::clone(&rpc_client),
            websocket_url,
            TpuClientConfig::default(),
            cache,
        )
        .expect("Should build Quic Client."),
        _ => {
            todo!()
        }
    };
    return (Arc::new(tpu_client), Arc::clone(&rpc_client));
}

fn alice() -> Keypair {
    Keypair::from_bytes(&[
        182, 66, 221, 204, 169, 194, 132, 75, 137, 215, 189, 243, 67, 178, 228, 32, 139, 231, 102,
        191, 0, 115, 156, 92, 17, 9, 92, 204, 163, 255, 248, 12, 139, 243, 50, 97, 252, 102, 133,
        250, 20, 225, 37, 44, 11, 194, 65, 202, 183, 253, 150, 254, 16, 171, 151, 23, 106, 46, 176,
        21, 49, 64, 120, 56,
    ])
    .unwrap()
}

fn bob() -> Keypair {
    Keypair::from_bytes(&[
        165, 63, 205, 72, 74, 49, 76, 51, 203, 81, 5, 101, 135, 76, 240, 152, 12, 13, 94, 149, 99,
        174, 220, 74, 239, 147, 71, 156, 128, 42, 134, 125, 170, 77, 134, 241, 250, 18, 128, 159,
        183, 32, 90, 139, 60, 115, 23, 34, 159, 7, 99, 151, 74, 20, 25, 115, 104, 5, 197, 65, 110,
        147, 199, 134,
    ])
    .unwrap()
}

#[test]
fn test_init_airdrop() {
    let rpc_client = rpc_client();
    let alice = alice();
    let bob = bob();
    let _ = init_airdrop(Arc::clone(&rpc_client), &alice.pubkey(), 520000000);
    let _ = init_airdrop(Arc::clone(&rpc_client), &bob.pubkey(), 520000000);
}

#[test]
fn test_tpu_client_send_transaction() {
    let (tpu_client, rpc_client) = tpu_client();

    let alice = alice();
    let bob = bob();

    let recent_blockhash = rpc_client.get_latest_blockhash().unwrap();
    let tx = system_transaction::transfer(&alice, &bob.pubkey(), 42, recent_blockhash);
    let success = tpu_client.send_transaction(&tx);
    assert!(success);

    let timeout = Duration::from_secs(10);
    let now = Instant::now();
    let signatures = vec![tx.signatures[0]];
    loop {
        assert!(now.elapsed() < timeout);
        let statuses = rpc_client.get_signature_statuses(&signatures).unwrap();
        if !statuses.value.is_empty() && statuses.value[0].is_some() {
            println!("{:?}", statuses.value[0]);
            return;
        }
    }
}

#[test]
fn test_tpu_client_send_transactions() {
    let (tpu_client, rpc_client) = tpu_client();

    let alice = alice();
    let bob = bob();

    let recent_blockhash = rpc_client.get_latest_blockhash().unwrap();
    let tx1 = system_transaction::transfer(&alice, &bob.pubkey(), 661, recent_blockhash);
    let tx2 = system_transaction::transfer(&bob, &alice.pubkey(), 662, recent_blockhash);

    let transactions = vec![tx1];
    let success = tpu_client.try_send_transaction_batch(&transactions);
    assert!(success.is_ok());

    let timeout = Duration::from_secs(50);
    let now = Instant::now();
    let signatures: Vec<_> = transactions.iter().map(|tx| tx.signatures[0]).collect();
    loop {
        assert!(now.elapsed() < timeout);
        let statuses = rpc_client.get_signature_statuses(&signatures).unwrap();
        if !statuses.value.is_empty() && statuses.value[0].is_some() {
            println!("{:?}", statuses.value[0]);
            break;
        }
    }

    let transactions = vec![tx2];
    let success = tpu_client.try_send_transaction_batch(&transactions);
    assert!(success.is_ok());

    let timeout = Duration::from_secs(50);
    let now = Instant::now();
    let signatures: Vec<_> = transactions.iter().map(|tx| tx.signatures[0]).collect();
    loop {
        assert!(now.elapsed() < timeout);
        let statuses = rpc_client.get_signature_statuses(&signatures).unwrap();
        if !statuses.value.is_empty() && statuses.value[0].is_some() {
            println!("{:?}", statuses.value[0]);
            break;
        }
    }
}

fn main() {}

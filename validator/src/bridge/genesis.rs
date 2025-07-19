use {
    crate::bridge::{ipc::IpcClient, util::send_and_confirm_transaction},
    solana_client::rpc_client::RpcClient,
    solana_sdk::{
        pubkey::Pubkey,
        signature::{Keypair, SeedDerivable, Signer},
        system_instruction,
        transaction::Transaction,
    },
};

pub fn keypair_from_seed(seed: &[u8; 32]) -> Keypair {
    Keypair::from_seed(seed).unwrap()
}

pub fn mint_keypair() -> Keypair {
    let seed_phrase = "THERAINISME.MINT";
    let mut seed = [0u8; 32];
    let phrase_bytes = seed_phrase.as_bytes();
    let len = std::cmp::min(phrase_bytes.len(), 32);
    seed[..len].copy_from_slice(&phrase_bytes[..len]);
    keypair_from_seed(&seed)
}

pub fn faucet_keypair() -> Keypair {
    let seed_phrase = "THERAINISME.FAUCET";
    let mut seed = [0u8; 32];
    let phrase_bytes = seed_phrase.as_bytes();
    let len = std::cmp::min(phrase_bytes.len(), 32);
    seed[..len].copy_from_slice(&phrase_bytes[..len]);
    keypair_from_seed(&seed)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::bridge::{ipc::IpcClient, util::send_and_confirm_transaction},
        solana_client::rpc_client::RpcClient,
        solana_sdk::{
            pubkey::Pubkey,
            signature::{Keypair, Signer},
            system_instruction,
            transaction::Transaction,
        },
    };

    #[test]
    pub fn test_airdrop() {
        // 创建客户端连接
        let tick_client = IpcClient::new("/tmp/solana-private-validator".to_string());
        let rpc_client = RpcClient::new("http://127.0.0.1:8899".to_string());

        // 创建 faucet keypair (发送方)
        let faucet_keypair = super::faucet_keypair();

        // 创建一个随机的接收方账户
        let to_keypair = Keypair::new();
        let to_pubkey = to_keypair.pubkey();

        // 转账金额 (1 SOL)
        let transfer_amount = 1_000_000_000;

        // 创建转账指令
        let transfer_instruction =
            system_instruction::transfer(&faucet_keypair.pubkey(), &to_pubkey, transfer_amount);

        // 获取最新的 blockhash
        let recent_blockhash = match rpc_client.get_latest_blockhash() {
            Ok(blockhash) => blockhash,
            Err(e) => {
                println!("Failed to get latest blockhash: {}", e);
                return;
            }
        };

        // 创建交易
        let mut transaction =
            Transaction::new_with_payer(&[transfer_instruction], Some(&faucet_keypair.pubkey()));

        // 签名交易
        transaction.sign(&[&faucet_keypair], recent_blockhash);

        // 发送并确认交易
        match send_and_confirm_transaction(&tick_client, &rpc_client, &transaction) {
            Ok(signature) => {
                println!(
                    "✅ Airdrop successful! Transaction signature: {}",
                    signature
                );
                println!(
                    "Transferred {} lamports from {} to {}",
                    transfer_amount,
                    faucet_keypair.pubkey(),
                    to_pubkey
                );
            }
            Err(e) => {
                println!("❌ Airdrop failed: {}", e);
            }
        }
    }
}

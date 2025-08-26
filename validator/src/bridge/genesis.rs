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

        // 测试次数
        let test_count = 1000;
        
        // 初始转账金额
        let base_transfer_amount = 1_000_000_000;
        let mut successful_transactions = 0;
        let mut failed_transactions = 0;

        println!("🚀 开始执行 {} 次 airdrop 测试...", test_count);
        println!("发送方地址: {}", faucet_keypair.pubkey());
        println!("{}", "=".repeat(60));

        let start_time = std::time::Instant::now();

        for i in 1..=test_count {
            // 为每次测试创建一个新的接收方账户
            let to_keypair = Keypair::new();
            let to_pubkey = to_keypair.pubkey();

            // 计算当前转账金额（每次递增1）
            let transfer_amount = base_transfer_amount + (i) as u64;

            // 创建转账指令
            let transfer_instruction =
                system_instruction::transfer(&faucet_keypair.pubkey(), &to_pubkey, transfer_amount);

            // 获取最新的 blockhash
            let recent_blockhash = match rpc_client.get_latest_blockhash() {
                Ok(blockhash) => blockhash,
                Err(e) => {
                    println!("❌ 测试 {}: 获取 blockhash 失败: {}", i, e);
                    failed_transactions += 1;
                    continue;
                }
            };

            // 创建交易
            let mut transaction = Transaction::new_with_payer(
                &[transfer_instruction],
                Some(&faucet_keypair.pubkey()),
            );

            // 签名交易
            transaction.sign(&[&faucet_keypair], recent_blockhash);
            let test_hex_jwt_secret = "bd1fa71e224227a12439367e525610e7c0d242ecfa595ec471299b535e5d179d";
            // 发送并确认交易
            match send_and_confirm_transaction(&tick_client, &rpc_client, &transaction, test_hex_jwt_secret) {
                Ok(signature) => {
                    successful_transactions += 1;
                    if i % 100 == 0 || i <= 10 {
                        println!(
                            "✅ 测试 {}: 交易成功! 签名: {}",
                            i, signature
                        );
                        println!(
                            "   转账 {} lamports 到 {}",
                            transfer_amount, to_pubkey
                        );
                    }
                }
                Err(e) => {
                    failed_transactions += 1;
                    println!("❌ 测试 {}: 交易失败: {}", i, e);
                }
            }

            // 每100次测试显示进度
            if i % 100 == 0 {
                let elapsed = start_time.elapsed();
                let avg_time_per_tx = elapsed.as_millis() as f64 / i as f64;
                println!(
                    "📊 进度: {}/{} | 成功: {} | 失败: {} | 平均耗时: {:.2}ms/tx",
                    i, test_count, successful_transactions, failed_transactions, avg_time_per_tx
                );
                println!("{}", "-".repeat(60));
            }
        }

        let total_time = start_time.elapsed();
        let success_rate = (successful_transactions as f64 / test_count as f64) * 100.0;
        let avg_time_per_tx = total_time.as_millis() as f64 / test_count as f64;

        println!("🎯 测试完成!");
        println!("{}", "=".repeat(60));
        println!("总测试次数: {}", test_count);
        println!("成功交易: {}", successful_transactions);
        println!("失败交易: {}", failed_transactions);
        println!("成功率: {:.2}%", success_rate);
        println!("总耗时: {:.2}秒", total_time.as_secs_f64());
        println!("平均每笔交易耗时: {:.2}ms", avg_time_per_tx);
        println!("TPS (每秒交易数): {:.2}", successful_transactions as f64 / total_time.as_secs_f64());

        // 如果成功率低于90%，测试失败
        assert!(
            success_rate >= 90.0,
            "测试失败: 成功率 {:.2}% 低于预期的 90%",
            success_rate
        );
    }
}

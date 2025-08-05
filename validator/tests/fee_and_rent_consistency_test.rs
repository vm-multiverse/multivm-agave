use solana_sdk::commitment_config::CommitmentLevel;
use {
    solana_client::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::CommitmentConfig,
        message::Message,
        pubkey::Pubkey,
        signature::{Keypair, Signer},
        system_instruction,
        transaction::Transaction,
        native_token::LAMPORTS_PER_SOL,
        rent::Rent,
    },
    std::{
        thread,
        time::Duration,
    },
};

use agave_validator::bridge::ipc::IpcClient;
use agave_validator::bridge::util::send_and_confirm_transaction;

#[test]
#[ignore]
fn test_fee_consistency_across_block_heights() {
    println!("🚀 开始测试手续费在不同区块高度的一致性...");
    
    let rpc_url = "http://127.0.0.1:8899".to_string();
    let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::processed());
    let ipc_client = IpcClient::new("/tmp/solana-private-validator".to_string());

    // 创建测试账户
    let from = Keypair::new();
    let to = Pubkey::new_unique();
    let transfer_amount = 1000_000_000; // 转账1000_000_000 lamports

    println!("测试账户:");
    println!("  发送方: {}", from.pubkey());
    println!("  接收方: {}", to);
    println!("  转账金额: {} lamports", transfer_amount);

    // 空投SOL到发送方账户
    let airdrop_amount = 10 * LAMPORTS_PER_SOL; // 空投10 SOL
    let airdrop_signature = rpc_client
        .request_airdrop(&from.pubkey(), airdrop_amount)
        .expect("Failed to request airdrop");

    // 等待空投确认
    for _ in 0..10 {
        ipc_client.tick().unwrap();
        thread::sleep(Duration::from_millis(500));
    }

    let mut airdrop_confirmed = false;
    for attempt in 1..=60 {
        match rpc_client.get_signature_status(&airdrop_signature) {
            Ok(Some(Ok(_))) => {
                airdrop_confirmed = true;
                println!("✅ 空投确认成功");
                break;
            }
            Ok(Some(Err(e))) => {
                panic!("空投失败: {}", e);
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                println!("检查空投状态时出错 (尝试 {}): {}", attempt, e);
                thread::sleep(Duration::from_millis(500));
            }
        }
    }

    if !airdrop_confirmed {
        let balance = rpc_client.get_balance(&from.pubkey()).unwrap_or(0);
        if balance < airdrop_amount {
            panic!("空投确认超时，当前余额: {} lamports", balance);
        }
    }

    let balance = rpc_client.get_balance(&from.pubkey()).unwrap();
    println!("账户余额: {} lamports", balance);

    // 创建转账指令和消息
    let transfer_instruction = system_instruction::transfer(&from.pubkey(), &to, transfer_amount);
    let message = Message::new(&[transfer_instruction], Some(&from.pubkey()));

    // 存储不同区块高度的手续费信息
    let mut fee_records = Vec::new();
    let test_blocks = 5; // 测试5个不同的区块高度

    println!("\n🔍 开始在不同区块高度测试手续费一致性...");

    for block_test in 1..=test_blocks {
        println!("\n--- 测试 {}/{} ---", block_test, test_blocks);
        
        // 获取当前区块信息
        let current_height = rpc_client.get_block_height().unwrap();
        let current_blockhash = rpc_client.get_latest_blockhash().unwrap();
        
        println!("当前区块高度: {}", current_height);
        println!("当前blockhash: {}", current_blockhash);


        // 创建并发送交易
        let mut transaction = Transaction::new_unsigned(message.clone());
        transaction.sign(&[&from], current_blockhash);
        
        println!("发送交易...");
        let send_result = send_and_confirm_transaction(&ipc_client, &rpc_client, &transaction);
        match send_result{
            Ok(signature) => {
                println!("交易已发送，签名: {}", signature);
                
                // 等待交易确认
                thread::sleep(Duration::from_secs(2));
                
                match rpc_client.get_signature_status_with_commitment(
                    &signature,
                    CommitmentConfig {
                        commitment: CommitmentLevel::Processed,
                    },
                ) {
                    Ok(Some(Ok(_))) => {
                        println!("✅ 交易确认成功");
                        // 计算这笔交易的手续费
                        let fee = rpc_client.get_fee_for_message(&transaction.message).unwrap();
                        

                        println!("计算的手续费: {} lamports", fee);
                        fee_records.push((current_height, fee, fee)); // 使用计算的手续费
                    }
                    Ok(Some(Err(e))) => {
                        panic!("❌ 交易失败: {}", e);
                    }
                    Ok(None) => {
                        panic!("⚠️ 交易未被处理");
                    }
                    Err(e) => {
                        panic!("❌ 检查交易状态时出错: {}", e);
                    }
                }
            }
            Err(e) => {
                panic!("❌ 发送交易失败: {}", e);
            }
        }
        // 推进一点区块
        for _ in 0..3 {
            // 每个区块2个tick
            ipc_client.tick().unwrap();
            ipc_client.tick().unwrap();
        }
    }

    let mut calculated_fees = Vec::new();
    
    for (height, calculated_fee, actual_fee) in &fee_records {
        calculated_fees.push(*calculated_fee);
    }

    // 验证计算手续费的一致性
    let first_calculated_fee = calculated_fees[0];
    let calculated_fees_consistent = calculated_fees.iter().all(|&fee| fee == first_calculated_fee);
    
    if calculated_fees_consistent {
        println!("\n✅ 计算手续费在所有区块高度都一致: {} lamports", first_calculated_fee);
    } else {
        println!("\n❌ 计算手续费在不同区块高度不一致:");
        for (i, fee) in calculated_fees.iter().enumerate() {
            println!("  区块 {}: {} lamports", i + 1, fee);
        }
        panic!("手续费计算不一致！");
    }

    println!("\n✅ 手续费一致性测试通过！");
}
// 
// #[test]
// #[ignore]
// fn test_rent_collection_consistency() {
//     println!("🚀 开始测试租金收集的一致性...");
//     
//     let rpc_url = "http://127.0.0.1:8899".to_string();
//     let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::processed());
//     let ipc_client = IpcClient::new("/tmp/solana-private-validator".to_string());
// 
//     // 创建测试账户
//     let from = Keypair::new();
//     println!("测试账户:");
//     println!("  发送方: {}", from.pubkey());
// 
//     // 空投SOL到发送方账户
//     println!("\n💰 请求空投...");
//     let airdrop_amount = 10 * LAMPORTS_PER_SOL; // 空投10 SOL
//     let airdrop_signature = rpc_client
//         .request_airdrop(&from.pubkey(), airdrop_amount)
//         .expect("Failed to request airdrop");
//     
//     // 等待空投确认
//     println!("等待空投确认...");
//     for _ in 0..10 {
//         ipc_client.tick().unwrap();
//         thread::sleep(Duration::from_millis(500));
//     }
// 
//     let mut airdrop_confirmed = false;
//     for attempt in 1..=60 {
//         match rpc_client.get_signature_status(&airdrop_signature) {
//             Ok(Some(Ok(_))) => {
//                 airdrop_confirmed = true;
//                 println!("✅ 空投确认成功");
//                 break;
//             }
//             Ok(Some(Err(e))) => {
//                 panic!("空投失败: {}", e);
//             }
//             Ok(None) => {
//                 thread::sleep(Duration::from_millis(500));
//             }
//             Err(e) => {
//                 println!("检查空投状态时出错 (尝试 {}): {}", attempt, e);
//                 thread::sleep(Duration::from_millis(500));
//             }
//         }
//     }
// 
//     if !airdrop_confirmed {
//         let balance = rpc_client.get_balance(&from.pubkey()).unwrap_or(0);
//         if balance < airdrop_amount {
//             panic!("空投确认超时，当前余额: {} lamports", balance);
//         }
//     }
// 
//     let balance = rpc_client.get_balance(&from.pubkey()).unwrap();
//     println!("账户余额: {} lamports", balance);
// 
//     // 获取当前租金信息
//     let rent = Rent::default();
//     println!("\n当前租金配置:");
//     println!("  每字节每年租金: {} lamports", rent.lamports_per_byte_year);
//     println!("  豁免阈值: {}", rent.exemption_threshold);
//     println!("  燃烧百分比: {}%", rent.burn_percent);
// 
//     // 计算创建新账户所需的最小余额（0字节账户）
//     let min_rent_for_new_account = rent.minimum_balance(0);
//     println!("  创建新账户最小租金: {} lamports", min_rent_for_new_account);
// 
//     // 存储不同区块高度的租金信息
//     let mut rent_records = Vec::new();
//     let test_blocks = 5; // 测试5个不同的区块高度
// 
//     println!("\n🔍 开始在不同区块高度测试租金一致性...");
// 
//     for block_test in 1..=test_blocks {
//         println!("\n--- 测试 {}/{} ---", block_test, test_blocks);
//         
//         // 获取当前区块信息
//         let current_height = rpc_client.get_block_height().unwrap();
//         let current_blockhash = rpc_client.get_latest_blockhash().unwrap();
//         
//         println!("当前区块高度: {}", current_height);
//         println!("当前blockhash: {}", current_blockhash);
// 
//         // 创建新的接收账户（每次测试使用不同的账户）
//         let to = Pubkey::new_unique();
//         println!("  接收方（新账户）: {}", to);
// 
//         // 逐步增加转账金额，直到找到能成功创建账户的最小金额
//         // 从1 lamport开始，每次增加1 lamport，确保找到精确的最小值
//         let mut transfer_amount = 1u64;
//         let mut successful_amount = None;
//         let max_attempts = min_rent_for_new_account + 1000_000_000; // 足够大的尝试次数
//         let max_reasonable_amount = min_rent_for_new_account * 2; // 合理的最大金额上限
// 
//         for attempt in 1..=max_attempts {
//             // 如果转账金额超过合理上限，停止尝试
//             if transfer_amount > max_reasonable_amount {
//                 println!("  ❌ 转账金额 {} 超过合理上限 {}，停止尝试", transfer_amount, max_reasonable_amount);
//                 break;
//             }
// 
//             if attempt % 1000 == 1 || attempt <= 10 {
//                 println!("  尝试转账 {} lamports (尝试 {}/{})", transfer_amount, attempt, max_attempts);
//             }
//             
//             // 创建转账指令和消息
//             let transfer_instruction = system_instruction::transfer(&from.pubkey(), &to, transfer_amount);
//             let message = Message::new(&[transfer_instruction], Some(&from.pubkey()));
//             
//             // 创建并发送交易
//             let mut transaction = Transaction::new_unsigned(message.clone());
//             transaction.sign(&[&from], current_blockhash);
//             
//             let send_result = send_and_confirm_transaction(&ipc_client, &rpc_client, &transaction);
// 
//             match send_result {
//                 Ok(signature) => {
//                     if attempt % 1000 == 1 || attempt <= 10 {
//                         println!("    交易已发送，签名: {}", signature);
//                     }
//                     
//                     // 等待交易确认
//                     thread::sleep(Duration::from_secs(1));
//                     
//                     match rpc_client.get_signature_status_with_commitment(
//                         &signature,
//                         CommitmentConfig {
//                             commitment: CommitmentLevel::Processed,
//                         },
//                     ) {
//                         Ok(Some(Ok(_))) => {
//                             println!("    ✅ 交易确认成功，找到最小转账金额: {} lamports", transfer_amount);
//                             successful_amount = Some(transfer_amount);
//                             break;
//                         }
//                         Ok(Some(Err(e))) => {
//                             if attempt % 1000 == 1 || attempt <= 10 {
//                                 println!("    ❌ 交易失败: {}", e);
//                             }
//                             if e.to_string().contains("insufficient funds for rent") {
//                                 // 租金不足，增加转账金额
//                                 transfer_amount += 1; // 每次增加1 lamport，确保精确
//                             } else {
//                                 panic!("意外的交易错误: {}", e);
//                             }
//                         }
//                         Ok(None) => {
//                             if attempt % 1000 == 1 || attempt <= 10 {
//                                 println!("    ⚠️ 交易未被处理");
//                             }
//                             transfer_amount += 1;
//                         }
//                         Err(e) => {
//                             if attempt % 1000 == 1 || attempt <= 10 {
//                                 println!("    ❌ 检查交易状态时出错: {}", e);
//                             }
//                             transfer_amount += 1;
//                         }
//                     }
//                 }
//                 Err(e) => {
//                     if attempt % 1000 == 1 || attempt <= 10 {
//                         println!("    ❌ 发送交易失败: {}", e);
//                     }
//                     if e.to_string().contains("insufficient funds for rent") {
//                         transfer_amount += 1; // 每次增加1 lamport
//                     } else {
//                         panic!("意外的发送错误: {}", e);
//                     }
//                 }
//             }
//         }
// 
//         if let Some(amount) = successful_amount {
//             println!("  ✅ 在区块高度 {} 成功创建账户，最小转账金额: {} lamports", current_height, amount);
//             rent_records.push((current_height, amount, min_rent_for_new_account));
//         } else {
//             panic!("❌ 在区块高度 {} 无法找到成功的转账金额", current_height);
//         }
// 
//         // 推进一些区块
//         for _ in 0..3 {
//             // 每个区块2个tick
//             ipc_client.tick().unwrap();
//             ipc_client.tick().unwrap();
//         }
//     }
// 
//     // 分析结果
//     println!("\n📊 租金一致性分析结果:");
//     println!("区块高度 | 实际最小金额 | 理论租金");
//     println!("---------|-------------|----------");
//     
//     let mut actual_amounts = Vec::new();
//     let mut theoretical_rents = Vec::new();
//     
//     for (height, actual_amount, theoretical_rent) in &rent_records {
//         println!("{:8} | {:11} | {:8}", height, actual_amount, theoretical_rent);
//         actual_amounts.push(*actual_amount);
//         theoretical_rents.push(*theoretical_rent);
//     }
// 
//     // 验证实际转账金额的一致性
//     let first_actual_amount = actual_amounts[0];
//     let actual_amounts_consistent = actual_amounts.iter().all(|&amount| amount == first_actual_amount);
//     
//     // 验证理论租金的一致性
//     let first_theoretical_rent = theoretical_rents[0];
//     let theoretical_rents_consistent = theoretical_rents.iter().all(|&rent| rent == first_theoretical_rent);
//     
//     if actual_amounts_consistent {
//         println!("\n✅ 实际最小转账金额在所有区块高度都一致: {} lamports", first_actual_amount);
//     } else {
//         println!("\n❌ 实际最小转账金额在不同区块高度不一致:");
//         for (i, amount) in actual_amounts.iter().enumerate() {
//             println!("  区块 {}: {} lamports", i + 1, amount);
//         }
//         panic!("实际转账金额不一致！");
//     }
// 
//     if theoretical_rents_consistent {
//         println!("✅ 理论租金计算在所有区块高度都一致: {} lamports", first_theoretical_rent);
//     } else {
//         println!("❌ 理论租金计算在不同区块高度不一致:");
//         for (i, rent) in theoretical_rents.iter().enumerate() {
//             println!("  区块 {}: {} lamports", i + 1, rent);
//         }
//         panic!("理论租金计算不一致！");
//     }
// 
//     // 验证实际金额是否符合理论预期
//     if first_actual_amount >= first_theoretical_rent {
//         println!("✅ 实际转账金额 ({} lamports) >= 理论租金 ({} lamports)",
//                  first_actual_amount, first_theoretical_rent);
//     } else {
//         println!("❌ 实际转账金额 ({} lamports) < 理论租金 ({} lamports)",
//                  first_actual_amount, first_theoretical_rent);
//         panic!("实际转账金额小于理论租金！");
//     }
// 
//     println!("\n✅ 租金一致性测试通过！");
//     println!("📋 测试结论:");
//     println!("  ✅ 创建新账户的最小转账金额在不同区块高度保持一致");
//     println!("  ✅ 理论租金计算在不同区块高度保持一致");
//     println!("  ✅ 实际转账金额符合理论租金要求");
//     println!("  ✅ 租金机制在不同区块高度下工作正常");
// }
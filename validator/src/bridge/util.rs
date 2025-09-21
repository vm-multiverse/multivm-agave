use std::time::{SystemTime, UNIX_EPOCH};
use log::info;
use solana_sdk::account::AccountSharedData;
use solana_sdk::pubkey::Pubkey;
use jsonwebtoken::{encode, Header as JwtHeader, EncodingKey, Algorithm};

use {
    crate::bridge::{
        ipc::IpcClient,
        tick::{LocalTickClient, TickDriver},
    },
    log::{debug, error, warn},
    solana_client::rpc_client::RpcClient,
    solana_rpc_client_api::config::RpcBlockConfig,
    solana_sdk::{
        commitment_config::{CommitmentConfig, CommitmentLevel},
        hash::Hash,
        signature::{Keypair, Signature, Signer},
        system_instruction,
        transaction::Transaction,
        system_program,
    },
    solana_system_interface::instruction::SystemInstruction,
    solana_transaction_status_client_types::UiConfirmedBlock,
    std::time::Duration,
};

/// 使用默认重试设置发送并确认交易
///
/// 这是一个便捷函数，使用预设的默认参数调用 `send_and_confirm_transaction_with_config`。
///
/// ### 默认配置
/// - 最大重试次数：60次
/// - 轮询间隔：100毫秒
///
/// ### 参数
/// - `tick_client`: IPC客户端，用于在轮询过程中执行tick操作
/// - `rpc_client`: Solana RPC客户端，用于发送交易和查询状态
/// - `transaction`: 要发送的交易对象
/// - `jwt_secret`: 本地jwt秘密hex
///
/// ### 返回值
/// - `Ok(Signature)`: 交易成功确认后返回交易签名
/// - `Err(Box<dyn std::error::Error + Send + Sync>)`: 发送或确认失败时返回错误
///
/// ### 错误情况
/// - tick操作失败
/// - 交易发送失败
/// - 交易确认超时
/// - 交易执行失败
///
/// ### 示例
/// ```rust
/// let signature = send_and_confirm_transaction(&tick_client, &rpc_client, &transaction)?;
/// println!("交易已确认，签名: {}", signature);
/// ```
pub fn send_and_confirm_transaction(
    tick_client: &IpcClient,
    rpc_client: &RpcClient,
    transaction: &Transaction,
    jwt_secret: &str,
) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {
    send_and_confirm_transaction_with_driver(
        tick_client,
        rpc_client,
        transaction,
        60,                         // 默认最大重试次数
        Duration::from_millis(100), // 默认轮询间隔 100ms
        jwt_secret,
    )
}

/// 使用本地 tick（无 IPC/RPC）发送并确认交易
pub fn send_and_confirm_transaction_local(
    rpc_client: &RpcClient,
    transaction: &Transaction,
    jwt_secret: &str,
) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {
    let local = LocalTickClient::default();
    send_and_confirm_transaction_with_driver(
        &local,
        rpc_client,
        transaction,
        60,
        Duration::from_millis(100),
        jwt_secret,
    )
}

/// 使用自定义重试设置发送并确认交易
///
/// 这是核心的交易发送和确认函数，提供完整的交易生命周期管理。
/// 该函数会执行以下步骤：
/// 1. 发送交易到网络获取签名
/// 2. 轮询交易状态，每次轮询前执行tick操作
/// 3. 检查交易是否达到processed承诺级别
/// 4. 重复轮询直到确认成功或达到最大重试次数
///
/// ### 参数
/// - `tick_client`: IPC客户端，用于与验证器进行tick同步
/// - `rpc_client`: Solana RPC客户端，用于网络通信
/// - `transaction`: 要发送的交易对象
/// - `max_retries`: 最大重试次数，超过此次数将返回超时错误
/// - `poll_interval`: 轮询间隔，每次状态检查之间的等待时间
///
/// ### 返回值
/// - `Ok(Signature)`: 交易成功确认后返回交易签名
/// - `Err(Box<dyn std::error::Error + Send + Sync>)`: 操作失败时返回错误
///
/// ### 错误情况
/// - 交易发送到网络失败
/// - tick操作失败（轮询过程中）
/// - 交易在网络中执行失败
/// - 达到最大重试次数仍未确认（超时）
/// - RPC调用异常
///
/// ### 承诺级别
/// 使用 `CommitmentLevel::Processed` 级别进行确认，这意味着交易已被验证器处理
/// 但可能还未达到最终确认状态。
///
/// ### 注意事项
/// - 函数会在每次轮询前执行一次tick操作，确保验证器状态同步
/// - 轮询过程中的临时错误不会立即终止，会继续重试
/// - 只有交易执行错误才会立即返回失败
/// - 每次轮询间会等待指定的轮询间隔时间
fn send_and_confirm_transaction_with_driver<T: TickDriver>(
    tick_driver: &T,
    rpc_client: &RpcClient,
    transaction: &Transaction,
    max_retries: u32,
    poll_interval: Duration,
    jwt_secret: &str,
) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {
    // Step 1: Send transaction to get signature
    let jwt_secret = rpc_client.get_auth_token_secret();
    let jwt_secret = jwt_secret.ok_or_else(|| {
        // 记录错误日志
        error!("Failed to send transaction: JWT token not set");
        // 创建并返回自定义错误
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,  
            "JWT token not set"
        )
    })?;  

    let jwt_token = create_jwt_token(jwt_secret.as_str())?;
    let signature = rpc_client.send_transaction_with_auto_token(transaction, jwt_token).map_err(|e| {
        error!("Failed to send transaction: {}", e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Transaction send failed: {}", e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;
    debug!("Transaction sent with signature: {}", signature);
    // Step 2: Poll until commitment level is processed
    for attempt in 1..=max_retries {
        debug!(
            "Polling transaction status, attempt {}/{}",
            attempt, max_retries
        );

        // Optional: tick before status check to drive PoH
        // (kept minimal; callers can add more ticks as needed)

        match rpc_client.get_signature_status_with_commitment(
            &signature,
            CommitmentConfig {
                commitment: CommitmentLevel::Processed,
            },
        ) {
            Ok(Some(status)) => match status {
                Ok(_) => {
                    debug!(
                        "Transaction {} confirmed with processed commitment",
                        signature
                    );
                    return Ok(signature);
                }
                Err(e) => {
                    error!("Transaction {} failed: {}", signature, e);
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Transaction failed: {}", e),
                    ))
                        as Box<dyn std::error::Error + Send + Sync>);
                }
            },
            Ok(None) => {
                debug!("Transaction {} not yet processed, retrying...", signature);
            }
            Err(e) => {
                warn!("Error checking transaction status: {}, retrying...", e);
            }
        }
        // Drive another tick at the end of the attempt before sleeping
        tick_driver.trigger_tick().map_err(|e| {
            error!("Failed to tick during polling: {}", e);
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Tick failed: {}", e),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        // Wait before next poll
        std::thread::sleep(poll_interval);
    }

    // If we reach here, we've exceeded max retries
    Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "Transaction {} confirmation timeout after {} attempts",
            signature, max_retries
        ),
    )) as Box<dyn std::error::Error + Send + Sync>)
}

/// 获取区块链的创世哈希
///
/// 创世哈希是区块链网络的唯一标识符，用于确保客户端连接到正确的网络。
/// 不同的Solana网络（主网、测试网、开发网）具有不同的创世哈希。
///
/// ### 参数
/// - `rpc_client`: Solana RPC客户端，用于查询网络信息
///
/// ### 返回值
/// - `Ok(Hash)`: 成功获取创世哈希
/// - `Err(Box<dyn std::error::Error + Send + Sync>)`: 获取失败时返回错误
///
/// ### 示例
/// ```rust
/// let genesis_hash = get_genesis_hash(&rpc_client)?;
/// println!("当前网络的创世哈希: {}", genesis_hash);
/// ```
pub fn get_genesis_hash(
    rpc_client: &RpcClient,
) -> Result<Hash, Box<dyn std::error::Error + Send + Sync>> {
    rpc_client.get_genesis_hash().map_err(|e| {
        error!("Failed to get genesis hash: {}", e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to get genesis hash: {}", e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })
}

/// 获取指定槽位的区块信息
///
/// 此函数用于获取区块链中指定槽位的完整区块信息，包括交易列表、区块哈希、
/// 父区块哈希、时间戳等详细信息。
///
/// ### 参数
/// - `rpc_client`: Solana RPC客户端，用于查询区块链数据
/// - `slot`: 要查询的槽位号
///
/// ### 返回值
/// - `Ok(RpcConfirmedBlock)`: 成功获取区块信息
/// - `Err(Box<dyn std::error::Error + Send + Sync>)`: 获取失败时返回错误
///
/// ### 注意事项
/// - 使用 `CommitmentLevel::Confirmed` 承诺级别确保数据可靠性
///
/// ### 示例
/// ```rust
/// let slot = 12345;
/// let block = get_block(&rpc_client, slot)?;
/// println!("区块 {} 包含 {} 个交易", slot, block.transactions.len());
/// ```
pub fn get_block(
    rpc_client: &RpcClient,
    slot: u64,
) -> Result<UiConfirmedBlock, Box<dyn std::error::Error + Send + Sync>> {
    let config = RpcBlockConfig {
        encoding: None,
        transaction_details: None,
        rewards: None,
        commitment: Some(CommitmentConfig {
            commitment: CommitmentLevel::Confirmed,
        }),
        max_supported_transaction_version: None,
    };

    rpc_client.get_block_with_config(slot, config).map_err(|e| {
        error!("Failed to get block at slot {}: {}", slot, e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to get block at slot {}: {}", slot, e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })
}

/// 获取当前最新的槽位号
///
/// 此函数用于获取区块链网络中当前最新的槽位号
///
/// ### 参数
/// - `rpc_client`: Solana RPC客户端，用于查询网络状态
///
/// ### 返回值
/// - `Ok(u64)`: 成功获取当前槽位号
/// - `Err(Box<dyn std::error::Error + Send + Sync>)`: 获取失败时返回错误
///
/// ### 注意事项
/// - 使用 `CommitmentLevel::Processed` 承诺级别获取最新状态
///
/// ### 示例
/// ```rust
/// let current_slot = get_slot(&rpc_client)?;
/// println!("当前 Slot: {}", current_slot);
/// ```
pub fn get_slot(rpc_client: &RpcClient) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    rpc_client
        .get_slot_with_commitment(CommitmentConfig {
            commitment: CommitmentLevel::Processed,
        })
        .map_err(|e| {
            error!("Failed to get current slot: {}", e);
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get current slot: {}", e),
            )) as Box<dyn std::error::Error + Send + Sync>
        })
}

// 创建一个bank内的账户，不清楚会不会用到
// 考虑到发奖励的时候没有account咋办，逻辑上应该要先创建，在distribute里也加了这个判断
// pub fn create_bank_account()

#[derive(serde::Serialize)]
struct Claims {
    iat: u64,
    exp: u64,
}
fn create_jwt_token(secret: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let claims = Claims {
        iat: now,
        exp: now + 3600, // 1小时过期
    };

    let key = EncodingKey::from_secret(hex::decode(secret.to_string())?.as_ref());
    let token = encode(&JwtHeader::new(Algorithm::HS256), &claims, &key)?;
    Ok(token)
}
pub fn distribute_reward_to_account(rpc_client: &RpcClient, ipc_client: &IpcClient, recipient: &Pubkey, amount: u64) -> Result<Option<AccountSharedData>, Box<dyn std::error::Error + Send + Sync>> {
    // 发送RPC请求
    let jwt_secret = rpc_client.get_auth_token_secret();
    let jwt_secret = jwt_secret.ok_or_else(|| {
        // 记录错误日志
        error!("Failed to send transaction: JWT token not set");
        // 创建并返回自定义错误
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "JWT token not set"
        )
    })?;
    let jwt_token = create_jwt_token(jwt_secret.as_str())?;
    ipc_client.tick()?;
    ipc_client.tick()?;
    let response = rpc_client.distribute_reward_to_account(recipient, amount, jwt_token)
        .map_err(|e| {
            error!("Failed to send distribute reward RPC: {}", e);
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("RPC call failed: {}", e),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
    info!("Successfully distributed reward to {}", recipient);
    ipc_client.tick()?;
    ipc_client.tick()?;
    Ok(response) // todo 这里现在是返回AccountShareData
}

/// 使用本地 tick（无 IPC/RPC）分发奖励到账户
pub fn distribute_reward_to_account_local(
    rpc_client: &RpcClient,
    recipient: &Pubkey,
    amount: u64,
) -> Result<Option<AccountSharedData>, Box<dyn std::error::Error + Send + Sync>> {
    let jwt_secret = rpc_client.get_auth_token_secret();
    let jwt_secret = jwt_secret.ok_or_else(|| {
        error!("Failed to send transaction: JWT token not set");
        std::io::Error::new(std::io::ErrorKind::InvalidData, "JWT token not set")
    })?;

    let jwt_token = create_jwt_token(jwt_secret.as_str())?;
    let driver = LocalTickClient::default();
    driver.trigger_tick()?;
    driver.trigger_tick()?;
    let response = rpc_client
        .distribute_reward_to_account(recipient, amount, jwt_token)
        .map_err(|e| {
            error!("Failed to send distribute reward RPC: {}", e);
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("RPC call failed: {}", e),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
    info!("Successfully distributed reward to {}", recipient);
    driver.trigger_tick()?;
    driver.trigger_tick()?;
    Ok(response)
}

/// 触发一次本地 tick（如果已初始化）
pub fn tick_local() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    LocalTickClient::default().trigger_tick()
}

/// 解析转账交易信息（支持 EVM 地址 memo）
///
/// 此函数检查给定的交易是否是SOL转账交易，如果是，则提取发送方、接收方、转账金额和可能的EVM地址。
/// 支持的交易模式：
/// - 包含转账指令和memo指令的转账（memo中包含EVM地址）
///
/// ### 实现说明
/// 本函数使用 `bincode::deserialize` 来安全地解析系统指令，而不是硬编码指令类型数字。
/// 这种方法更加安全和可靠，因为它：
/// - 不依赖于枚举变体的内部数字表示
/// - 能够正确处理未来可能的 SystemInstruction 枚举变化
/// - 使用 Solana 官方的序列化格式进行验证
///
/// ### 参数
/// - `transaction`: 要解析的交易对象
///
/// ### 返回值
/// - `Ok(Some((from, to, amount, evm_address)))`: 成功解析转账交易，返回发送方、接收方、转账金额和EVM地址
/// - `Ok(None)`: 交易不是符合条件的转账交易
/// - `Err(Box<dyn std::error::Error + Send + Sync>)`: 解析过程中发生错误
///
/// ### 示例
/// ```rust
/// if let Ok(Some((from, to, amount, evm_address))) = parse_transfer_transaction(&transaction) {
///     println!("转账: {} -> {}, 金额: {} lamports", from, to, amount);
///     println!("EVM地址: {}", evm_address);
/// }
/// ```
pub fn parse_transfer_transaction(
    transaction: &Transaction,
) -> Result<Option<(Pubkey, Pubkey, u64, String)>, Box<dyn std::error::Error + Send + Sync>> {
    let instructions = &transaction.message.instructions;
    let account_keys = &transaction.message.account_keys;

    // 必须恰好包含2个指令：转账指令 + memo指令
    if instructions.len() != 2 {
        return Ok(None);
    }

    // 第一个指令必须是转账指令
    let transfer_instruction = &instructions[0];
    let memo_instruction = &instructions[1];

    // 验证指令索引
    if transfer_instruction.program_id_index as usize >= account_keys.len() ||
       memo_instruction.program_id_index as usize >= account_keys.len() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid program_id_index in instruction",
        )));
    }

    let transfer_program_id = &account_keys[transfer_instruction.program_id_index as usize];
    let memo_program_id = &account_keys[memo_instruction.program_id_index as usize];

    // 验证第一个指令是系统程序的转账指令
    if *transfer_program_id != system_program::id() {
        return Ok(None);
    }

    // 验证第二个指令是memo程序指令
    if memo_program_id.to_string() != "11111111111111111111111111111112" {
        return Ok(None);
    }

    // 解析转账指令
    let lamports = match bincode::deserialize::<SystemInstruction>(&transfer_instruction.data) {
        Ok(SystemInstruction::Transfer { lamports }) => lamports,
        _ => return Ok(None), // 不是转账指令
    };

    // 验证转账指令的账户索引
    if transfer_instruction.accounts.len() != 2 {
        return Ok(None);
    }

    let from_index = transfer_instruction.accounts[0] as usize;
    let to_index = transfer_instruction.accounts[1] as usize;

    if from_index >= account_keys.len() || to_index >= account_keys.len() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid account index in transfer instruction",
        )));
    }

    let from = account_keys[from_index];
    let to = account_keys[to_index];

    // 从memo指令中提取EVM地址
    let evm_address = match extract_evm_address_from_memo(&memo_instruction.data)? {
        Some(addr) => addr,
        None => return Ok(None), // memo中没有有效的EVM地址
    };

    Ok(Some((from, to, lamports, evm_address)))
}

/// 从memo数据中提取EVM地址
///
/// ### 参数
/// - `memo_data`: memo指令的数据部分
///
/// ### 返回值
/// - `Ok(Some(String))`: 成功提取到EVM地址
/// - `Ok(None)`: memo中没有有效的EVM地址
/// - `Err(...)`: 解析过程中发生错误
fn extract_evm_address_from_memo(memo_data: &[u8]) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    // 将memo数据转换为UTF-8字符串
    let memo_text = match std::str::from_utf8(memo_data) {
        Ok(text) => text.trim(),
        Err(_) => return Ok(None), // 不是有效的UTF-8，跳过
    };

    // 检查是否是有效的EVM地址格式（0x开头的40个十六进制字符）
    if memo_text.len() == 42 && memo_text.starts_with("0x") {
        let hex_part = &memo_text[2..];
        // 验证是否都是十六进制字符
        if hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(Some(memo_text.to_string()));
        }
    }

    // 也支持不带0x前缀的40个十六进制字符
    if memo_text.len() == 40 && memo_text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(Some(format!("0x{}", memo_text)));
    }

    Ok(None)
}

/// 创建包含转账和EVM地址memo的交易
///
/// 此函数用于构建一个包含转账指令和memo指令的交易，memo中包含指定的EVM地址。
/// 这种交易格式专门用于跨链桥接场景。
///
/// ### 参数
/// - `from`: 发送方的密钥对，用于签名交易
/// - `to`: 接收方的公钥
/// - `amount`: 转账金额（lamports）
/// - `evm_address`: 目标EVM地址（支持带或不带0x前缀）
/// - `recent_blockhash`: 最新的区块哈希，用于交易签名
///
/// ### 返回值
/// - `Ok(Transaction)`: 成功创建的已签名交易
/// - `Err(Box<dyn std::error::Error + Send + Sync>)`: 创建过程中发生错误
///
/// ### 示例
/// ```rust
/// let from_keypair = Keypair::new();
/// let to_pubkey = Keypair::new().pubkey();
/// let amount = 1_000_000_000; // 1 SOL
/// let evm_address = "0x742d35Cc6634C0532925a3b8D4C2C4e0C8b83265";
/// let recent_blockhash = rpc_client.get_latest_blockhash()?;
///
/// let transaction = create_transfer_with_evm_memo(
///     &from_keypair,
///     &to_pubkey,
///     amount,
///     evm_address,
///     recent_blockhash,
/// )?;
/// ```
pub fn create_transfer_with_evm_memo(
    from: &Keypair,
    to: &Pubkey,
    amount: u64,
    evm_address: &str,
    recent_blockhash: Hash,
) -> Result<Transaction, Box<dyn std::error::Error + Send + Sync>> {
    use solana_sdk::instruction::Instruction;
    
    // 标准化EVM地址格式（确保有0x前缀）
    let normalized_evm_address = if evm_address.starts_with("0x") {
        evm_address.to_string()
    } else if evm_address.len() == 40 && evm_address.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("0x{}", evm_address)
    } else {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Invalid EVM address format: {}", evm_address),
        )));
    };

    // 创建转账指令
    let transfer_instruction = system_instruction::transfer(
        &from.pubkey(),
        to,
        amount,
    );

    // 创建memo指令（包含EVM地址）
    let memo_program_id = Pubkey::try_from("11111111111111111111111111111112")
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    
    let memo_instruction = Instruction::new_with_bytes(
        memo_program_id,
        normalized_evm_address.as_bytes(),
        vec![], // memo指令不需要账户
    );

    // 创建包含转账和memo的交易
    let mut transaction = Transaction::new_with_payer(
        &[transfer_instruction, memo_instruction],
        Some(&from.pubkey()),
    );

    // 签名交易
    transaction.sign(&[from], recent_blockhash);

    Ok(transaction)
}

#[cfg(test)]
mod tests {
    use solana_sdk::hash::hash;
    use solana_sdk::signature::{Keypair, Signer};
    use solana_sdk::system_instruction;
    use {super::*, crate::bridge::genesis, solana_client::rpc_client::RpcClient};
    use crate::bridge::genesis::keypair_from_seed;

    /// 测试获取创世哈希功能
    ///
    /// 这个测试函数验证 `get_genesis_hash` 函数是否能够正常工作。
    /// 测试连接到本地开发网络（127.0.0.1:8899）并尝试获取创世哈希。
    ///
    /// ### 注意事项
    /// 本地需要手动运行Solana验证器
    #[test]
    fn test_get_genesis_hash() {
        let rpc_url = "http://127.0.0.1:8899";
        let rpc_client = RpcClient::new(rpc_url.to_string());
        let result = get_genesis_hash(&rpc_client);
        assert!(
            result.is_ok(),
            "Failed to get genesis hash: {:?}",
            result.err()
        );
        if let Ok(hash) = result {
            println!("Successfully got genesis hash: {}", hash);
        }
    }

    /// 测试获取区块功能
    ///
    /// 这个测试函数验证 `get_block` 函数是否能够正常工作。
    /// 测试连接到本地开发网络并获取创世区块（槽位0），然后验证
    /// 创世区块的哈希是否与网络的创世哈希一致。
    ///
    /// ### 测试步骤
    /// 1. 获取槽位0的区块信息（创世区块）
    /// 2. 获取网络的创世哈希
    /// 3. 验证两者是否一致
    ///
    /// ### 注意事项
    /// 本地需要手动运行Solana验证器
    #[test]
    fn test_get_block() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let rpc_url = "http://127.0.0.1:8899";
        let rpc_client = RpcClient::new(rpc_url.to_string());
        let block_0 = get_block(&rpc_client, 0)?;
        let genesis_blockhash = get_genesis_hash(&rpc_client)?;
        assert_eq!(block_0.blockhash, genesis_blockhash.to_string());
        Ok(())
    }

    /// 测试一致性
    ///
    /// ### 测试步骤
    /// 1. 固定随机数种子，创建 1000 个交易，用 faucet 给不同的账户转账 1_000_000 lamport。
    ///                                （可以用 genesis.rs 里面的 keypair_from_seed）
    /// 2. 通过 get_slot(&rpc_client)?; 获取最新 slot，是否每次执行都是 2000
    /// 3. 通过 get_block(&rpc_client, slot)? 获取最新区块信息;
    /// 3. 验证区块哈希是否每次执行都一致
    ///
    /// ### 注意事项
    /// 本地需要手动运行Solana验证器 之前忘记push了这个
    #[test]fn test_slot_hash_consistency() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let rpc_url = "http://127.0.0.1:8899";
        let mut rpc_client = RpcClient::new(rpc_url.to_string());
        let faucet_keypair = genesis::faucet_keypair();
        let ipc_client = IpcClient::new("/tmp/solana-private-validator".to_string());
        // TODO
        let nb_transaction = 1000;
        let random_seed = "yzm_test_seed_str";
        let transactions = (0..nb_transaction).into_iter().map(|x| {
            let unique_input = format!("{}-{}", random_seed, x);

            // 2. 对这个唯一输入进行哈希，得到一个 32 字节的哈希值
            //    solana_sdk::hash::hash 返回一个 `Hash` 类型
            let account_seed_hash = hash(unique_input.as_bytes());

            // 3. 将 `Hash` 类型转换为一个 [u8; 32] 字节数组
            let account_seed_bytes = account_seed_hash.to_bytes();
            let account = keypair_from_seed(&account_seed_bytes);
            let transfer_amount = 1_000_000_000;
            let transfer_instruction =
                system_instruction::transfer(&faucet_keypair.pubkey(), &account.pubkey(), transfer_amount);

            let recent_blockhash = match rpc_client.get_latest_blockhash() {
                Ok(blockhash) => blockhash,
                Err(e) => {
                    panic!("Failed to get latest blockhash: {}", e);
                }
            };

            // 创建交易
            let mut transaction =
                Transaction::new_with_payer(&[transfer_instruction], Some(&faucet_keypair.pubkey()));

            // 签名交易
            transaction.sign(&[&faucet_keypair], recent_blockhash);
            transaction
        }).collect::<Vec<_>>();
        rpc_client.set_auth_token_secret("bd1fa71e224227a12439367e525610e7c0d242ecfa595ec471299b535e5d179d".to_string());
        for tx in transactions.iter() {
            let send_result = send_and_confirm_transaction(&ipc_client, &rpc_client, tx,"bd1fa71e224227a12439367e525610e7c0d242ecfa595ec471299b535e5d179d");
            match send_result {
                Ok(signature) => {
                    match rpc_client.get_signature_status_with_commitment(
                        &signature,
                        CommitmentConfig {
                            commitment: CommitmentLevel::Processed,
                        },
                    ) {
                        Ok(Some(Ok(_))) => {

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
                    panic!("{}", format!("Transaction with expired blockhash was correctly rejected,Rejection reason: {}", e));
                }
            }
        }
        let nb_slot = get_slot(&rpc_client).unwrap();
        println!("{}", nb_slot);
        let block = get_block(&rpc_client, nb_slot).unwrap();
        println!("{}", block.blockhash);

        Ok(())
    }

    #[test]
    fn test_distribute_reward() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let rpc_url = "http://127.0.0.1:8899";
        let mut rpc_client =RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::processed());
        let client = IpcClient::new("/tmp/solana-private-validator".to_string());
        let recipient = Keypair::new().pubkey();
        let amount = 1000;
        let test_hex_jwt_secret = "bd1fa71e224227a12439367e525610e7c0d242ecfa595ec471299b535e5d179d";
        rpc_client.set_auth_token_secret(test_hex_jwt_secret.to_string());
        let account_data = distribute_reward_to_account(&rpc_client, &client, &recipient, amount)?;
        if let Some(account_in_response) = account_data {
            println!("{:#?}", account_in_response);
        } else {
            panic!("Failed to distribute reward to account");
        }
        // 如果成功了，再查一下余额
        let account = rpc_client.get_account(&recipient).unwrap();
        assert_eq!(account.lamports, amount);
        Ok(())
    }

    /// 测试解析转账交易功能
    ///
    /// 这个测试验证 `parse_transfer_transaction` 函数能够正确解析普通的SOL转账交易，
    /// 并提取出发送方、接收方和转账金额。
    #[test]
    fn test_parse_transfer_transaction() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 创建测试用的密钥对
        let from_keypair = Keypair::new();
        let to_pubkey = Keypair::new().pubkey();
        let transfer_amount = 1_000_000; // 1 SOL in lamports

        // 创建转账指令
        let transfer_instruction = system_instruction::transfer(
            &from_keypair.pubkey(),
            &to_pubkey,
            transfer_amount,
        );

        // 创建交易
        let mut transaction = Transaction::new_with_payer(
            &[transfer_instruction],
            Some(&from_keypair.pubkey()),
        );

        // 使用一个虚拟的最近区块哈希进行签名
        let recent_blockhash = Hash::default();
        transaction.sign(&[&from_keypair], recent_blockhash);

        // 解析交易
        let result = parse_transfer_transaction(&transaction)?;

        // 验证解析结果 - 现在函数只支持带memo的转账，普通转账应该返回None
        assert!(result.is_none(), "普通转账交易应该返回None");
        println!("✓ 普通转账交易正确返回None");

        Ok(())
    }

    /// 测试解析非转账交易功能
    ///
    /// 这个测试验证 `parse_transfer_transaction` 函数对于非转账交易能够正确返回 None。
    /// 测试使用创建账户指令作为非转账交易的例子。
    #[test]
    fn test_parse_non_transfer_transaction() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 创建测试用的密钥对
        let payer_keypair = Keypair::new();
        let new_account_keypair = Keypair::new();
        
        // 创建一个非转账指令（创建账户指令）
        let create_account_instruction = system_instruction::create_account(
            &payer_keypair.pubkey(),
            &new_account_keypair.pubkey(),
            1_000_000, // 最小租金豁免金额
            0,         // 账户数据大小
            &system_program::id(), // 所有者程序
        );

        // 创建交易
        let mut transaction = Transaction::new_with_payer(
            &[create_account_instruction],
            Some(&payer_keypair.pubkey()),
        );

        // 使用一个虚拟的最近区块哈希进行签名
        let recent_blockhash = Hash::default();
        transaction.sign(&[&payer_keypair, &new_account_keypair], recent_blockhash);

        // 解析交易
        let result = parse_transfer_transaction(&transaction)?;

        // 验证解析结果
        assert!(result.is_none(), "非转账交易应该返回 None");
        
        println!("✓ 非转账交易正确返回 None");

        Ok(())
    }

    /// 测试解析多指令交易功能
    ///
    /// 这个测试验证 `parse_transfer_transaction` 函数对于包含多个指令的交易能够正确返回 None。
    #[test]
    fn test_parse_multi_instruction_transaction() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 创建测试用的密钥对
        let from_keypair = Keypair::new();
        let to_pubkey1 = Keypair::new().pubkey();
        let to_pubkey2 = Keypair::new().pubkey();
        
        // 创建两个转账指令
        let transfer_instruction1 = system_instruction::transfer(
            &from_keypair.pubkey(),
            &to_pubkey1,
            500_000,
        );
        
        let transfer_instruction2 = system_instruction::transfer(
            &from_keypair.pubkey(),
            &to_pubkey2,
            500_000,
        );

        // 创建包含多个指令的交易
        let mut transaction = Transaction::new_with_payer(
            &[transfer_instruction1, transfer_instruction2],
            Some(&from_keypair.pubkey()),
        );

        // 使用一个虚拟的最近区块哈希进行签名
        let recent_blockhash = Hash::default();
        transaction.sign(&[&from_keypair], recent_blockhash);

        // 解析交易
        let result = parse_transfer_transaction(&transaction)?;

        // 验证解析结果
        assert!(result.is_none(), "多指令交易应该返回 None");
        
        println!("✓ 多指令交易正确返回 None");

        Ok(())
    }

    /// 测试解析带有EVM地址memo的转账交易功能
    ///
    /// 这个测试验证 `parse_transfer_transaction` 函数能够正确解析包含memo指令的转账交易，
    /// 并提取出EVM地址。
    #[test]
    fn test_parse_transfer_transaction_with_evm_memo() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 创建测试用的密钥对
        let from_keypair = Keypair::new();
        let to_pubkey = Keypair::new().pubkey();
        let transfer_amount = 2_000_000; // 2 SOL in lamports
        let evm_address = "0x742d35Cc6634C0532925a3b8D4C2C4e0C8b83265";

        // 使用新的辅助函数创建交易
        let recent_blockhash = Hash::default();
        let transaction = create_transfer_with_evm_memo(
            &from_keypair,
            &to_pubkey,
            transfer_amount,
            evm_address,
            recent_blockhash,
        )?;

        // 解析交易
        let result = parse_transfer_transaction(&transaction)?;

        // 验证解析结果
        assert!(result.is_some(), "应该成功解析带memo的转账交易");
        
        if let Some((parsed_from, parsed_to, parsed_amount, parsed_evm_address)) = result {
            assert_eq!(parsed_from, from_keypair.pubkey(), "发送方公钥应该匹配");
            assert_eq!(parsed_to, to_pubkey, "接收方公钥应该匹配");
            assert_eq!(parsed_amount, transfer_amount, "转账金额应该匹配");
            assert_eq!(parsed_evm_address, evm_address, "EVM地址应该匹配");
            
            println!("✓ 成功解析带EVM memo的转账交易:");
            println!("  发送方: {}", parsed_from);
            println!("  接收方: {}", parsed_to);
            println!("  金额: {} lamports", parsed_amount);
            println!("  EVM地址: {}", parsed_evm_address);
        }

        Ok(())
    }

    /// 测试解析带有无效memo的转账交易功能
    ///
    /// 这个测试验证 `parse_transfer_transaction` 函数对于包含无效EVM地址的memo能够正确处理。
    #[test]
    fn test_parse_transfer_transaction_with_invalid_memo() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use solana_sdk::instruction::Instruction;
        
        // 创建测试用的密钥对
        let from_keypair = Keypair::new();
        let to_pubkey = Keypair::new().pubkey();
        let transfer_amount = 1_500_000;
        let invalid_memo = "这不是一个有效的EVM地址";

        // 对于无效memo，我们需要手动构建交易，因为create_transfer_with_evm_memo会验证EVM地址格式
        let transfer_instruction = system_instruction::transfer(
            &from_keypair.pubkey(),
            &to_pubkey,
            transfer_amount,
        );

        // 创建memo指令（包含无效的EVM地址）
        let memo_program_id = Pubkey::try_from("11111111111111111111111111111112").unwrap();
        let memo_instruction = Instruction::new_with_bytes(
            memo_program_id,
            invalid_memo.as_bytes(),
            vec![],
        );

        // 创建包含转账和memo的交易
        let mut transaction = Transaction::new_with_payer(
            &[transfer_instruction, memo_instruction],
            Some(&from_keypair.pubkey()),
        );

        // 使用一个虚拟的最近区块哈希进行签名
        let recent_blockhash = Hash::default();
        transaction.sign(&[&from_keypair], recent_blockhash);

        // 解析交易
        let result = parse_transfer_transaction(&transaction)?;

        // 验证解析结果 - 无效memo应该返回None
        assert!(result.is_none(), "无效memo的转账交易应该返回None");
        println!("✓ 带无效memo的转账交易正确返回None");

        Ok(())
    }

    /// 测试解析带有不带0x前缀EVM地址的转账交易功能
    ///
    /// 这个测试验证函数能够正确处理不带0x前缀的40位十六进制EVM地址。
    #[test]
    fn test_parse_transfer_transaction_with_evm_memo_no_prefix() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 创建测试用的密钥对
        let from_keypair = Keypair::new();
        let to_pubkey = Keypair::new().pubkey();
        let transfer_amount = 3_000_000;
        let evm_address_no_prefix = "742d35Cc6634C0532925a3b8D4C2C4e0C8b83265"; // 不带0x前缀
        let expected_evm_address = "0x742d35Cc6634C0532925a3b8D4C2C4e0C8b83265"; // 期望的带0x前缀

        // 使用新的辅助函数创建交易（会自动添加0x前缀）
        let recent_blockhash = Hash::default();
        let transaction = create_transfer_with_evm_memo(
            &from_keypair,
            &to_pubkey,
            transfer_amount,
            evm_address_no_prefix,
            recent_blockhash,
        )?;

        // 解析交易
        let result = parse_transfer_transaction(&transaction)?;

        // 验证解析结果
        assert!(result.is_some(), "应该成功解析带memo的转账交易");
        
        if let Some((parsed_from, parsed_to, parsed_amount, parsed_evm_address)) = result {
            assert_eq!(parsed_from, from_keypair.pubkey(), "发送方公钥应该匹配");
            assert_eq!(parsed_to, to_pubkey, "接收方公钥应该匹配");
            assert_eq!(parsed_amount, transfer_amount, "转账金额应该匹配");
            assert_eq!(parsed_evm_address, expected_evm_address, "EVM地址应该自动添加0x前缀");
            
            println!("✓ 成功解析带无前缀EVM memo的转账交易:");
            println!("  发送方: {}", parsed_from);
            println!("  接收方: {}", parsed_to);
            println!("  金额: {} lamports", parsed_amount);
            println!("  EVM地址: {}", parsed_evm_address);
        }

        Ok(())
    }

    /// 测试创建包含EVM地址memo的转账交易功能
    ///
    /// 这个测试验证 `create_transfer_with_evm_memo` 函数能够正确创建包含转账和memo指令的交易。
    #[test]
    fn test_create_transfer_with_evm_memo() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 创建测试用的密钥对
        let from_keypair = Keypair::new();
        let to_pubkey = Keypair::new().pubkey();
        let transfer_amount = 5_000_000; // 5 SOL in lamports
        let evm_address = "0x742d35Cc6634C0532925a3b8D4C2C4e0C8b83265";
        let recent_blockhash = Hash::default();

        // 使用辅助函数创建交易
        let transaction = create_transfer_with_evm_memo(
            &from_keypair,
            &to_pubkey,
            transfer_amount,
            evm_address,
            recent_blockhash,
        )?;

        // 验证交易结构
        assert_eq!(transaction.message.instructions.len(), 2, "交易应该包含2个指令");
        
        // 验证第一个指令是转账指令
        let transfer_instruction = &transaction.message.instructions[0];
        let transfer_program_id = &transaction.message.account_keys[transfer_instruction.program_id_index as usize];
        assert_eq!(*transfer_program_id, system_program::id(), "第一个指令应该是系统程序指令");

        // 验证第二个指令是memo指令
        let memo_instruction = &transaction.message.instructions[1];
        let memo_program_id = &transaction.message.account_keys[memo_instruction.program_id_index as usize];
        assert_eq!(memo_program_id.to_string(), "11111111111111111111111111111112", "第二个指令应该是自定义memo程序指令");

        // 验证memo数据包含EVM地址
        let memo_data = std::str::from_utf8(&memo_instruction.data)?;
        assert_eq!(memo_data, evm_address, "memo数据应该包含EVM地址");

        // 验证交易已正确签名
        assert!(!transaction.signatures.is_empty(), "交易应该已签名");
        assert_eq!(transaction.signatures[0], from_keypair.sign_message(&transaction.message.serialize()), "签名应该正确");

        // 验证可以被解析函数正确解析
        let parsed_result = parse_transfer_transaction(&transaction)?;
        assert!(parsed_result.is_some(), "创建的交易应该能被解析函数正确解析");

        if let Some((parsed_from, parsed_to, parsed_amount, parsed_evm_address)) = parsed_result {
            assert_eq!(parsed_from, from_keypair.pubkey(), "解析的发送方应该匹配");
            assert_eq!(parsed_to, to_pubkey, "解析的接收方应该匹配");
            assert_eq!(parsed_amount, transfer_amount, "解析的金额应该匹配");
            assert_eq!(parsed_evm_address, evm_address, "解析的EVM地址应该匹配");
        }

        println!("✓ 成功创建并验证包含EVM memo的转账交易");
        Ok(())
    }

    /// 测试创建包含无前缀EVM地址memo的转账交易功能
    ///
    /// 这个测试验证函数能够自动为无前缀的EVM地址添加0x前缀。
    #[test]
    fn test_create_transfer_with_evm_memo_auto_prefix() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 创建测试用的密钥对
        let from_keypair = Keypair::new();
        let to_pubkey = Keypair::new().pubkey();
        let transfer_amount = 1_000_000;
        let evm_address_no_prefix = "742d35Cc6634C0532925a3b8D4C2C4e0C8b83265"; // 无前缀
        let expected_evm_address = "0x742d35Cc6634C0532925a3b8D4C2C4e0C8b83265"; // 期望的带前缀
        let recent_blockhash = Hash::default();

        // 使用辅助函数创建交易
        let transaction = create_transfer_with_evm_memo(
            &from_keypair,
            &to_pubkey,
            transfer_amount,
            evm_address_no_prefix,
            recent_blockhash,
        )?;

        // 验证memo数据包含带前缀的EVM地址
        let memo_instruction = &transaction.message.instructions[1];
        let memo_data = std::str::from_utf8(&memo_instruction.data)?;
        assert_eq!(memo_data, expected_evm_address, "memo数据应该包含带0x前缀的EVM地址");

        // 验证解析结果
        let parsed_result = parse_transfer_transaction(&transaction)?;
        if let Some((_, _, _, parsed_evm_address)) = parsed_result {
            assert_eq!(parsed_evm_address, expected_evm_address, "解析的EVM地址应该带有0x前缀");
        }

        println!("✓ 成功自动添加0x前缀到EVM地址");
        Ok(())
    }

    /// 测试创建包含无效EVM地址的交易功能
    ///
    /// 这个测试验证函数对无效EVM地址格式的错误处理。
    #[test]
    fn test_create_transfer_with_invalid_evm_address() {
        let from_keypair = Keypair::new();
        let to_pubkey = Keypair::new().pubkey();
        let transfer_amount = 1_000_000;
        let invalid_evm_address = "invalid_address";
        let recent_blockhash = Hash::default();

        // 尝试创建包含无效EVM地址的交易
        let result = create_transfer_with_evm_memo(
            &from_keypair,
            &to_pubkey,
            transfer_amount,
            invalid_evm_address,
            recent_blockhash,
        );

        // 验证应该返回错误
        assert!(result.is_err(), "无效EVM地址应该导致错误");
        
        if let Err(e) = result {
            assert!(e.to_string().contains("Invalid EVM address format"), "错误信息应该指出EVM地址格式无效");
        }

        println!("✓ 正确拒绝无效的EVM地址格式");
    }
}

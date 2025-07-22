use {
    crate::bridge::ipc::IpcClient,
    log::{debug, error, warn},
    solana_client::rpc_client::RpcClient,
    solana_rpc_client_api::config::RpcBlockConfig,
    solana_sdk::{
        commitment_config::{CommitmentConfig, CommitmentLevel},
        hash::Hash,
        signature::Signature,
        transaction::Transaction,
    },
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
/// - `tick_client`: IPC客户端，用于在交易发送前后执行tick操作
/// - `rpc_client`: Solana RPC客户端，用于发送交易和查询状态
/// - `transaction`: 要发送的交易对象
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
) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {
    send_and_confirm_transaction_with_config(
        tick_client,
        rpc_client,
        transaction,
        60,                         // 默认最大重试次数
        Duration::from_millis(100), // 默认轮询间隔 100ms
    )
}

/// 使用自定义重试设置发送并确认交易
///
/// 这是核心的交易发送和确认函数，提供完整的交易生命周期管理。
/// 该函数会执行以下步骤：
/// 1. 发送前执行tick操作
/// 2. 发送交易到网络
/// 3. 发送后执行多次tick操作（3次）
/// 4. 轮询交易状态直到达到processed承诺级别
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
/// - tick操作失败（发送前或发送后）
/// - 交易发送到网络失败
/// - 交易在网络中执行失败
/// - 达到最大重试次数仍未确认（超时）
/// - RPC调用异常
///
/// ### 承诺级别
/// 使用 `CommitmentLevel::Processed` 级别进行确认，这意味着交易已被验证器处理
/// 但可能还未达到最终确认状态。
///
/// ### 注意事项
/// - 函数会在发送后执行3次tick操作，这是为了确保验证器状态同步
/// - 轮询过程中的临时错误不会立即终止，会继续重试
/// - 只有交易执行错误才会立即返回失败
pub fn send_and_confirm_transaction_with_config(
    tick_client: &IpcClient,
    rpc_client: &RpcClient,
    transaction: &Transaction,
    max_retries: u32,
    poll_interval: Duration,
) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {
    // Step 1: Call tick before sending transaction
    tick_client.tick().map_err(|e| {
        error!("Failed to tick before sending transaction: {}", e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Tick failed: {}", e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // Step 2: Send transaction to get signature
    let signature = rpc_client.send_transaction(transaction).map_err(|e| {
        error!("Failed to send transaction: {}", e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Transaction send failed: {}", e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    debug!("Transaction sent with signature: {}", signature);

    // Step 3: Call tick again after sending
    tick_client.tick().map_err(|e| {
        error!("Failed to tick after sending transaction: {}", e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Post-send tick failed: {}", e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    tick_client.tick().map_err(|e| {
        error!("Failed to tick (2nd time) after sending transaction: {}", e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Post-send tick failed: {}", e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    tick_client.tick().map_err(|e| {
        error!("Failed to tick (3rd time) after sending transaction: {}", e);
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Post-send tick failed: {}", e),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // Step 4: Poll until commitment level is processed
    for attempt in 1..=max_retries {
        debug!(
            "Polling transaction status, attempt {}/{}",
            attempt, max_retries
        );

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

#[cfg(test)]
mod tests {
    use {super::*, crate::bridge::genesis, solana_client::rpc_client::RpcClient};

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
    /// 本地需要手动运行Solana验证器
    #[test]
    fn test_slot_hash_consistency() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let rpc_url = "http://127.0.0.1:8899";
        let rpc_client = RpcClient::new(rpc_url.to_string());
        let faucet_keypair = genesis::faucet_keypair();
        // TODO
        Ok(())
    }
}

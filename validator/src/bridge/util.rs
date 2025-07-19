use {
    crate::bridge::ipc::IpcClient,
    log::{debug, error, warn},
    solana_client::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::{CommitmentConfig, CommitmentLevel},
        signature::Signature,
        transaction::Transaction,
    },
    std::time::Duration,
};

/// Send and confirm transaction with default retry settings
/// Default: 60 retries, 100ms poll interval
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

/// Send and confirm transaction with custom retry settings
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

#[cfg(test)]
mod tests {}

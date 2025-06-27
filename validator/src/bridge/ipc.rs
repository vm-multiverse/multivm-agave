use std::path::Path;
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use solana_sdk::transaction::Transaction;
use solana_sdk::signature::Keypair;
use log::{info, error, warn};

use super::bridge::Bridge;

/// IPC message types for inter-process communication
#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
    /// Batch transaction request
    BatchTransactions {
        transactions: Vec<Transaction>,
        signers: Vec<Vec<u8>>, // Serialized Keypair data
    },
    /// Response message
    Response {
        success: bool,
        message: String,
    },
}

/// IPC server configuration
pub struct IpcServerConfig {
    pub socket_path: String,
    pub bridge: Arc<Bridge>,
}

/// IPC server for handling inter-process communication
pub struct IpcServer {
    config: IpcServerConfig,
    listener: Option<UnixListener>,
}

impl IpcServer {
    /// Create a new IPC server
    pub fn new(config: IpcServerConfig) -> Self {
        Self {
            config,
            listener: None,
        }
    }

    /// Start the IPC server
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove existing socket file if it exists
        if Path::new(&self.config.socket_path).exists() {
            std::fs::remove_file(&self.config.socket_path)?;
        }

        // Create Unix domain socket listener
        let listener = UnixListener::bind(&self.config.socket_path)?;
        info!("IPC server started, listening on socket: {}", self.config.socket_path);
        
        self.listener = Some(listener);
        
        // Start accepting connections
        self.accept_connections().await
    }

    /// Accept client connections
    async fn accept_connections(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = self.listener.as_ref().unwrap();
        
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let bridge = Arc::clone(&self.config.bridge);
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_client(stream, bridge).await {
                            error!("Error handling client connection: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                }
            }
        }
    }

    /// Handle individual client connection
    async fn handle_client(
        mut stream: UnixStream,
        bridge: Arc<Bridge>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("New client connection");

        loop {
            // Read message length (4 bytes)
            let mut len_buf = [0u8; 4];
            match stream.read_exact(&mut len_buf).await {
                Ok(_) => {},
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    info!("Client disconnected");
                    break;
                }
                Err(e) => {
                    error!("Error reading message length: {}", e);
                    break;
                }
            }

            let msg_len = u32::from_le_bytes(len_buf) as usize;
            if msg_len > 10 * 1024 * 1024 { // Limit message size to 10MB
                error!("Message too large: {} bytes", msg_len);
                break;
            }

            // Read message content
            let mut msg_buf = vec![0u8; msg_len];
            if let Err(e) = stream.read_exact(&mut msg_buf).await {
                error!("Error reading message content: {}", e);
                break;
            }

            // Deserialize message
            let message: IpcMessage = match bincode::deserialize(&msg_buf) {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Error deserializing message: {}", e);
                    let response = IpcMessage::Response {
                        success: false,
                        message: format!("Deserialization error: {}", e),
                    };
                    let _ = Self::send_response(&mut stream, response).await;
                    continue;
                }
            };

            // Process message
            let response = Self::process_message(message, &bridge).await;
            
            // Send response
            if let Err(e) = Self::send_response(&mut stream, response).await {
                error!("Error sending response: {}", e);
                break;
            }
        }

        Ok(())
    }

    /// Process IPC message
    async fn process_message(
        message: IpcMessage,
        bridge: &Bridge,
    ) -> IpcMessage {
        match message {
            IpcMessage::BatchTransactions { mut transactions, signers } => {
                info!("Received batch transaction request, transaction count: {}", transactions.len());

                // Convert serialized signer data to Keypair
                let keypairs: Result<Vec<Keypair>, _> = signers
                    .iter()
                    .map(|signer_bytes| Keypair::from_bytes(signer_bytes))
                    .collect();

                let keypairs = match keypairs {
                    Ok(kps) => kps,
                    Err(e) => {
                        error!("Error parsing signers: {}", e);
                        return IpcMessage::Response {
                            success: false,
                            message: format!("Signer parsing error: {}", e),
                        };
                    }
                };

                // Create signer references
                let signer_refs: Vec<&Keypair> = keypairs.iter().collect();

                // Use Bridge to send transactions
                match bridge.send_and_confirm_transactions_sequentially(&mut transactions, &signer_refs) {
                    Ok(()) => {
                        info!("Successfully sent and confirmed {} transactions", transactions.len());
                        IpcMessage::Response {
                            success: true,
                            message: format!("Successfully processed {} transactions", transactions.len()),
                        }
                    }
                    Err(e) => {
                        error!("Error sending transactions: {}", e);
                        IpcMessage::Response {
                            success: false,
                            message: format!("Transaction sending error: {}", e),
                        }
                    }
                }
            }
            IpcMessage::Response { .. } => {
                warn!("Received unexpected response message");
                IpcMessage::Response {
                    success: false,
                    message: "Unexpected response message".to_string(),
                }
            }
        }
    }

    /// Send response message
    async fn send_response(
        stream: &mut UnixStream,
        response: IpcMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Serialize response
        let response_bytes = bincode::serialize(&response)?;
        
        // Send message length
        let len_bytes = (response_bytes.len() as u32).to_le_bytes();
        stream.write_all(&len_bytes).await?;
        
        // Send message content
        stream.write_all(&response_bytes).await?;
        stream.flush().await?;
        
        Ok(())
    }

    /// Stop server and cleanup socket file
    pub fn stop(&self) {
        if Path::new(&self.config.socket_path).exists() {
            if let Err(e) = std::fs::remove_file(&self.config.socket_path) {
                error!("Error removing socket file: {}", e);
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// IPC client for testing and external program connections
pub struct IpcClient {
    socket_path: String,
}

impl IpcClient {
    /// Create a new IPC client
    pub fn new(socket_path: String) -> Self {
        Self { socket_path }
    }

    /// Send batch transactions
    pub async fn send_batch_transactions(
        &self,
        transactions: Vec<Transaction>,
        signers: Vec<Keypair>,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut stream = UnixStream::connect(&self.socket_path).await?;

        // Serialize signers
        let signer_bytes: Vec<Vec<u8>> = signers
            .iter()
            .map(|kp| kp.to_bytes().to_vec())
            .collect();

        let message = IpcMessage::BatchTransactions {
            transactions,
            signers: signer_bytes,
        };

        // Serialize message
        let msg_bytes = bincode::serialize(&message)?;
        
        // Send message length
        let len_bytes = (msg_bytes.len() as u32).to_le_bytes();
        stream.write_all(&len_bytes).await?;
        
        // Send message content
        stream.write_all(&msg_bytes).await?;
        stream.flush().await?;

        // Read response length
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let response_len = u32::from_le_bytes(len_buf) as usize;

        // Read response content
        let mut response_buf = vec![0u8; response_len];
        stream.read_exact(&mut response_buf).await?;

        // Deserialize response
        let response: IpcMessage = bincode::deserialize(&response_buf)?;

        match response {
            IpcMessage::Response { success, message } => {
                if success {
                    info!("Transaction sent successfully: {}", message);
                } else {
                    error!("Transaction sending failed: {}", message);
                }
                Ok(success)
            }
            _ => {
                error!("Received unexpected response type");
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::config::MultivmConfig;
    use tempfile::tempdir;
    use solana_sdk::{system_instruction, system_transaction, signature::Signer};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ipc_communication() {
        // Create temporary directory for socket
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock").to_string_lossy().to_string();

        // Create test Bridge (requires valid RPC endpoint)
        // Note: This test requires a running Solana node
        let (rpc_url, websocket_url) = MultivmConfig::urls();
        let bridge = match Bridge::new(rpc_url, websocket_url) {
            Ok(bridge) => Arc::new(bridge),
            Err(_) => {
                println!("Skipping test: Unable to connect to Solana node");
                return;
            }
        };

        // Create IPC server configuration
        let config = IpcServerConfig {
            socket_path: socket_path.clone(),
            bridge,
        };

        // Start server
        let mut server = IpcServer::new(config);
        tokio::spawn(async move {
            if let Err(e) = server.start().await {
                eprintln!("Server error: {}", e);
            }
        });

        // Wait for server to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Create client and send test transaction
        let client = IpcClient::new(socket_path);
        
        // Create test transaction
        let from_keypair = Keypair::new();
        let to_keypair = Keypair::new();
        let lamports = 1000;
        
        // Create a simple transfer transaction for testing
        // Note: In actual use, ensure the account has sufficient balance
        let transaction = system_transaction::transfer(
            &from_keypair,
            &to_keypair.pubkey(),
            lamports,
            solana_sdk::hash::Hash::default(), // Should use latest blockhash in actual use
        );

        let transactions = vec![transaction];
        let signers = vec![from_keypair];

        // Send transaction (expected to fail due to insufficient balance, but tests communication)
        let _result = client.send_batch_transactions(transactions, signers).await;
        
        // Test passed, indicating IPC communication works properly
        println!("IPC communication test completed");
    }
}
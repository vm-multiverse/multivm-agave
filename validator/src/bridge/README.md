# Bridge IPC Module

This module provides inter-process communication (IPC) functionality, allowing external programs to send Solana transactions via Unix domain socket.

## Configuration

The bridge module includes a centralized configuration system via the `config` module:

```rust
use agave_validator::bridge::config::MultivmConfig;

// Get default RPC and WebSocket URLs
let (rpc_url, websocket_url) = MultivmConfig::urls();

// Or access individual URLs
let rpc_url = MultivmConfig::rpc_url();
let websocket_url = MultivmConfig::websocket_url();
```

**⚠️ IMPORTANT WARNING**: The default configuration values are set for internal network tunneling:
- RPC URL: `http://100.68.83.77:8899`
- WebSocket URL: `ws://100.68.83.77:8900`

**You MUST manually change these addresses in `validator/src/bridge/config.rs` before execution** to match your actual Solana node endpoints. These are not public addresses and will not work in most environments.

For local development, you typically want:
- RPC URL: `http://127.0.0.1:8899`
- WebSocket URL: `ws://127.0.0.1:8900`

For testnet/mainnet, use the appropriate public endpoints.

## Features

- **Unix Domain Socket Communication**: High-efficiency inter-process communication using local sockets
- **Batch Transaction Processing**: Support for sending multiple Solana transactions at once
- **Asynchronous Processing**: Tokio-based asynchronous architecture supporting high concurrency
- **Automatic Transaction Confirmation**: Integrated transaction confirmation mechanism ensuring successful execution
- **Error Handling**: Comprehensive error handling and logging

## Architecture Components

### IpcServer
The IPC server is responsible for:
- Creating and managing Unix domain socket
- Receiving transaction batches from clients
- Calling Bridge module to send transactions to Solana network
- Returning execution results to clients

### IpcClient
The IPC client is responsible for:
- Connecting to IPC server
- Sending transaction batches
- Receiving execution results

### IpcMessage
Defines the communication protocol between client and server:
```rust
pub enum IpcMessage {
    BatchTransactions {
        transactions: Vec<Transaction>,
        signers: Vec<Vec<u8>>,
    },
    Response {
        success: bool,
        message: String,
    },
}
```

## Usage

### 1. Starting IPC Server

```rust
use std::sync::Arc;
use agave_validator::bridge::{
    bridge::Bridge,
    ipc::{IpcServer, IpcServerConfig},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Bridge instance using default configuration
    let (rpc_url, websocket_url) = config::MultivmConfig::urls();
    let bridge = Bridge::new(rpc_url, websocket_url)?;

    // Configure IPC server
    let config = IpcServerConfig {
        socket_path: "/tmp/solana_bridge.sock".to_string(),
        bridge: Arc::new(bridge),
    };

    // Start server
    let mut server = IpcServer::new(config);
    server.start().await?;

    Ok(())
}
```

### 2. Using IPC Client to Send Transactions

```rust
use agave_validator::bridge::ipc::IpcClient;
use solana_sdk::{
    signature::Keypair,
    system_instruction,
    transaction::Transaction,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client
    let client = IpcClient::new("/tmp/solana_bridge.sock".to_string());

    // Prepare transaction
    let from_keypair = Keypair::new();
    let to_keypair = Keypair::new();
    let lamports = 1000;

    let instruction = system_instruction::transfer(
        &from_keypair.pubkey(),
        &to_keypair.pubkey(),
        lamports,
    );

    let mut transaction = Transaction::new_with_payer(
        &[instruction], 
        Some(&from_keypair.pubkey())
    );

    // Send transaction
    let success = client.send_batch_transactions(
        vec![transaction],
        vec![from_keypair],
    ).await?;

    if success {
        println!("Transaction sent successfully!");
    } else {
        println!("Transaction sending failed");
    }

    Ok(())
}
```

## Socket Path Configuration

Default socket path is `/tmp/solana_bridge.sock`, you can modify as needed:

```rust
let config = IpcServerConfig {
    socket_path: "/var/run/solana/bridge.sock".to_string(),
    bridge: Arc::new(bridge),
};
```

## Message Protocol

### Message Format
Each message contains:
1. **Length Prefix** (4 bytes): Message body length, little-endian
2. **Message Body**: `IpcMessage` serialized using bincode

### Message Flow
1. Client connects to socket
2. Client sends `BatchTransactions` message
3. Server processes transactions and sends to Solana network
4. Server returns `Response` message
5. Client receives result

## Error Handling

Server handles the following error conditions:
- Socket connection errors
- Message deserialization errors
- Transaction signing errors
- Solana network errors
- Transaction confirmation timeouts

All errors are logged and appropriate error responses are returned to clients.

## Security Considerations

- Socket file permissions should be set appropriately to prevent unauthorized access
- Message size is limited to 10MB to prevent memory exhaustion attacks
- Consider using more secure authentication mechanisms in production environments

## Performance Optimization

- Uses asynchronous I/O for improved concurrency performance
- Batch transaction processing reduces network overhead
- Connection pooling reuse reduces connection establishment costs

## Example Program

See `validator/examples/ipc_example.rs` for a complete usage example.

Run the example:
```bash
cargo run --example ipc_example
```

## Dependencies

Ensure the following dependencies are included in `Cargo.toml`:
- `tokio` - Async runtime
- `serde` - Serialization support
- `bincode` - Binary serialization
- `log` - Logging
- `solana-sdk` - Solana SDK

## Troubleshooting

### Common Issues

1. **Socket file already exists**
   - Server automatically removes existing socket file on startup
   - If insufficient permissions, manually delete or change path

2. **Connection refused**
   - Ensure IPC server is running
   - Check socket path is correct

3. **Transaction sending failed**
   - Check if Solana node is running
   - Confirm account has sufficient balance
   - Verify network connection

4. **Permission errors**
   - Ensure process has permission to access socket file
   - Check filesystem permission settings
use {
    crossbeam_channel::{Receiver, Sender},
    log::{debug, error, info, warn},
    serde::{Deserialize, Serialize},
    std::{
        io::{Read, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::Path,
        thread,
    },
};

/// Private tick message constant
pub const PRIVATE_TICK_MESSAGE: &str = "private_therainisme_tick";

/// IPC message types
#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
    /// Tick message
    Tick { message: String },
    /// Response message
    Response { success: bool, message: String },
}

/// IPC Server struct
pub struct IpcServer {
    socket_path: String,
    tick_sender: Sender<()>,
    tick_done_receiver: Receiver<()>,
    listener: Option<UnixListener>,
}

impl IpcServer {
    /// Create a new IPC server, initialized with tick_sender obtained from unbound()
    pub fn new(
        socket_path: String,
        tick_sender: Sender<()>,
        tick_done_receiver: Receiver<()>,
    ) -> Self {
        Self {
            socket_path,
            tick_sender,
            tick_done_receiver,
            listener: None,
        }
    }

    /// Start the IPC server
    pub fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove existing socket file if it exists
        if Path::new(&self.socket_path).exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        // Create Unix domain socket listener
        let listener = UnixListener::bind(&self.socket_path)?;
        // info!("IPC server started, listening on socket: {}", self.socket_path);

        self.listener = Some(listener);

        // Start accepting connections
        self.accept_connections()
    }

    /// Accept client connections
    fn accept_connections(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = self.listener.as_ref().unwrap();

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let tick_sender = self.tick_sender.clone();
                    let tick_done_receiver = self.tick_done_receiver.clone();
                    thread::spawn(move || {
                        if let Err(e) = Self::handle_client(stream, tick_sender, tick_done_receiver)
                        {
                            error!("Error handling client connection: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Handle individual client connection
    fn handle_client(
        mut stream: UnixStream,
        tick_sender: Sender<()>,
        tick_done_receiver: Receiver<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("New client connection");

        loop {
            // Read message length (4 bytes)
            let mut len_buf = [0u8; 4];
            match stream.read_exact(&mut len_buf) {
                Ok(_) => {}
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
            if msg_len > 1024 * 1024 {
                // Limit message size to 1MB
                error!("Message too large: {} bytes", msg_len);
                break;
            }

            // Read message content
            let mut msg_buf = vec![0u8; msg_len];
            if let Err(e) = stream.read_exact(&mut msg_buf) {
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
                    let _ = Self::send_response(&mut stream, response);
                    continue;
                }
            };

            // Process message
            let response = Self::process_message(message, &tick_sender, &tick_done_receiver);

            // Send response
            if let Err(e) = Self::send_response(&mut stream, response) {
                error!("Error sending response: {}", e);
                break;
            }
        }

        Ok(())
    }

    /// Process IPC message
    fn process_message(
        message: IpcMessage,
        tick_sender: &Sender<()>,
        tick_done_receiver: &Receiver<()>,
    ) -> IpcMessage {
        match message {
            IpcMessage::Tick { message } => {
                info!("Received tick message: {}", message);

                // Check if it's the specific tick message
                if message == PRIVATE_TICK_MESSAGE {
                    info!("Received private_therainisme_tick message, triggering tick");

                    // Send () to tick_sender to trigger tick
                    match tick_sender.send(()) {
                        Ok(_) => {
                            info!("Successfully triggered tick");
                            // Wait for the tick to be done
                            match tick_done_receiver.recv() {
                                Ok(_) => {
                                    info!("Tick processing confirmed");
                                    IpcMessage::Response {
                                        success: true,
                                        message: "Tick triggered and processed successfully"
                                            .to_string(),
                                    }
                                }
                                Err(e) => {
                                    error!("Error waiting for tick done signal: {}", e);
                                    IpcMessage::Response {
                                        success: false,
                                        message: format!("Failed to get tick confirmation: {}", e),
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Error triggering tick: {}", e);
                            IpcMessage::Response {
                                success: false,
                                message: format!("Tick trigger failed: {}", e),
                            }
                        }
                    }
                } else {
                    warn!("Received unknown tick message: {}", message);
                    IpcMessage::Response {
                        success: false,
                        message: "Unknown tick message".to_string(),
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
    fn send_response(
        stream: &mut UnixStream,
        response: IpcMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Serialize response
        let response_bytes = bincode::serialize(&response)?;

        // Send message length
        let len_bytes = (response_bytes.len() as u32).to_le_bytes();
        stream.write_all(&len_bytes)?;

        // Send message content
        stream.write_all(&response_bytes)?;
        stream.flush()?;

        Ok(())
    }

    /// Stop server and cleanup socket file
    pub fn stop(&self) {
        if Path::new(&self.socket_path).exists() {
            if let Err(e) = std::fs::remove_file(&self.socket_path) {
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

/// IPC Client struct
pub struct IpcClient {
    socket_path: String,
}

impl IpcClient {
    /// Create a new IPC client initialized with a path
    pub fn new(socket_path: String) -> Self {
        Self { socket_path }
    }

    /// Send tick message, sends "private_therainisme_tick" message to server
    pub fn tick(&self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut stream = UnixStream::connect(&self.socket_path)?;

        let message = IpcMessage::Tick {
            message: PRIVATE_TICK_MESSAGE.to_string(),
        };

        // Serialize message
        let msg_bytes = bincode::serialize(&message)?;

        // Send message length
        let len_bytes = (msg_bytes.len() as u32).to_le_bytes();
        stream.write_all(&len_bytes)?;

        // Send message content
        stream.write_all(&msg_bytes)?;
        stream.flush()?;

        // Read response length
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let response_len = u32::from_le_bytes(len_buf) as usize;

        // Read response content
        let mut response_buf = vec![0u8; response_len];
        stream.read_exact(&mut response_buf)?;

        // Deserialize response
        let response: IpcMessage = bincode::deserialize(&response_buf)?;

        match response {
            IpcMessage::Response { success, message } => {
                if success {
                    debug!("Tick sent successfully: {}", message);
                } else {
                    error!("Tick sending failed: {}", message);
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
    use {super::*, crossbeam_channel::unbounded, std::time::Duration, tempfile::tempdir};

    #[test]
    fn test_ipc_tick_communication() {
        solana_logger::setup();
        // Create temporary directory for socket
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir
            .path()
            .join("test_tick.sock")
            .to_string_lossy()
            .to_string();

        // Create tick channel
        let (tick_sender, tick_receiver) = unbounded::<()>();
        let (tick_done_sender, tick_done_receiver) = unbounded::<()>();
        tick_done_sender.send(()).unwrap(); // mock tick done

        // Create and start IPC server
        let mut server = IpcServer::new(socket_path.clone(), tick_sender, tick_done_receiver);
        let server_socket_path = socket_path.clone();
        thread::spawn(move || {
            if let Err(e) = server.start() {
                eprintln!("Server error: {}", e);
            }
        });

        // Wait for server to start
        thread::sleep(Duration::from_millis(100));

        // Create client and send tick
        let client = IpcClient::new(socket_path);

        // Send tick message
        let result = client.tick();
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Verify tick was received and our mock service "processed" it
        let tick_received = tick_receiver.recv_timeout(Duration::from_millis(100));
        assert!(tick_received.is_ok());
        tick_done_sender.send(()).unwrap();

        println!("IPC tick communication test completed");
    }

    #[test]
    fn test_tick_ipc() {
        let client = IpcClient::new("/tmp/solana-private-validator".to_string());
        let result = client.tick();
        assert!(result.is_ok());
        assert!(result.unwrap());

        // loop {
        //     let result = client.tick();
        //     assert!(result.is_ok());
        //     assert!(result.unwrap());
        // }
    }
}

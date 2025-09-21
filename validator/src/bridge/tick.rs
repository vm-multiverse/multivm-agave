use {
    crate::admin_rpc_service::ManualTickChannels,
    crossbeam_channel::{Receiver, Sender},
    lazy_static::lazy_static,
    std::sync::RwLock,
};

/// Abstraction for driving a manual PoH tick
pub trait TickDriver {
    fn trigger_tick(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Global holder for locally-wired tick channels
lazy_static! {
    static ref LOCAL_TICK_CHANNELS: RwLock<Option<ManualTickChannels>> = RwLock::new(None);
}

/// Install local tick channels so that LocalTickClient can drive ticks in-process
pub fn set_local_tick_channels(channels: ManualTickChannels) {
    *LOCAL_TICK_CHANNELS.write().unwrap() = Some(channels);
}

/// A TickDriver that uses in-process channels (no IPC/RPC)
#[derive(Clone, Default)]
pub struct LocalTickClient;

impl LocalTickClient {
    fn get_channels() -> Result<(Sender<()>, Receiver<()>), Box<dyn std::error::Error + Send + Sync>> {
        let guard = LOCAL_TICK_CHANNELS.read().unwrap();
        let Some(channels) = guard.as_ref() else {
            return Err("Local tick channels not initialized".into());
        };
        Ok((channels.tick_sender.clone(), channels.tick_done_receiver.clone()))
    }
}

impl TickDriver for LocalTickClient {
    fn trigger_tick(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (tx, rx) = Self::get_channels()?;
        tx.send(())?;
        // Wait for completion confirmation
        rx.recv()?;
        Ok(())
    }
}

// Bridge existing IPC client into the new abstraction without changing callers
impl TickDriver for crate::bridge::ipc::IpcClient {
    fn trigger_tick(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match crate::bridge::ipc::IpcClient::tick(self) {
            Ok(true) => Ok(()),
            Ok(false) => Err("Tick response indicated failure".into()),
            Err(e) => Err(e),
        }
    }
}


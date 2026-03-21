use std::{collections::HashMap, net::IpAddr, sync::Arc};
use tokio::sync::{broadcast, Mutex};

use crate::{config::Config, protocol::DeviceStatus};

/// A discovered device on the network.
#[derive(Debug, serde::Serialize)]
pub struct Device {
    pub id: String,
    pub ip: IpAddr,
    pub nickname: Option<String>,
    pub last_status: Option<DeviceStatus>,
}

/// Shared registry of all discovered devices.
pub type Registry = Arc<Mutex<HashMap<String, Device>>>;

/// Ensures only one UDP operation binds to port 4000 at a time.
pub type UdpLock = Arc<Mutex<()>>;

/// Persisted nicknames keyed by device ID.
pub type Nicknames = Arc<Mutex<HashMap<String, String>>>;

/// Shared Axum state threaded through all handlers and background tasks.
#[derive(Clone)]
pub struct AppState {
    pub registry: Registry,
    pub udp_lock: UdpLock,
    pub nicknames: Nicknames,
    pub config: Config,
    /// Broadcasts serialised Device JSON to all active SSE connections after each update.
    pub event_tx: broadcast::Sender<String>,
}

pub fn new_app_state(config: Config, nicknames: HashMap<String, String>) -> AppState {
    let (event_tx, _) = broadcast::channel(64);
    AppState {
        registry: Arc::new(Mutex::new(HashMap::new())),
        udp_lock: Arc::new(Mutex::new(())),
        nicknames: Arc::new(Mutex::new(nicknames)),
        config,
        event_tx,
    }
}

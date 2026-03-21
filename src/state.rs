use std::{collections::HashMap, net::IpAddr, sync::Arc};
use tokio::sync::Mutex;

use crate::protocol::DeviceStatus;

/// A discovered device on the network.
#[derive(Debug, serde::Serialize)]
pub struct Device {
    pub id: String,
    pub ip: IpAddr,
    pub last_status: Option<DeviceStatus>,
}

/// Shared registry of all discovered devices.
pub type Registry = Arc<Mutex<HashMap<String, Device>>>;

/// Ensures only one UDP operation binds to port 4000 at a time.
pub type UdpLock = Arc<Mutex<()>>;

/// Shared Axum state threaded through all handlers and background tasks.
#[derive(Clone)]
pub struct AppState {
    pub registry: Registry,
    pub udp_lock: UdpLock,
}

pub fn new_app_state() -> AppState {
    AppState {
        registry: Arc::new(Mutex::new(HashMap::new())),
        udp_lock: Arc::new(Mutex::new(())),
    }
}

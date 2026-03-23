use std::{collections::HashMap, net::IpAddr, sync::Arc};
use tokio::sync::{broadcast, Mutex};

use crate::{config::Config, persist::DeviceSettings, protocol::DeviceStatus};

/// A discovered device on the network.
#[derive(Debug, serde::Serialize)]
pub struct Device {
    pub id: String,
    pub ip: IpAddr,
    pub nickname: Option<String>,
    pub unreachable: bool,
    #[serde(skip)]
    pub consecutive_failures: u32,
    pub last_status: Option<DeviceStatus>,
    pub automation_enabled: bool,
    /// When set, automation will not go below this speed (1–3).
    pub automation_min_speed: Option<u8>,
    /// When set, automation will not exceed this speed (1–3).
    pub automation_max_speed: Option<u8>,
    /// Per-device assumed indoor temperature (°C) for absolute humidity calculation.
    pub assumed_indoor_temp_c: Option<f64>,
}

/// Shared registry of all discovered devices.
pub type Registry = Arc<Mutex<HashMap<String, Device>>>;

/// Ensures only one UDP operation binds to port 4000 at a time.
pub type UdpLock = Arc<Mutex<()>>;

/// Persisted per-device settings keyed by device ID.
pub type Settings = Arc<Mutex<HashMap<String, DeviceSettings>>>;

/// Shared Axum state threaded through all handlers and background tasks.
#[derive(Clone)]
pub struct AppState {
    pub registry: Registry,
    pub udp_lock: UdpLock,
    pub settings: Settings,
    pub config: Config,
    /// Broadcasts serialised Device JSON to all active SSE connections after each update.
    pub event_tx: broadcast::Sender<String>,
}

pub fn new_app_state(config: Config, device_settings: HashMap<String, DeviceSettings>) -> AppState {
    let (event_tx, _) = broadcast::channel(64);
    AppState {
        registry: Arc::new(Mutex::new(HashMap::new())),
        udp_lock: Arc::new(Mutex::new(())),
        settings: Arc::new(Mutex::new(device_settings)),
        config,
        event_tx,
    }
}

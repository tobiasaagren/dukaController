use std::collections::HashMap;

/// All persisted settings for a single device.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DeviceSettings {
    pub nickname: Option<String>,
    #[serde(default)]
    pub automation_enabled: bool,
    pub automation_min_speed: Option<u8>,
    pub automation_max_speed: Option<u8>,
    /// Per-device assumed indoor temperature (°C) for absolute humidity calculation.
    /// Overrides the global `assumed_indoor_temp_c` from config.toml.
    pub assumed_indoor_temp_c: Option<f64>,
}

/// Load persisted device settings from disk. Returns an empty map if the file doesn't exist yet.
pub fn load_settings(path: &str) -> HashMap<String, DeviceSettings> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save device settings to disk, overwriting the existing file.
pub fn save_settings(settings: &HashMap<String, DeviceSettings>, path: &str) {
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        if let Err(e) = std::fs::write(path, json) {
            eprintln!("Failed to save settings: {e}");
        }
    }
}

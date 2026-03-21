use serde::Deserialize;

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub broadcast_address: String,
    pub duka_port: u16,
    pub device_password: String,
    pub discovery_interval_secs: u64,
    pub status_interval_secs: u64,
    pub nicknames_file: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            broadcast_address: "192.168.1.255".to_string(),
            duka_port: 4000,
            device_password: "1111".to_string(),
            discovery_interval_secs: 30,
            status_interval_secs: 3,
            nicknames_file: "nicknames.json".to_string(),
        }
    }
}

/// Load config from `config.toml` next to the binary, falling back to defaults.
pub fn load_config() -> Config {
    match std::fs::read_to_string(CONFIG_FILE) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
            eprintln!("Warning: failed to parse {CONFIG_FILE}: {e} — using defaults");
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

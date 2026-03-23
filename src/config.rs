use serde::Deserialize;

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AutomationConfig {
    pub latitude: f64,
    pub longitude: f64,
    /// How often to fetch outdoor humidity from the weather API (seconds).
    pub outdoor_fetch_interval_secs: u64,
    /// Assumed indoor temperature (°C) used to convert indoor RH% to absolute humidity.
    /// Devices only report RH, so a fixed assumption is needed. Typical homes: 18–22°C.
    pub assumed_indoor_temp_c: f64,
    /// Indoor-minus-outdoor absolute humidity delta (g/m³) at which speed 1 is selected.
    pub speed1_abs_humidity_delta: f64,
    /// Indoor-minus-outdoor absolute humidity delta (g/m³) at which speed 2 is selected.
    pub speed2_abs_humidity_delta: f64,
    /// Indoor-minus-outdoor absolute humidity delta (g/m³) at which speed 3 is selected.
    pub speed3_abs_humidity_delta: f64,
    /// How long (seconds) to wait after a speed increase before allowing a decrease.
    pub speed_decrease_lockout_secs: u64,
}

impl Default for AutomationConfig {
    fn default() -> Self {
        Self {
            latitude: 0.0,
            longitude: 0.0,
            outdoor_fetch_interval_secs: 3600,
            assumed_indoor_temp_c: 20.0,
            speed1_abs_humidity_delta: 0.0,
            speed2_abs_humidity_delta: 1.5,
            speed3_abs_humidity_delta: 3.0,
            speed_decrease_lockout_secs: 3600,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub broadcast_address: String,
    pub duka_port: u16,
    pub device_password: String,
    pub discovery_interval_secs: u64,
    pub status_interval_secs: u64,
    pub settings_file: String,
    pub automation: AutomationConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            broadcast_address: "192.168.1.255".to_string(),
            duka_port: 4000,
            device_password: "1111".to_string(),
            discovery_interval_secs: 30,
            status_interval_secs: 3,
            settings_file: "settings.json".to_string(),
            automation: AutomationConfig::default(),
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

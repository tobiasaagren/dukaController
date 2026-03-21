use std::collections::HashMap;

const NICKNAMES_FILE: &str = "nicknames.json";

/// Load persisted nicknames from disk. Returns an empty map if the file doesn't exist yet.
pub fn load_nicknames() -> HashMap<String, String> {
    match std::fs::read_to_string(NICKNAMES_FILE) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save nicknames to disk, overwriting the existing file.
pub fn save_nicknames(nicknames: &HashMap<String, String>) {
    if let Ok(json) = serde_json::to_string_pretty(nicknames) {
        if let Err(e) = std::fs::write(NICKNAMES_FILE, json) {
            eprintln!("Failed to save nicknames: {e}");
        }
    }
}

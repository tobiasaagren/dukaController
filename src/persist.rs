use std::collections::HashMap;

/// Load persisted nicknames from disk. Returns an empty map if the file doesn't exist yet.
pub fn load_nicknames(path: &str) -> HashMap<String, String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save nicknames to disk, overwriting the existing file.
pub fn save_nicknames(nicknames: &HashMap<String, String>, path: &str) {
    if let Ok(json) = serde_json::to_string_pretty(nicknames) {
        if let Err(e) = std::fs::write(path, json) {
            eprintln!("Failed to save nicknames: {e}");
        }
    }
}

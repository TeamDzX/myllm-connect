//! Small persisted preferences (issue #8). Login auto-start is not stored here:
//! the OS owns that (LaunchAgent / registry Run key), so we ask it directly.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// The user turned sharing on and hasn't stopped it — resume on launch.
    pub sharing: bool,
    /// Hold off idle sleep while sharing, so the phone can still reach us.
    pub keep_awake: bool,
}

fn path() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("MyLLM Connect").join("settings.json")
}

/// Load saved settings; a missing or unreadable file means defaults.
pub fn load() -> Settings {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) {
    let path = path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    if let Ok(json) = serde_json::to_vec_pretty(s) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_default() {
        let s: Settings = serde_json::from_str(r#"{"sharing":true}"#).unwrap();
        assert!(s.sharing);
        assert!(!s.keep_awake);
    }
}

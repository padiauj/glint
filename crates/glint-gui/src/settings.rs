//! Application settings persistence.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application settings
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// Volumes to index
    pub indexed_volumes: Vec<char>,
    /// Maximum search results to display
    pub max_results: usize,
    /// Enable real-time monitoring service
    pub service_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            indexed_volumes: Vec::new(),
            max_results: 100,
            service_enabled: true,
        }
    }
}

impl Settings {
    /// Load settings from disk.
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::settings_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let settings: Settings = serde_json::from_str(&content)?;
            Ok(settings)
        } else {
            Ok(Self::default())
        }
    }
    
    /// Save settings to disk.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::settings_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
    
    /// Get the settings file path.
    fn settings_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dirs = directories::ProjectDirs::from("org", "glint", "glint")
            .ok_or("Could not determine config directory")?;
        Ok(dirs.config_dir().join("settings.json"))
    }
}

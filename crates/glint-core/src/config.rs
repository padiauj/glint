//! Configuration management for Glint.
//!
//! This module provides configuration loading, saving, and defaults.
//! Configuration is stored in TOML format in a platform-appropriate location.

use crate::error::{GlintError, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Main configuration structure for Glint.
///
/// ## Example Configuration File (glint.toml)
///
/// ```toml
/// [general]
/// auto_start_usn = true
/// max_results = 1000
///
/// [exclude]
/// paths = ["C:\\Windows\\Temp", "C:\\$Recycle.Bin"]
/// patterns = ["*.tmp", "~$*"]
///
/// [performance]
/// max_memory_mb = 512
/// parallel_search = true
///
/// [ui]
/// show_hidden = false
/// show_system = false
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// General settings
    pub general: GeneralConfig,

    /// Path and pattern exclusions
    pub exclude: ExcludeConfig,

    /// Performance tuning
    pub performance: PerformanceConfig,

    /// UI settings
    pub ui: UiConfig,

    /// Volumes to index (empty = all NTFS volumes)
    pub volumes: VolumesConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            general: GeneralConfig::default(),
            exclude: ExcludeConfig::default(),
            performance: PerformanceConfig::default(),
            ui: UiConfig::default(),
            volumes: VolumesConfig::default(),
        }
    }
}

/// General configuration options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Automatically start USN monitoring on startup
    pub auto_start_usn: bool,

    /// Maximum number of search results to return
    pub max_results: usize,

    /// Index file location (None = default location)
    pub index_path: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    pub log_level: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        GeneralConfig {
            auto_start_usn: true,
            max_results: 10000,
            index_path: None,
            log_level: "info".to_string(),
        }
    }
}

/// Exclusion configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ExcludeConfig {
    /// Paths to exclude from indexing
    pub paths: Vec<String>,

    /// Glob patterns to exclude
    pub patterns: Vec<String>,

    /// Exclude hidden files and directories
    pub hidden: bool,

    /// Exclude system files and directories
    pub system: bool,
}

/// Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PerformanceConfig {
    /// Maximum memory usage hint in MB (0 = no limit)
    pub max_memory_mb: u64,

    /// Use parallel search for large indices
    pub parallel_search: bool,

    /// Threshold for switching to parallel search
    pub parallel_threshold: usize,

    /// Use compression for index storage
    pub compress_index: bool,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        PerformanceConfig {
            max_memory_mb: 0,
            parallel_search: true,
            parallel_threshold: 10000,
            compress_index: true,
        }
    }
}

/// UI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Show hidden files in results
    pub show_hidden: bool,

    /// Show system files in results
    pub show_system: bool,

    /// Number of results to display per page
    pub page_size: usize,

    /// Highlight matches in results
    pub highlight_matches: bool,

    /// Show file sizes
    pub show_size: bool,

    /// Show modification times
    pub show_modified: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            show_hidden: true,
            show_system: true,
            page_size: 100,
            highlight_matches: true,
            show_size: true,
            show_modified: true,
        }
    }
}

/// Volume selection configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VolumesConfig {
    /// Specific volumes to index (empty = all NTFS volumes)
    pub include: Vec<String>,

    /// Volumes to exclude
    pub exclude: Vec<String>,
}

impl Config {
    /// Load configuration from the default location.
    ///
    /// Returns default config if no config file exists.
    pub fn load() -> Result<Self> {
        let config_path = Self::default_config_path()?;
        Self::load_from(&config_path)
    }

    /// Load configuration from a specific path.
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            debug!(path = %path.display(), "Config file not found, using defaults");
            return Ok(Config::default());
        }

        info!(path = %path.display(), "Loading configuration");
        let contents = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents).map_err(|e| GlintError::ConfigError {
            reason: format!("Failed to parse config: {}", e),
        })?;

        Ok(config)
    }

    /// Save configuration to the default location.
    pub fn save(&self) -> Result<()> {
        let config_path = Self::default_config_path()?;
        self.save_to(&config_path)
    }

    /// Save configuration to a specific path.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        info!(path = %path.display(), "Saving configuration");
        let contents = toml::to_string_pretty(self).map_err(|e| GlintError::ConfigError {
            reason: format!("Failed to serialize config: {}", e),
        })?;

        fs::write(path, contents)?;
        Ok(())
    }

    /// Get the default configuration file path.
    pub fn default_config_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "glint").ok_or_else(|| GlintError::ConfigError {
            reason: "Could not determine config directory".to_string(),
        })?;

        Ok(dirs.config_dir().join("glint.toml"))
    }

    /// Get the default data directory path.
    pub fn default_data_dir() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "glint").ok_or_else(|| GlintError::ConfigError {
            reason: "Could not determine data directory".to_string(),
        })?;

        Ok(dirs.data_dir().to_path_buf())
    }

    /// Get the index directory (from config or default).
    pub fn index_dir(&self) -> Result<PathBuf> {
        if let Some(ref path) = self.general.index_path {
            Ok(path.clone())
        } else {
            Self::default_data_dir()
        }
    }

    /// Check if a path should be excluded.
    pub fn should_exclude_path(&self, path: &str) -> bool {
        let path_lower = path.to_lowercase();

        // Check exact path exclusions
        for excluded in &self.exclude.paths {
            if path_lower.starts_with(&excluded.to_lowercase()) {
                return true;
            }
        }

        false
    }

    /// Check if a filename should be excluded based on patterns.
    pub fn should_exclude_name(&self, name: &str) -> bool {
        for pattern in &self.exclude.patterns {
            if matches_simple_pattern(name, pattern) {
                return true;
            }
        }
        false
    }

    /// Check if a volume should be indexed.
    pub fn should_index_volume(&self, mount_point: &str) -> bool {
        // If explicit includes are specified, check them
        if !self.volumes.include.is_empty() {
            return self
                .volumes
                .include
                .iter()
                .any(|v| mount_point.eq_ignore_ascii_case(v));
        }

        // Check excludes
        if self
            .volumes
            .exclude
            .iter()
            .any(|v| mount_point.eq_ignore_ascii_case(v))
        {
            return false;
        }

        true
    }
}

/// Simple pattern matching for exclusion patterns.
///
/// Supports:
/// - `*` at start or end (e.g., `*.tmp`, `~*`)
/// - Exact match otherwise
fn matches_simple_pattern(name: &str, pattern: &str) -> bool {
    let name_lower = name.to_lowercase();
    let pattern_lower = pattern.to_lowercase();

    if pattern_lower.starts_with('*') && pattern_lower.ends_with('*') && pattern.len() > 2 {
        // Contains pattern
        let middle = &pattern_lower[1..pattern_lower.len() - 1];
        name_lower.contains(middle)
    } else if let Some(suffix) = pattern_lower.strip_prefix('*') {
        // Ends with pattern
        name_lower.ends_with(suffix)
    } else if let Some(prefix) = pattern_lower.strip_suffix('*') {
        // Starts with pattern
        name_lower.starts_with(prefix)
    } else {
        // Exact match
        name_lower == pattern_lower
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.general.auto_start_usn);
        assert_eq!(config.general.max_results, 10000);
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test.toml");

        let mut config = Config::default();
        config.general.max_results = 5000;
        config.exclude.paths = vec!["C:\\Temp".to_string()];

        config.save_to(&config_path).unwrap();
        let loaded = Config::load_from(&config_path).unwrap();

        assert_eq!(loaded.general.max_results, 5000);
        assert_eq!(loaded.exclude.paths, vec!["C:\\Temp".to_string()]);
    }

    #[test]
    fn test_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("nonexistent.toml");

        let config = Config::load_from(&config_path).unwrap();
        assert_eq!(config.general.max_results, 10000); // Default value
    }

    #[test]
    fn test_should_exclude_path() {
        let mut config = Config::default();
        config.exclude.paths = vec![
            "C:\\Windows\\Temp".to_string(),
            "C:\\$Recycle.Bin".to_string(),
        ];

        assert!(config.should_exclude_path("C:\\Windows\\Temp\\file.txt"));
        assert!(config.should_exclude_path("c:\\windows\\temp\\subdir"));
        assert!(!config.should_exclude_path("C:\\Users\\file.txt"));
    }

    #[test]
    fn test_should_exclude_name() {
        let mut config = Config::default();
        config.exclude.patterns = vec!["*.tmp".to_string(), "~$*".to_string()];

        assert!(config.should_exclude_name("document.tmp"));
        assert!(config.should_exclude_name("~$document.docx"));
        assert!(!config.should_exclude_name("document.txt"));
    }

    #[test]
    fn test_simple_pattern() {
        assert!(matches_simple_pattern("file.tmp", "*.tmp"));
        assert!(matches_simple_pattern("FILE.TMP", "*.tmp"));
        assert!(!matches_simple_pattern("file.txt", "*.tmp"));

        assert!(matches_simple_pattern("~$doc.docx", "~$*"));
        assert!(!matches_simple_pattern("doc.docx", "~$*"));

        assert!(matches_simple_pattern("readme.md", "readme.md"));
        assert!(matches_simple_pattern("README.MD", "readme.md"));
    }

    #[test]
    fn test_should_index_volume() {
        let mut config = Config::default();

        // With no includes/excludes, all volumes should be indexed
        assert!(config.should_index_volume("C:"));
        assert!(config.should_index_volume("D:"));

        // With explicit includes
        config.volumes.include = vec!["C:".to_string()];
        assert!(config.should_index_volume("C:"));
        assert!(!config.should_index_volume("D:"));

        // With excludes only
        config.volumes.include.clear();
        config.volumes.exclude = vec!["D:".to_string()];
        assert!(config.should_index_volume("C:"));
        assert!(!config.should_index_volume("D:"));
    }
}

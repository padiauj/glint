//! Error types for Glint core operations.
//!
//! This module defines well-structured error types using `thiserror` for
//! library-level errors, while higher-level code can use `anyhow` for
//! convenient error handling.

use std::path::PathBuf;
use thiserror::Error;

/// Result type alias using GlintError
pub type Result<T> = std::result::Result<T, GlintError>;

/// Core error types for Glint operations.
///
/// These errors represent specific failure modes that callers may want to
/// handle differently (e.g., triggering a rescan on journal truncation).
#[derive(Error, Debug)]
pub enum GlintError {
    // === Index Errors ===
    /// The index file is missing or could not be found
    #[error("index not found at {path}")]
    IndexNotFound { path: PathBuf },

    /// The index file exists but is corrupted or unreadable
    #[error("index is corrupted: {reason}")]
    IndexCorrupted { reason: String },

    /// The index format version doesn't match the current version
    #[error("index version mismatch: found {found}, expected {expected}")]
    IndexVersionMismatch { found: u32, expected: u32 },

    /// The index is stale and needs to be rebuilt
    #[error("index is stale for volume {volume}: {reason}")]
    IndexStale { volume: String, reason: String },

    // === Filesystem Backend Errors ===
    /// Volume not found or inaccessible
    #[error("volume not found: {volume}")]
    VolumeNotFound { volume: String },

    /// Permission denied when accessing filesystem
    #[error("permission denied: {operation} on {path}")]
    PermissionDenied { operation: String, path: String },

    /// The USN Change Journal is unavailable or disabled
    #[error("USN journal unavailable for volume {volume}: {reason}")]
    UsnJournalUnavailable { volume: String, reason: String },

    /// The USN Change Journal has been truncated, requiring a rescan
    #[error("USN journal truncated for volume {volume}, rescan required")]
    UsnJournalTruncated { volume: String },

    /// The USN journal ID has changed (journal was deleted and recreated)
    #[error("USN journal ID changed for volume {volume}, rescan required")]
    UsnJournalIdChanged { volume: String },

    /// Generic filesystem operation failure
    #[error("filesystem error: {operation} failed: {reason}")]
    FilesystemError { operation: String, reason: String },

    // === Search Errors ===
    /// Invalid search pattern (e.g., bad regex)
    #[error("invalid search pattern: {pattern}: {reason}")]
    InvalidPattern { pattern: String, reason: String },

    // === Configuration Errors ===
    /// Configuration file parsing failed
    #[error("configuration error: {reason}")]
    ConfigError { reason: String },

    // === I/O Errors ===
    /// Generic I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // === Serialization Errors ===
    /// Serialization/deserialization failed
    #[error("serialization error: {0}")]
    Serialization(String),

    // === Internal Errors ===
    /// Internal error that should not happen
    #[error("internal error: {0}")]
    Internal(String),
}

impl GlintError {
    /// Returns true if this error indicates the index needs to be rebuilt
    pub fn requires_rescan(&self) -> bool {
        matches!(
            self,
            GlintError::IndexNotFound { .. }
                | GlintError::IndexCorrupted { .. }
                | GlintError::IndexVersionMismatch { .. }
                | GlintError::IndexStale { .. }
                | GlintError::UsnJournalTruncated { .. }
                | GlintError::UsnJournalIdChanged { .. }
        )
    }

    /// Returns true if this error is recoverable (e.g., can retry)
    pub fn is_recoverable(&self) -> bool {
        matches!(self, GlintError::Io(_))
    }

    /// Create a filesystem error
    pub fn filesystem(operation: impl Into<String>, reason: impl Into<String>) -> Self {
        GlintError::FilesystemError {
            operation: operation.into(),
            reason: reason.into(),
        }
    }

    /// Create a serialization error
    pub fn serialization(reason: impl Into<String>) -> Self {
        GlintError::Serialization(reason.into())
    }
}

impl From<bincode::Error> for GlintError {
    fn from(err: bincode::Error) -> Self {
        GlintError::Serialization(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requires_rescan() {
        let err = GlintError::IndexNotFound {
            path: PathBuf::from("/test"),
        };
        assert!(err.requires_rescan());

        let err = GlintError::UsnJournalTruncated {
            volume: "C:".to_string(),
        };
        assert!(err.requires_rescan());

        let err = GlintError::InvalidPattern {
            pattern: "[".to_string(),
            reason: "unclosed bracket".to_string(),
        };
        assert!(!err.requires_rescan());
    }
}

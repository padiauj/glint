//! Core data types for Glint.
//!
//! This module defines the fundamental data structures used throughout the
//! indexing and search system. These types are designed to be:
//!
//! - **Serializable**: For persistence to disk
//! - **Platform-agnostic**: No OS-specific details leak into these types
//! - **Efficient**: Optimized for both memory usage and search performance

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::Hash;

/// Unique identifier for a file within a volume.
///
/// On NTFS, this corresponds to the MFT record number. On other filesystems,
/// it might be an inode number or other unique identifier.
///
/// The identifier is volume-scoped: two files on different volumes may have
/// the same `FileId`, but `(VolumeId, FileId)` is globally unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub u64);

impl FileId {
    /// The root directory file ID (typically 5 on NTFS for the root MFT entry)
    pub const ROOT: FileId = FileId(5);

    /// Create a new file ID
    pub fn new(id: u64) -> Self {
        FileId(id)
    }

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a volume.
///
/// On Windows, this is typically derived from the volume serial number.
/// The string representation allows for cross-platform compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VolumeId(pub String);

impl VolumeId {
    /// Create a new volume ID from a string
    pub fn new(id: impl Into<String>) -> Self {
        VolumeId(id.into())
    }

    /// Get the volume ID as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VolumeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for VolumeId {
    fn from(s: String) -> Self {
        VolumeId(s)
    }
}

impl From<&str> for VolumeId {
    fn from(s: &str) -> Self {
        VolumeId(s.to_string())
    }
}

/// A record representing a single file or directory in the index.
///
/// This is the core data structure stored in the index. It contains all
/// information needed for searching and displaying results.
///
/// ## Design Notes
///
/// - `name` is stored separately from `path` for efficient filename-only searches
/// - `name_lower` is pre-computed for fast case-insensitive matching
/// - `path` is the full path including the filename
/// - Parent-child relationships are tracked via `parent_id` for path reconstruction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    /// Unique identifier within the volume
    pub id: FileId,

    /// Parent directory's file ID (None for root directories)
    pub parent_id: Option<FileId>,

    /// Volume this file belongs to
    pub volume_id: VolumeId,

    /// Filename without path (e.g., "document.txt")
    pub name: String,

    /// Pre-computed lowercase filename for fast case-insensitive search
    #[serde(skip)]
    pub name_lower: String,

    /// Full path including filename (e.g., "C:\Users\doc\document.txt")
    pub path: String,

    /// True if this is a directory, false for files
    pub is_dir: bool,

    /// File size in bytes (None for directories or if unavailable)
    pub size: Option<u64>,

    /// Last modification time
    pub modified: Option<DateTime<Utc>>,

    /// Creation time (if available)
    pub created: Option<DateTime<Utc>>,
}

impl FileRecord {
    /// Create a new file record with the given parameters.
    ///
    /// The `name_lower` field is automatically computed from `name`.
    pub fn new(
        id: FileId,
        parent_id: Option<FileId>,
        volume_id: VolumeId,
        name: String,
        path: String,
        is_dir: bool,
    ) -> Self {
        let name_lower = name.to_lowercase();
        FileRecord {
            id,
            parent_id,
            volume_id,
            name,
            name_lower,
            path,
            is_dir,
            size: None,
            modified: None,
            created: None,
        }
    }

    /// Set the file size
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Set the modification time
    pub fn with_modified(mut self, modified: DateTime<Utc>) -> Self {
        self.modified = Some(modified);
        self
    }

    /// Set the creation time
    pub fn with_created(mut self, created: DateTime<Utc>) -> Self {
        self.created = Some(created);
        self
    }

    /// Get the file extension (lowercase), if any
    pub fn extension(&self) -> Option<&str> {
        self.name.rsplit('.').next().filter(|ext| {
            // Make sure we actually found an extension, not the whole filename
            ext.len() < self.name.len()
        })
    }

    /// Initialize the lowercase name cache after deserialization
    pub fn init_cache(&mut self) {
        if self.name_lower.is_empty() {
            self.name_lower = self.name.to_lowercase();
        }
    }

    /// Check if this record matches the given extension (case-insensitive)
    pub fn has_extension(&self, ext: &str) -> bool {
        self.extension()
            .map(|e| e.eq_ignore_ascii_case(ext))
            .unwrap_or(false)
    }
}

impl PartialEq for FileRecord {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.volume_id == other.volume_id
    }
}

impl Eq for FileRecord {}

impl Hash for FileRecord {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.volume_id.hash(state);
    }
}

/// Statistics about the index
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexStats {
    /// Total number of files in the index
    pub total_files: u64,

    /// Total number of directories in the index
    pub total_dirs: u64,

    /// Total size of all indexed files in bytes
    pub total_size: u64,

    /// Number of volumes indexed
    pub volume_count: u32,

    /// When the index was last updated
    pub last_updated: Option<DateTime<Utc>>,

    /// Index format version
    pub version: u32,
}

impl IndexStats {
    /// Current index format version
    pub const CURRENT_VERSION: u32 = 1;

    /// Create new empty stats
    pub fn new() -> Self {
        IndexStats {
            version: Self::CURRENT_VERSION,
            ..Default::default()
        }
    }

    /// Total number of entries (files + directories)
    pub fn total_entries(&self) -> u64 {
        self.total_files + self.total_dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_record_extension() {
        let record = FileRecord::new(
            FileId(1),
            None,
            VolumeId::new("C"),
            "document.txt".to_string(),
            "C:\\document.txt".to_string(),
            false,
        );
        assert_eq!(record.extension(), Some("txt"));

        let record = FileRecord::new(
            FileId(2),
            None,
            VolumeId::new("C"),
            "archive.tar.gz".to_string(),
            "C:\\archive.tar.gz".to_string(),
            false,
        );
        assert_eq!(record.extension(), Some("gz"));

        let record = FileRecord::new(
            FileId(3),
            None,
            VolumeId::new("C"),
            "noextension".to_string(),
            "C:\\noextension".to_string(),
            false,
        );
        // "noextension" split on '.' returns just "noextension", which has the same length
        assert_eq!(record.extension(), None);
    }

    #[test]
    fn test_file_record_has_extension() {
        let record = FileRecord::new(
            FileId(1),
            None,
            VolumeId::new("C"),
            "Document.TXT".to_string(),
            "C:\\Document.TXT".to_string(),
            false,
        );
        assert!(record.has_extension("txt"));
        assert!(record.has_extension("TXT"));
        assert!(!record.has_extension("doc"));
    }

    #[test]
    fn test_volume_id() {
        let v1 = VolumeId::new("C");
        let v2 = VolumeId::from("C");
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_file_id() {
        let id = FileId::new(12345);
        assert_eq!(id.as_u64(), 12345);
        assert_eq!(format!("{}", id), "12345");
    }
}

//! Persistence layer for the Glint index.
//!
//! This module handles saving and loading the index to/from disk. The on-disk
//! format is designed for:
//!
//! - Fast loading: Binary format with optional compression
//! - Versioning: Format changes are detected and handled
//! - Atomic writes: Prevent corruption on crash
//! - Integrity: Basic checksums to detect corruption
//!
//! ## Index File Format
//!
//! The index file has the following structure:
//!
//! ```text
//! [Header: 32 bytes]
//!   - Magic: "GLNT" (4 bytes)
//!   - Version: u32 (4 bytes)
//!   - Flags: u32 (4 bytes) - compression, etc.
//!   - Record count: u64 (8 bytes)
//!   - Reserved: 12 bytes
//!
//! [Volume States: variable]
//!   - Volume count: u32
//!   - For each volume:
//!     - Volume info (bincode)
//!     - Journal state (bincode)
//!
//! [Records: variable]
//!   - Compressed bincode data
//!
//! [Footer: 8 bytes]
//!   - CRC32 checksum: u32
//!   - Magic: "TGLN" (4 bytes)
//! ```

use crate::backend::{JournalState, VolumeInfo};
use crate::error::{GlintError, Result};
use crate::index::{Index, VolumeIndexState};
use crate::types::{FileRecord, IndexStats, VolumeId};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Magic bytes at the start of index files
const MAGIC_HEADER: &[u8; 4] = b"GLNT";
/// Magic bytes at the end of index files (reversed)
const MAGIC_FOOTER: &[u8; 4] = b"TGLN";
/// Current index format version
pub const INDEX_VERSION: u32 = 1;

/// Flags for index file format
#[derive(Debug, Clone, Copy)]
pub struct IndexFlags(u32);

impl IndexFlags {
    /// No compression
    pub const NONE: Self = IndexFlags(0);
    /// LZ4 compression
    pub const COMPRESSED_LZ4: Self = IndexFlags(1);

    fn is_compressed(&self) -> bool {
        self.0 & 1 != 0
    }
}

/// Header structure for the index file
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexHeader {
    magic: [u8; 4],
    version: u32,
    flags: u32,
    record_count: u64,
    reserved: [u8; 12],
}

impl IndexHeader {
    fn new(record_count: u64, flags: IndexFlags) -> Self {
        IndexHeader {
            magic: *MAGIC_HEADER,
            version: INDEX_VERSION,
            flags: flags.0,
            record_count,
            reserved: [0; 12],
        }
    }

    fn validate(&self) -> Result<()> {
        if self.magic != *MAGIC_HEADER {
            return Err(GlintError::IndexCorrupted {
                reason: "Invalid magic bytes in header".to_string(),
            });
        }

        if self.version != INDEX_VERSION {
            return Err(GlintError::IndexVersionMismatch {
                found: self.version,
                expected: INDEX_VERSION,
            });
        }

        Ok(())
    }
}

/// Volume state as stored on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredVolumeState {
    id: String,
    mount_point: String,
    filesystem_type: String,
    label: Option<String>,
    journal_state: Option<JournalState>,
    record_count: u64,
}

impl From<&VolumeIndexState> for StoredVolumeState {
    fn from(state: &VolumeIndexState) -> Self {
        StoredVolumeState {
            id: state.info.id.as_str().to_string(),
            mount_point: state.info.mount_point.clone(),
            filesystem_type: state.info.filesystem_type.clone(),
            label: state.info.label.clone(),
            journal_state: state.journal_state.clone(),
            record_count: state.record_count,
        }
    }
}

impl StoredVolumeState {
    fn to_volume_index_state(&self) -> VolumeIndexState {
        VolumeIndexState {
            info: VolumeInfo::new(
                VolumeId::new(&self.id),
                &self.mount_point,
                &self.filesystem_type,
            ),
            journal_state: self.journal_state.clone(),
            record_count: self.record_count,
            needs_rescan: false,
        }
    }
}

/// Stored index data
#[derive(Debug, Serialize, Deserialize)]
struct StoredIndex {
    stats: IndexStats,
    volumes: Vec<StoredVolumeState>,
    records: Vec<FileRecord>,
}

/// Manages persistence of the index to disk.
///
/// ## Example
///
/// ```rust,ignore
/// use glint_core::{Index, IndexStore};
/// use std::path::Path;
///
/// let store = IndexStore::new(Path::new("./data"));
///
/// // Save an index
/// let index = Index::new();
/// store.save(&index)?;
///
/// // Load an index
/// let loaded = store.load()?;
/// ```
pub struct IndexStore {
    /// Base directory for storing index files
    base_dir: PathBuf,

    /// Whether to use compression
    use_compression: bool,
}

impl IndexStore {
    /// Create a new IndexStore with the given base directory.
    ///
    /// The directory will be created if it doesn't exist.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        IndexStore {
            base_dir: base_dir.as_ref().to_path_buf(),
            use_compression: true,
        }
    }

    /// Set whether to use compression when saving.
    pub fn with_compression(mut self, compress: bool) -> Self {
        self.use_compression = compress;
        self
    }

    /// Get the path to the main index file.
    pub fn index_path(&self) -> PathBuf {
        self.base_dir.join("glint.idx")
    }

    /// Get the path to a backup index file.
    fn backup_path(&self) -> PathBuf {
        self.base_dir.join("glint.idx.bak")
    }

    /// Get the path to a temporary file during save.
    fn temp_path(&self) -> PathBuf {
        self.base_dir.join("glint.idx.tmp")
    }

    /// Check if an index file exists.
    pub fn exists(&self) -> bool {
        self.index_path().exists()
    }

    /// Save the index to disk.
    ///
    /// Uses atomic write (write to temp, then rename) to prevent corruption.
    pub fn save(&self, index: &Index) -> Result<()> {
        // Ensure directory exists
        fs::create_dir_all(&self.base_dir)?;

        let records = index.all_records();
        let record_count = records.len() as u64;

        info!(
            path = %self.index_path().display(),
            records = record_count,
            "Saving index to disk"
        );

        let stored = StoredIndex {
            stats: index.stats(),
            volumes: index
                .volume_states()
                .iter()
                .map(StoredVolumeState::from)
                .collect(),
            records,
        };

        let flags = if self.use_compression {
            IndexFlags::COMPRESSED_LZ4
        } else {
            IndexFlags::NONE
        };

        // Serialize the data
        let data = bincode::serialize(&stored)?;

        // Optionally compress
        let final_data = if self.use_compression {
            lz4_flex::compress_prepend_size(&data)
        } else {
            data
        };

        // Calculate checksum
        let checksum = crc32_fast(&final_data);

        // Write to temp file
        let temp_path = self.temp_path();
        {
            let file = File::create(&temp_path)?;
            let mut writer = BufWriter::new(file);

            // Write header
            let header = IndexHeader::new(record_count, flags);
            let header_bytes = bincode::serialize(&header)?;
            writer.write_all(&header_bytes)?;

            // Write data
            writer.write_all(&final_data)?;

            // Write footer
            writer.write_all(&checksum.to_le_bytes())?;
            writer.write_all(MAGIC_FOOTER)?;

            writer.flush()?;
        }

        // Backup existing index
        let index_path = self.index_path();
        let backup_path = self.backup_path();
        if index_path.exists() {
            let _ = fs::remove_file(&backup_path);
            let _ = fs::rename(&index_path, &backup_path);
        }

        // Rename temp to final
        fs::rename(&temp_path, &index_path)?;

        debug!(
            size = final_data.len(),
            compressed = self.use_compression,
            "Index saved successfully"
        );

        Ok(())
    }

    /// Load the index from disk.
    ///
    /// Returns a new Index populated with the stored data.
    pub fn load(&self) -> Result<Index> {
        let index_path = self.index_path();

        if !index_path.exists() {
            return Err(GlintError::IndexNotFound { path: index_path });
        }

        info!(path = %index_path.display(), "Loading index from disk");

        let file = File::open(&index_path)?;
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        // Read and validate header
        let mut header_bytes = [0u8; 32];
        reader.read_exact(&mut header_bytes)?;
        let header: IndexHeader = bincode::deserialize(&header_bytes)?;
        header.validate()?;

        let flags = IndexFlags(header.flags);

        // Read data (everything except footer)
        let data_len = file_len as usize - 32 - 8; // header + footer
        let mut data = vec![0u8; data_len];
        reader.read_exact(&mut data)?;

        // Read and verify footer
        let mut footer = [0u8; 8];
        reader.read_exact(&mut footer)?;

        let stored_checksum = u32::from_le_bytes([footer[0], footer[1], footer[2], footer[3]]);
        let footer_magic = &footer[4..8];

        if footer_magic != MAGIC_FOOTER {
            return Err(GlintError::IndexCorrupted {
                reason: "Invalid footer magic bytes".to_string(),
            });
        }

        // Verify checksum
        let computed_checksum = crc32_fast(&data);
        if stored_checksum != computed_checksum {
            return Err(GlintError::IndexCorrupted {
                reason: format!(
                    "Checksum mismatch: expected {:08x}, got {:08x}",
                    stored_checksum, computed_checksum
                ),
            });
        }

        // Decompress if needed
        let decompressed = if flags.is_compressed() {
            lz4_flex::decompress_size_prepended(&data).map_err(|e| GlintError::IndexCorrupted {
                reason: format!("Decompression failed: {}", e),
            })?
        } else {
            data
        };

        // Deserialize
        let stored: StoredIndex =
            bincode::deserialize(&decompressed).map_err(|e| GlintError::IndexCorrupted {
                reason: format!("Deserialization failed: {}", e),
            })?;

        // Build the index
        let index = Index::with_capacity(stored.records.len());

        // Group records by volume and add them
        let mut records_by_volume: std::collections::HashMap<String, Vec<FileRecord>> =
            std::collections::HashMap::new();

        for mut record in stored.records {
            record.init_cache();
            let vid = record.volume_id.as_str().to_string();
            records_by_volume.entry(vid).or_default().push(record);
        }

        for vol_state in stored.volumes {
            let vid = vol_state.id.clone();
            if let Some(records) = records_by_volume.remove(&vid) {
                let volume_info = VolumeInfo::new(
                    VolumeId::new(&vol_state.id),
                    &vol_state.mount_point,
                    &vol_state.filesystem_type,
                );
                index.add_volume_records(&volume_info, records);

                // Restore journal state
                if let Some(js) = vol_state.journal_state {
                    index.update_journal_state(&VolumeId::new(&vid), js);
                }
            }
        }

        info!(
            records = index.len(),
            volumes = index.volume_states().len(),
            "Index loaded successfully"
        );

        Ok(index)
    }

    /// Load the index, or return a new empty one if loading fails.
    ///
    /// Logs a warning if loading fails.
    pub fn load_or_new(&self) -> Index {
        match self.load() {
            Ok(index) => index,
            Err(e) => {
                warn!(error = %e, "Failed to load index, starting fresh");
                Index::new()
            }
        }
    }

    /// Delete all stored index data.
    pub fn clear(&self) -> Result<()> {
        let index_path = self.index_path();
        let backup_path = self.backup_path();

        if index_path.exists() {
            fs::remove_file(&index_path)?;
        }
        if backup_path.exists() {
            fs::remove_file(&backup_path)?;
        }

        Ok(())
    }

    /// Restore from backup if main index is corrupted.
    pub fn restore_from_backup(&self) -> Result<Index> {
        let backup_path = self.backup_path();
        let index_path = self.index_path();

        if !backup_path.exists() {
            return Err(GlintError::IndexNotFound { path: backup_path });
        }

        // Copy backup to main
        fs::copy(&backup_path, &index_path)?;

        // Try to load
        self.load()
    }
}

/// Fast CRC32 checksum calculation.
///
/// Uses a simple implementation; could be optimized with SIMD or crc32fast crate.
fn crc32_fast(data: &[u8]) -> u32 {
    // CRC-32/ISO-HDLC polynomial
    const POLY: u32 = 0xEDB88320;

    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileId;
    use tempfile::TempDir;

    fn make_test_records() -> Vec<FileRecord> {
        vec![
            FileRecord::new(
                FileId::new(1),
                None,
                VolumeId::new("C"),
                "file1.txt".to_string(),
                "C:\\file1.txt".to_string(),
                false,
            ),
            FileRecord::new(
                FileId::new(2),
                None,
                VolumeId::new("C"),
                "file2.rs".to_string(),
                "C:\\file2.rs".to_string(),
                false,
            ),
        ]
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let store = IndexStore::new(temp_dir.path());

        // Create and populate an index
        let index = Index::new();
        let volume = VolumeInfo::new(VolumeId::new("C"), "C:", "NTFS");
        index.add_volume_records(&volume, make_test_records());

        // Save
        store.save(&index).unwrap();
        assert!(store.exists());

        // Load
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), index.len());
    }

    #[test]
    fn test_save_and_load_uncompressed() {
        let temp_dir = TempDir::new().unwrap();
        let store = IndexStore::new(temp_dir.path()).with_compression(false);

        let index = Index::new();
        let volume = VolumeInfo::new(VolumeId::new("C"), "C:", "NTFS");
        index.add_volume_records(&volume, make_test_records());

        store.save(&index).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), index.len());
    }

    #[test]
    fn test_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let store = IndexStore::new(temp_dir.path());

        let result = store.load();
        assert!(matches!(result, Err(GlintError::IndexNotFound { .. })));
    }

    #[test]
    fn test_load_or_new() {
        let temp_dir = TempDir::new().unwrap();
        let store = IndexStore::new(temp_dir.path());

        let index = store.load_or_new();
        assert!(index.is_empty());
    }

    #[test]
    fn test_clear() {
        let temp_dir = TempDir::new().unwrap();
        let store = IndexStore::new(temp_dir.path());

        let index = Index::new();
        store.save(&index).unwrap();
        assert!(store.exists());

        store.clear().unwrap();
        assert!(!store.exists());
    }

    #[test]
    fn test_crc32() {
        let data = b"Hello, World!";
        let crc = crc32_fast(data);
        // This is the expected CRC-32/ISO-HDLC value
        assert_eq!(crc, 0xEC4AC3D0);
    }

    #[test]
    fn test_corrupted_index() {
        let temp_dir = TempDir::new().unwrap();
        let index_path = temp_dir.path().join("glint.idx");

        // Write garbage
        fs::write(&index_path, b"not a valid index file").unwrap();

        let store = IndexStore::new(temp_dir.path());
        let result = store.load();
        assert!(result.is_err());
    }
}

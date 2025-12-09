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
use crate::archive;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use rayon::prelude::*;

/// Magic bytes at the start of index files
pub const MAGIC_HEADER: &[u8; 4] = b"GLNT";
/// Magic bytes at the end of index files (reversed)
pub const MAGIC_FOOTER: &[u8; 4] = b"TGLN";
/// Current index format version
pub const INDEX_VERSION: u32 = 3;

/// Flags for index file format
#[derive(Debug, Clone, Copy)]
pub struct IndexFlags(u32);

impl IndexFlags {
    /// No compression
    pub const NONE: Self = IndexFlags(0);
    /// LZ4 compression
    pub const COMPRESSED_LZ4: Self = IndexFlags(1);
    /// Chunked records section (v2+)
    pub const CHUNKED: Self = IndexFlags(2);

    fn is_compressed(&self) -> bool {
        self.0 & 1 != 0
    }
    fn is_chunked(&self) -> bool { self.0 & 2 != 0 }
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
        // Accept older versions; newer versions fail.
        if self.version > INDEX_VERSION {
            return Err(GlintError::IndexVersionMismatch { found: self.version, expected: INDEX_VERSION });
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

/// Stored index metadata (stats + volumes) used in v2 chunked format
#[derive(Debug, Serialize, Deserialize)]
struct StoredMeta {
    stats: IndexStats,
    volumes: Vec<StoredVolumeState>,
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

        // v3 rkyv format (uncompressed for fastest startup)
        let flags = IndexFlags::NONE;

        // (v3 does not use meta_bytes)

        // Prepare chunks of records
        let chunk_size: usize = 200_000.max(1);
        let total = records.len();
        let chunks: Vec<&[FileRecord]> = (0..total)
            .step_by(chunk_size)
            .map(|start| {
                let end = (start + chunk_size).min(total);
                &records[start..end]
            })
            .collect();

        // Serialize (and compress) each chunk
        let mut chunk_blobs: Vec<Vec<u8>> = Vec::with_capacity(chunks.len());
        for ch in &chunks {
            let bytes = bincode::serialize(ch)?;
            let blob = if self.use_compression {
                lz4_flex::compress_prepend_size(&bytes)
            } else {
                bytes
            };
            chunk_blobs.push(blob);
        }

        // Checksum computed after assembling data buffer below

        // Write to temp file
        let temp_path = self.temp_path();
        {
            let file = File::create(&temp_path)?;
            let mut writer = BufWriter::new(file);

            // Write header
            let header = IndexHeader::new(record_count, flags);
            let header_bytes = bincode::serialize(&header)?;
            writer.write_all(&header_bytes)?;

            // Build rkyv archive in memory and write directly
            let data_buf = archive::build_archived_bytes(index);
            writer.write_all(&data_buf)?;

            // Write footer
            let checksum = crc32fast::hash(&data_buf);
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

        debug!(compressed = false, "Index saved successfully (v3 rkyv)");

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
        let computed_checksum = crc32fast::hash(&data);
        if stored_checksum != computed_checksum {
            return Err(GlintError::IndexCorrupted {
                reason: format!(
                    "Checksum mismatch: expected {:08x}, got {:08x}",
                    stored_checksum, computed_checksum
                ),
            });
        }

        // v3 path: rkyv archive (uncompressed)
        if header.version == 3 {
            // Map into memory for zero-copy view
            // (We still build an Index today for compatibility. Next step: expose a zero-copy view.)
            // No decompression step; data is an rkyv archive
            unsafe {
                let root = archive::archived_root(&data);
                let mut recs: Vec<FileRecord> = Vec::with_capacity(root.is_dir.len());
                for i in 0..root.is_dir.len() {
                    let noff = root.name_offsets[i] as usize;
                    let poff = root.path_offsets[i] as usize;
                    let name = read_cstr(&root.names_blob[noff..]);
                    let path = read_cstr(&root.paths_blob[poff..]);
                    use crate::types::{FileId, VolumeId as VID};
                    let rec = FileRecord::new(
                        FileId::new(i as u64 + 1),
                        None,
                        VID::new("V"),
                        name.to_string(),
                        path.to_string(),
                        root.is_dir[i] != 0,
                    );
                    recs.push(rec);
                }
                let idx = Index::with_capacity(recs.len());
                let vol = VolumeInfo::new(VolumeId::new("V"), "V:", "NTFS");
                idx.add_volume_records(&vol, recs);
                info!(records = idx.len(), "Index loaded successfully (v3 rkyv)");
                return Ok(idx);
            }
        }

        // v1 path (legacy): single blob (maybe compressed) containing StoredIndex
        if header.version == 1 && !flags.is_chunked() {
            let decompressed = if flags.is_compressed() {
                lz4_flex::decompress_size_prepended(&data)
                    .map_err(|e| GlintError::IndexCorrupted { reason: format!("Decompression failed: {}", e) })?
            } else { data };

            let stored: StoredIndexV1 = bincode::deserialize(&decompressed)
                .map_err(|e| GlintError::IndexCorrupted { reason: format!("Deserialization failed: {}", e) })?;

            let mut records: Vec<FileRecord> = stored.records;
            records.par_iter_mut().for_each(|r| r.init_cache());
            let index = Index::with_capacity(records.len());
            let mut records_by_volume: std::collections::HashMap<String, Vec<FileRecord>> = std::collections::HashMap::new();
            for record in records { records_by_volume.entry(record.volume_id.as_str().to_string()).or_default().push(record); }
            for vol_state in stored.volumes {
                let vid = vol_state.id.clone();
                if let Some(records) = records_by_volume.remove(&vid) {
                    let volume_info = VolumeInfo::new(VolumeId::new(&vol_state.id), &vol_state.mount_point, &vol_state.filesystem_type);
                    index.add_volume_records(&volume_info, records);
                    if let Some(js) = vol_state.journal_state { index.update_journal_state(&VolumeId::new(&vid), js); }
                }
            }
            info!(records = index.len(), volumes = index.volume_states().len(), "Index loaded successfully (v1)");
            // Opportunistically rewrite to v2 chunked format for faster future loads
            if let Err(e) = self.save(&index) {
                warn!(error = %e, "Failed to rewrite index to v2 format");
            }
            return Ok(index);
        }

        // v2 path: chunked
        if !flags.is_chunked() {
            return Err(GlintError::IndexCorrupted { reason: "Expected chunked format (v2), but flag not set".to_string() });
        }

        // Parse meta and chunks from data buffer
        let mut cursor = 0usize;
        if data.len() < 4 { return Err(GlintError::IndexCorrupted { reason: "Truncated meta length".to_string() }); }
        let meta_len = u32::from_le_bytes([data[cursor], data[cursor+1], data[cursor+2], data[cursor+3]]) as usize; cursor += 4;
        if cursor + meta_len > data.len() { return Err(GlintError::IndexCorrupted { reason: "Truncated meta".to_string() }); }
        let meta_bytes = &data[cursor..cursor+meta_len]; cursor += meta_len;
        if cursor + 4 > data.len() { return Err(GlintError::IndexCorrupted { reason: "Truncated chunk count".to_string() }); }
        let chunk_count = u32::from_le_bytes([data[cursor], data[cursor+1], data[cursor+2], data[cursor+3]]) as usize; cursor += 4;

        let meta: StoredMeta = bincode::deserialize(meta_bytes)
            .map_err(|e| GlintError::IndexCorrupted { reason: format!("Meta deserialization failed: {}", e) })?;

        let mut chunk_slices: Vec<&[u8]> = Vec::with_capacity(chunk_count);
        for _ in 0..chunk_count {
            if cursor + 4 > data.len() { return Err(GlintError::IndexCorrupted { reason: "Truncated chunk length".to_string() }); }
            let len = u32::from_le_bytes([data[cursor], data[cursor+1], data[cursor+2], data[cursor+3]]) as usize; cursor += 4;
            if cursor + len > data.len() { return Err(GlintError::IndexCorrupted { reason: "Truncated chunk".to_string() }); }
            let slice = &data[cursor..cursor+len];
            cursor += len;
            chunk_slices.push(slice);
        }

        // Decompress + deserialize chunks in parallel
        let mut all_records: Vec<FileRecord> = chunk_slices
            .par_iter()
            .map(|blob| {
                let bytes = if flags.is_compressed() {
                    lz4_flex::decompress_size_prepended(blob)
                        .map_err(|e| GlintError::IndexCorrupted { reason: format!("Decompression failed: {}", e) })?
                } else { (*blob).to_vec() };
                let mut recs: Vec<FileRecord> = bincode::deserialize(&bytes)
                    .map_err(|e| GlintError::IndexCorrupted { reason: format!("Deserialization failed: {}", e) })?;
                recs.par_iter_mut().for_each(|r| r.init_cache());
                Ok::<Vec<FileRecord>, GlintError>(recs)
            })
            .try_reduce(|| Vec::new(), |mut acc, mut v| { acc.append(&mut v); Ok::<Vec<FileRecord>, GlintError>(acc) })?;

        // Build the index
        let index = Index::with_capacity(all_records.len());
        // Group by volume
        let mut records_by_volume: std::collections::HashMap<String, Vec<FileRecord>> = std::collections::HashMap::new();
        for record in all_records.drain(..) {
            records_by_volume.entry(record.volume_id.as_str().to_string()).or_default().push(record);
        }
        for vol_state in meta.volumes {
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

fn read_cstr(bytes: &[u8]) -> &str {
    let mut end = 0;
    while end < bytes.len() && bytes[end] != 0 { end += 1; }
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

// Legacy v1 stored representation used only for backward-compatible loads
#[derive(Debug, Serialize, Deserialize)]
struct StoredIndexV1 {
    stats: IndexStats,
    volumes: Vec<StoredVolumeState>,
    records: Vec<FileRecord>,
}

// Checksum calculation now uses the optimized crc32fast crate.

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

    // CRC is validated indirectly via save/load paths.

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

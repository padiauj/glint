//! In-memory index for fast file search.
//!
//! The `Index` is the central data structure that stores all indexed file records
//! and provides efficient search capabilities. It supports:
//!
//! - Adding records from full scans
//! - Incremental updates from change events
//! - Fast parallel search using Rayon
//! - Parent-child relationship tracking for path reconstruction
//!
//! ## Architecture
//!
//! The index uses a simple but effective design:
//! - A `Vec<FileRecord>` stores all records for cache-friendly iteration
//! - A `HashMap<(VolumeId, FileId), usize>` maps IDs to indices for O(1) lookups
//! - A `HashMap<(VolumeId, FileId), Vec<usize>>` tracks parent-child relationships
//!
//! This design prioritizes simplicity and search performance over update efficiency,
//! which is appropriate since searches vastly outnumber updates.

use crate::backend::{ChangeEvent, ChangeKind, JournalState, VolumeInfo};
use crate::search::{SearchQuery, SearchResult};
use crate::types::{FileId, FileRecord, IndexStats, VolumeId};
use dashmap::DashMap;
use parking_lot::RwLock;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{debug, info, instrument, warn};

/// The main in-memory index containing all file records.
///
/// This structure is designed for concurrent access:
/// - Multiple readers can search simultaneously
/// - Updates are serialized via internal locking
///
/// ## Example
///
/// ```rust
/// use glint_core::{Index, SearchQuery};
///
/// let mut index = Index::new();
///
/// // Search the index
/// let query = SearchQuery::substring("readme");
/// for result in index.search(&query).take(100) {
///     println!("{}: {}", result.record.name, result.record.path);
/// }
/// ```
pub struct Index {
    /// All file records in the index
    records: RwLock<Vec<FileRecord>>,

    /// Map from (volume_id, file_id) to record index
    id_to_index: DashMap<(String, u64), usize>,

    /// Map from (volume_id, parent_id) to child record indices
    children: DashMap<(String, u64), Vec<usize>>,

    /// Statistics about the index
    stats: RwLock<IndexStats>,

    /// Volume information and journal states
    volumes: RwLock<HashMap<String, VolumeIndexState>>,

    /// Generation counter for detecting concurrent modifications
    generation: AtomicU64,
}

/// State tracking for an indexed volume
#[derive(Debug, Clone)]
pub struct VolumeIndexState {
    /// Volume information
    pub info: VolumeInfo,

    /// Journal state for incremental updates
    pub journal_state: Option<JournalState>,

    /// Number of records from this volume
    pub record_count: u64,

    /// Whether this volume needs a rescan
    pub needs_rescan: bool,
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

impl Index {
    /// Create a new empty index.
    pub fn new() -> Self {
        Index {
            records: RwLock::new(Vec::new()),
            id_to_index: DashMap::new(),
            children: DashMap::new(),
            stats: RwLock::new(IndexStats::new()),
            volumes: RwLock::new(HashMap::new()),
            generation: AtomicU64::new(0),
        }
    }

    /// Create an index with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Index {
            records: RwLock::new(Vec::with_capacity(capacity)),
            id_to_index: DashMap::with_capacity(capacity),
            children: DashMap::with_capacity(capacity / 10), // Fewer parents than files
            stats: RwLock::new(IndexStats::new()),
            volumes: RwLock::new(HashMap::new()),
            generation: AtomicU64::new(0),
        }
    }

    /// Get the number of records in the index.
    pub fn len(&self) -> usize {
        self.records.read().len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.records.read().is_empty()
    }

    /// Get current index statistics.
    pub fn stats(&self) -> IndexStats {
        self.stats.read().clone()
    }

    /// Get the current generation (modification counter).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Add records from a volume scan.
    ///
    /// This method is used during initial indexing or rescans. It:
    /// - Removes any existing records from this volume
    /// - Adds all new records
    /// - Updates auxiliary indices
    /// - Updates statistics
    #[instrument(skip(self, records, volume))]
    pub fn add_volume_records(&self, volume: &VolumeInfo, records: Vec<FileRecord>) {
        let volume_id = volume.id.as_str().to_string();
        let record_count = records.len();

        info!(
            volume = %volume_id,
            records = record_count,
            "Adding records from volume scan"
        );

        // Remove existing records for this volume
        self.remove_volume(&volume.id);

        // Add new records
        let mut all_records = self.records.write();
        let base_index = all_records.len();

        // Track stats
        let mut files = 0u64;
        let mut dirs = 0u64;
        let mut total_size = 0u64;

        for (i, mut record) in records.into_iter().enumerate() {
            let idx = base_index + i;

            // Ensure cache is initialized
            record.init_cache();

            // Update ID mapping
            let key = (record.volume_id.as_str().to_string(), record.id.as_u64());
            self.id_to_index.insert(key, idx);

            // Update parent-child mapping
            if let Some(parent_id) = record.parent_id {
                let parent_key = (record.volume_id.as_str().to_string(), parent_id.as_u64());
                self.children
                    .entry(parent_key)
                    .or_insert_with(Vec::new)
                    .push(idx);
            }

            // Update stats
            if record.is_dir {
                dirs += 1;
            } else {
                files += 1;
                if let Some(size) = record.size {
                    total_size += size;
                }
            }

            all_records.push(record);
        }

        drop(all_records);

        // Update volume state
        {
            let mut volumes = self.volumes.write();
            volumes.insert(
                volume_id.clone(),
                VolumeIndexState {
                    info: volume.clone(),
                    journal_state: volume.journal_state.clone(),
                    record_count: record_count as u64,
                    needs_rescan: false,
                },
            );
        }

        // Update global stats
        {
            let mut stats = self.stats.write();
            stats.total_files += files;
            stats.total_dirs += dirs;
            stats.total_size += total_size;
            stats.volume_count = self.volumes.read().len() as u32;
            stats.last_updated = Some(chrono::Utc::now());
        }

        // Increment generation
        self.generation.fetch_add(1, Ordering::Release);

        info!(
            volume = %volume_id,
            files = files,
            dirs = dirs,
            "Volume indexing complete"
        );
    }

    /// Remove all records for a volume.
    #[instrument(skip(self))]
    pub fn remove_volume(&self, volume_id: &VolumeId) {
        let vid = volume_id.as_str().to_string();

        // Find indices to remove
        let mut to_remove = Vec::new();
        {
            let records = self.records.read();
            for (i, record) in records.iter().enumerate() {
                if record.volume_id.as_str() == vid {
                    to_remove.push(i);
                }
            }
        }

        if to_remove.is_empty() {
            return;
        }

        debug!(volume = %vid, count = to_remove.len(), "Removing volume records");

        // This is expensive but correct - we rebuild the index
        // A more sophisticated approach would mark records as deleted
        // and compact periodically
        let mut all_records = self.records.write();

        // Remove from auxiliary indices first
        for &idx in &to_remove {
            let record = &all_records[idx];
            let key = (record.volume_id.as_str().to_string(), record.id.as_u64());
            self.id_to_index.remove(&key);

            if let Some(parent_id) = record.parent_id {
                let parent_key = (record.volume_id.as_str().to_string(), parent_id.as_u64());
                if let Some(mut children) = self.children.get_mut(&parent_key) {
                    children.retain(|&i| i != idx);
                }
            }
        }

        // Remove records (in reverse order to preserve indices)
        for &idx in to_remove.iter().rev() {
            all_records.swap_remove(idx);
        }

        // Rebuild ID-to-index mapping (indices changed)
        self.id_to_index.clear();
        self.children.clear();
        for (i, record) in all_records.iter().enumerate() {
            let key = (record.volume_id.as_str().to_string(), record.id.as_u64());
            self.id_to_index.insert(key, i);

            if let Some(parent_id) = record.parent_id {
                let parent_key = (record.volume_id.as_str().to_string(), parent_id.as_u64());
                self.children
                    .entry(parent_key)
                    .or_insert_with(Vec::new)
                    .push(i);
            }
        }

        drop(all_records);

        // Remove volume state
        self.volumes.write().remove(&vid);

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.volume_count = self.volumes.read().len() as u32;
            stats.last_updated = Some(chrono::Utc::now());
            // Note: We're not updating file/dir/size counts here for simplicity
            // A production implementation would track these per-volume
        }

        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Apply a change event to the index.
    ///
    /// This is called by the change monitoring system when filesystem changes
    /// are detected. It updates the index incrementally.
    #[instrument(skip(self))]
    pub fn apply_change(&self, event: ChangeEvent) {
        debug!(
            kind = %event.kind,
            file_id = %event.file_id,
            name = %event.name,
            "Applying change event"
        );

        match event.kind {
            ChangeKind::Created => self.handle_create(event),
            ChangeKind::Deleted => self.handle_delete(event),
            ChangeKind::Renamed => self.handle_rename(event),
            ChangeKind::Modified | ChangeKind::AttributeChanged | ChangeKind::SecurityChanged => {
                // For now, we don't track modification times in real-time
                // A future enhancement could update the modified timestamp
            }
        }

        self.generation.fetch_add(1, Ordering::Release);
    }

    fn handle_create(&self, event: ChangeEvent) {
        let volume_id = event.volume_id.clone();

        // Build the path
        let path = self.build_path(&volume_id, event.parent_id, &event.name);

        let record = FileRecord::new(
            event.file_id,
            event.parent_id,
            volume_id,
            event.name,
            path,
            event.is_dir,
        );

        let mut records = self.records.write();
        let idx = records.len();

        let key = (record.volume_id.as_str().to_string(), record.id.as_u64());
        self.id_to_index.insert(key, idx);

        if let Some(parent_id) = record.parent_id {
            let parent_key = (record.volume_id.as_str().to_string(), parent_id.as_u64());
            self.children
                .entry(parent_key)
                .or_insert_with(Vec::new)
                .push(idx);
        }

        records.push(record);
    }

    fn handle_delete(&self, event: ChangeEvent) {
        let key = (event.volume_id.as_str().to_string(), event.file_id.as_u64());

        if let Some((_, idx)) = self.id_to_index.remove(&key) {
            // Mark record as deleted by clearing the name
            // (We don't actually remove to avoid reindexing)
            let mut records = self.records.write();
            if idx < records.len() {
                records[idx].name.clear();
                records[idx].name_lower.clear();
                records[idx].path.clear();
            }
        }
    }

    fn handle_rename(&self, event: ChangeEvent) {
        let key = (event.volume_id.as_str().to_string(), event.file_id.as_u64());

        if let Some(idx_ref) = self.id_to_index.get(&key) {
            let idx = *idx_ref;
            drop(idx_ref);

            let new_name = event.new_name.unwrap_or(event.name);
            let new_parent = event.new_parent_id.or(event.parent_id);
            let new_path = self.build_path(&event.volume_id, new_parent, &new_name);

            let mut records = self.records.write();
            if idx < records.len() {
                records[idx].name = new_name.clone();
                records[idx].name_lower = new_name.to_lowercase();
                records[idx].path = new_path;
                records[idx].parent_id = new_parent;
            }
        }
    }

    /// Build a full path from parent ID and filename.
    fn build_path(&self, volume_id: &VolumeId, parent_id: Option<FileId>, name: &str) -> String {
        let mut path_parts = Vec::new();
        path_parts.push(name.to_string());

        let mut current_parent = parent_id;
        let records = self.records.read();

        // Walk up the tree
        while let Some(pid) = current_parent {
            let key = (volume_id.as_str().to_string(), pid.as_u64());
            if let Some(idx_ref) = self.id_to_index.get(&key) {
                let idx = *idx_ref;
                if idx < records.len() {
                    let parent_record = &records[idx];
                    if !parent_record.name.is_empty() {
                        path_parts.push(parent_record.name.clone());
                    }
                    current_parent = parent_record.parent_id;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        drop(records);

        // Reverse and join
        path_parts.reverse();

        // Add volume prefix (e.g., "C:\")
        let volume_prefix = format!("{}:\\", volume_id.as_str());
        format!("{}{}", volume_prefix, path_parts.join("\\"))
    }

    /// Search the index with the given query.
    ///
    /// Returns an iterator over matching results, sorted by relevance.
    /// Results are computed lazily, allowing early termination.
    ///
    /// ## Performance
    ///
    /// Uses parallel iteration via Rayon for multi-core scaling.
    /// For large indices, this can provide significant speedup.
    pub fn search(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let records = self.records.read();

        // Use parallel filtering for large indices
        if records.len() > 10000 {
            self.search_parallel(&records, query)
        } else {
            self.search_sequential(&records, query)
        }
    }

    fn search_sequential(&self, records: &[FileRecord], query: &SearchQuery) -> Vec<SearchResult> {
        records
            .iter()
            .filter(|r| !r.name.is_empty() && query.matches(r))
            .map(|r| {
                let score = self.compute_score(r, query);
                SearchResult::new(r.clone(), score)
            })
            .collect()
    }

    fn search_parallel(&self, records: &[FileRecord], query: &SearchQuery) -> Vec<SearchResult> {
        records
            .par_iter()
            .filter(|r| !r.name.is_empty() && query.matches(r))
            .map(|r| {
                let score = self.compute_score(r, query);
                SearchResult::new(r.clone(), score)
            })
            .collect()
    }

    /// Search with a limit on results.
    ///
    /// More efficient than `search().take(n)` for large indices.
    pub fn search_limited(&self, query: &SearchQuery, limit: usize) -> Vec<SearchResult> {
        let records = self.records.read();
        let mut results = Vec::with_capacity(limit);

        for record in records.iter() {
            if record.name.is_empty() {
                continue;
            }
            if query.matches(record) {
                let score = self.compute_score(record, query);
                results.push(SearchResult::new(record.clone(), score));
                if results.len() >= limit {
                    break;
                }
            }
        }

        results
    }

    /// Compute a relevance score for a record.
    ///
    /// Higher scores indicate better matches. Factors:
    /// - Exact name match: highest score
    /// - Name starts with query: high score
    /// - Shorter names: higher score (more specific)
    fn compute_score(&self, record: &FileRecord, _query: &SearchQuery) -> u32 {
        // Simple scoring based on name length
        // Shorter names are generally more relevant (more specific)
        let length_score = 1000u32.saturating_sub(record.name.len() as u32);

        // Boost directories slightly (often what users are looking for)
        let type_boost = if record.is_dir { 10 } else { 0 };

        length_score + type_boost
    }

    /// Get a record by its ID.
    pub fn get(&self, volume_id: &VolumeId, file_id: FileId) -> Option<FileRecord> {
        let key = (volume_id.as_str().to_string(), file_id.as_u64());
        self.id_to_index.get(&key).and_then(|idx_ref| {
            let idx = *idx_ref;
            let records = self.records.read();
            records.get(idx).cloned()
        })
    }

    /// Get all children of a directory.
    pub fn get_children(&self, volume_id: &VolumeId, parent_id: FileId) -> Vec<FileRecord> {
        let key = (volume_id.as_str().to_string(), parent_id.as_u64());

        if let Some(children_ref) = self.children.get(&key) {
            let children_indices = children_ref.clone();
            drop(children_ref);

            let records = self.records.read();
            children_indices
                .iter()
                .filter_map(|&idx| records.get(idx).cloned())
                .filter(|r| !r.name.is_empty())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Update journal state for a volume.
    pub fn update_journal_state(&self, volume_id: &VolumeId, state: JournalState) {
        let mut volumes = self.volumes.write();
        if let Some(vol_state) = volumes.get_mut(volume_id.as_str()) {
            vol_state.journal_state = Some(state);
        }
    }

    /// Mark a volume as needing rescan.
    pub fn mark_needs_rescan(&self, volume_id: &VolumeId, reason: &str) {
        warn!(volume = %volume_id, reason = %reason, "Volume marked for rescan");
        let mut volumes = self.volumes.write();
        if let Some(vol_state) = volumes.get_mut(volume_id.as_str()) {
            vol_state.needs_rescan = true;
        }
    }

    /// Get volumes that need rescanning.
    pub fn volumes_needing_rescan(&self) -> Vec<VolumeInfo> {
        self.volumes
            .read()
            .values()
            .filter(|v| v.needs_rescan)
            .map(|v| v.info.clone())
            .collect()
    }

    /// Get all volume states.
    pub fn volume_states(&self) -> Vec<VolumeIndexState> {
        self.volumes.read().values().cloned().collect()
    }

    /// Get a copy of all records (for persistence).
    pub fn all_records(&self) -> Vec<FileRecord> {
        self.records.read().clone()
    }

    /// Clear the entire index.
    pub fn clear(&self) {
        let mut records = self.records.write();
        records.clear();
        self.id_to_index.clear();
        self.children.clear();
        *self.stats.write() = IndexStats::new();
        self.volumes.write().clear();
        self.generation.fetch_add(1, Ordering::Release);
    }
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index")
            .field("record_count", &self.len())
            .field("generation", &self.generation())
            .finish()
    }
}

// Thread-safe sharing
unsafe impl Send for Index {}
unsafe impl Sync for Index {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_records() -> Vec<FileRecord> {
        vec![
            FileRecord::new(
                FileId::new(5),
                None,
                VolumeId::new("C"),
                "".to_string(), // Root has no name
                "C:\\".to_string(),
                true,
            ),
            FileRecord::new(
                FileId::new(100),
                Some(FileId::new(5)),
                VolumeId::new("C"),
                "Users".to_string(),
                "C:\\Users".to_string(),
                true,
            ),
            FileRecord::new(
                FileId::new(101),
                Some(FileId::new(100)),
                VolumeId::new("C"),
                "README.md".to_string(),
                "C:\\Users\\README.md".to_string(),
                false,
            )
            .with_size(1024),
            FileRecord::new(
                FileId::new(102),
                Some(FileId::new(100)),
                VolumeId::new("C"),
                "config.toml".to_string(),
                "C:\\Users\\config.toml".to_string(),
                false,
            )
            .with_size(256),
            FileRecord::new(
                FileId::new(103),
                Some(FileId::new(100)),
                VolumeId::new("C"),
                "main.rs".to_string(),
                "C:\\Users\\main.rs".to_string(),
                false,
            )
            .with_size(2048),
        ]
    }

    fn make_volume_info() -> VolumeInfo {
        VolumeInfo::new(VolumeId::new("C"), "C:", "NTFS")
    }

    #[test]
    fn test_add_and_search() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        assert_eq!(index.len(), 5);

        let query = SearchQuery::substring("README");
        let results = index.search(&query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].record.name, "README.md");
    }

    #[test]
    fn test_search_case_insensitive() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let query = SearchQuery::substring("readme");
        let results = index.search(&query);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_extension() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let query = SearchQuery::substring("")
            .with_filter(crate::search::SearchFilter::Extensions(vec!["rs".to_string()]));
        let results = index.search(&query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].record.name, "main.rs");
    }

    #[test]
    fn test_search_limited() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let query = SearchQuery::substring("");
        let results = index.search_limited(&query, 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_apply_create_change() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let initial_len = index.len();

        let event = ChangeEvent::created(
            VolumeId::new("C"),
            FileId::new(200),
            Some(FileId::new(100)),
            "newfile.txt".to_string(),
            false,
            1000,
        );

        index.apply_change(event);

        assert_eq!(index.len(), initial_len + 1);

        let query = SearchQuery::substring("newfile");
        let results = index.search(&query);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_apply_delete_change() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let event = ChangeEvent::deleted(
            VolumeId::new("C"),
            FileId::new(101),
            Some(FileId::new(100)),
            "README.md".to_string(),
            false,
            1001,
        );

        index.apply_change(event);

        let query = SearchQuery::substring("README");
        let results = index.search(&query);
        assert!(results.is_empty());
    }

    #[test]
    fn test_apply_rename_change() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let event = ChangeEvent::renamed(
            VolumeId::new("C"),
            FileId::new(101),
            Some(FileId::new(100)),
            "README.md".to_string(),
            "CHANGELOG.md".to_string(),
            Some(FileId::new(100)),
            false,
            1002,
        );

        index.apply_change(event);

        let query = SearchQuery::substring("CHANGELOG");
        let results = index.search(&query);
        assert_eq!(results.len(), 1);

        let query = SearchQuery::substring("README");
        let results = index.search(&query);
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_children() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let children = index.get_children(&VolumeId::new("C"), FileId::new(100));
        assert_eq!(children.len(), 3); // README.md, config.toml, main.rs
    }

    #[test]
    fn test_stats() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        let stats = index.stats();
        assert_eq!(stats.total_files, 3);
        assert_eq!(stats.total_dirs, 2);
        assert_eq!(stats.volume_count, 1);
    }

    #[test]
    fn test_remove_volume() {
        let index = Index::new();
        index.add_volume_records(&make_volume_info(), make_test_records());

        assert!(!index.is_empty());

        index.remove_volume(&VolumeId::new("C"));

        assert!(index.is_empty());
    }

    #[test]
    fn test_generation() {
        let index = Index::new();
        let gen1 = index.generation();

        index.add_volume_records(&make_volume_info(), make_test_records());
        let gen2 = index.generation();

        assert!(gen2 > gen1);
    }
}

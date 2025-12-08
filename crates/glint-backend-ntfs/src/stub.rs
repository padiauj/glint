//! Stub implementation for non-Windows platforms.

use glint_core::backend::{
    ChangeHandler, FileSystemBackend, JournalState, ScanProgress, VolumeInfo, WatchHandle,
};
use glint_core::types::FileRecord;
use std::sync::Arc;

/// Stub NTFS backend for non-Windows platforms.
///
/// This allows the crate to compile on non-Windows platforms,
/// but all operations will fail with an appropriate error.
pub struct NtfsBackend;

impl NtfsBackend {
    /// Create a new stub backend.
    pub fn new() -> Self {
        NtfsBackend
    }
}

impl Default for NtfsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemBackend for NtfsBackend {
    fn list_volumes(&self) -> anyhow::Result<Vec<VolumeInfo>> {
        anyhow::bail!("NTFS backend is only available on Windows")
    }

    fn full_scan(
        &self,
        _volume: &VolumeInfo,
        _progress: Option<Arc<dyn ScanProgress>>,
    ) -> anyhow::Result<Vec<FileRecord>> {
        anyhow::bail!("NTFS backend is only available on Windows")
    }

    fn watch_changes(
        &self,
        _volume: VolumeInfo,
        _handler: Arc<dyn ChangeHandler>,
    ) -> anyhow::Result<WatchHandle> {
        anyhow::bail!("NTFS backend is only available on Windows")
    }

    fn get_journal_state(&self, _volume: &VolumeInfo) -> anyhow::Result<Option<JournalState>> {
        anyhow::bail!("NTFS backend is only available on Windows")
    }

    fn name(&self) -> &'static str {
        "ntfs-stub"
    }
}

//! NTFS backend implementation.
//!
//! This module implements the `FileSystemBackend` trait for NTFS volumes
//! on Windows. It combines MFT scanning and USN journal monitoring.

use crate::error::NtfsError;
use crate::mft::{scan_mft, scan_recursive};
use crate::usn::{get_journal_state, UsnWatcher};
use crate::volume::enumerate_ntfs_volumes;
use glint_core::backend::{
    ChangeHandler, FileSystemBackend, JournalState, ScanProgress, VolumeInfo, WatchHandle,
};
use glint_core::types::FileRecord;
use std::sync::Arc;
use tracing::{info, warn};

/// NTFS filesystem backend for Windows.
///
/// This backend provides:
/// - Fast initial indexing via MFT enumeration
/// - Real-time updates via USN Change Journal monitoring
///
/// ## Permissions
///
/// Full functionality requires elevated privileges:
/// - Run as Administrator, OR
/// - Have "Perform Volume Maintenance Tasks" privilege
///
/// Without elevation, the backend falls back to recursive directory
/// enumeration, which is slower but doesn't require special permissions.
pub struct NtfsBackend {
    /// Whether to attempt MFT access (requires elevation)
    try_mft: bool,
}

impl NtfsBackend {
    /// Create a new NTFS backend.
    pub fn new() -> Self {
        NtfsBackend { try_mft: true }
    }

    /// Create a backend that skips MFT access attempts.
    ///
    /// Use this if you know the process doesn't have elevated privileges
    /// to avoid the overhead of failed access attempts.
    pub fn without_mft() -> Self {
        NtfsBackend { try_mft: false }
    }

    /// Check if we have elevated privileges.
    pub fn has_elevated_privileges() -> bool {
        // Try to open C: volume for reading
        // This is a simple heuristic; actual privilege check would use OpenProcessToken
        crate::winapi_utils::open_volume("\\\\.\\C:").is_ok()
    }
}

impl Default for NtfsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemBackend for NtfsBackend {
    fn list_volumes(&self) -> anyhow::Result<Vec<VolumeInfo>> {
        let ntfs_volumes = enumerate_ntfs_volumes().map_err(|e| anyhow::anyhow!("{}", e))?;

        let volumes: Vec<VolumeInfo> = ntfs_volumes
            .into_iter()
            .map(|v| v.to_volume_info())
            .collect();

        info!(count = volumes.len(), "Enumerated NTFS volumes");

        for vol in &volumes {
            info!(
                mount = %vol.mount_point,
                label = ?vol.label,
                fs = %vol.filesystem_type,
                "Found volume"
            );
        }

        Ok(volumes)
    }

    fn full_scan(
        &self,
        volume: &VolumeInfo,
        progress: Option<Arc<dyn ScanProgress>>,
    ) -> anyhow::Result<Vec<FileRecord>> {
        // Get the native volume info
        let ntfs_info = crate::volume::get_volume_info(&volume.mount_point)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        info!(
            volume = %volume.mount_point,
            method = if self.try_mft { "MFT" } else { "recursive" },
            "Starting volume scan"
        );

        let records = if self.try_mft {
            // Try MFT first, fall back to recursive on access denied
            match scan_mft(&ntfs_info, &volume.id, progress.clone()) {
                Ok(records) => records,
                Err(NtfsError::AccessDenied { .. }) => {
                    warn!(
                        volume = %volume.mount_point,
                        "MFT access denied, falling back to recursive scan"
                    );
                    scan_recursive(&ntfs_info, &volume.id, progress)
                        .map_err(|e| anyhow::anyhow!("{}", e))?
                }
                Err(e) => return Err(anyhow::anyhow!("{}", e)),
            }
        } else {
            scan_recursive(&ntfs_info, &volume.id, progress)
                .map_err(|e| anyhow::anyhow!("{}", e))?
        };

        info!(
            volume = %volume.mount_point,
            files = records.iter().filter(|r| !r.is_dir).count(),
            dirs = records.iter().filter(|r| r.is_dir).count(),
            "Scan complete"
        );

        Ok(records)
    }

    fn watch_changes(
        &self,
        volume: VolumeInfo,
        handler: Arc<dyn ChangeHandler>,
    ) -> anyhow::Result<WatchHandle> {
        if !volume.supports_change_journal {
            return Err(anyhow::anyhow!(
                "Volume {} does not support change journal",
                volume.mount_point
            ));
        }

        let device_path = crate::winapi_utils::normalize_volume_path(&volume.mount_point);

        let watcher = UsnWatcher::start(
            device_path,
            volume.id.clone(),
            handler,
            volume.journal_state,
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

        // Create shutdown channel for the watch handle
        let (shutdown_tx, _shutdown_rx) = crossbeam_channel::bounded(1);

        Ok(WatchHandle::new(watcher, shutdown_tx))
    }

    fn get_journal_state(&self, volume: &VolumeInfo) -> anyhow::Result<Option<JournalState>> {
        let device_path = crate::winapi_utils::normalize_volume_path(&volume.mount_point);

        match get_journal_state(&device_path) {
            Ok(state) => Ok(Some(state)),
            Err(NtfsError::UsnJournalNotEnabled { .. }) => Ok(None),
            Err(NtfsError::AccessDenied { .. }) => {
                warn!(
                    volume = %volume.mount_point,
                    "Cannot access USN journal (requires elevation)"
                );
                Ok(None)
            }
            Err(e) => Err(anyhow::anyhow!("{}", e)),
        }
    }

    fn name(&self) -> &'static str {
        "ntfs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_volumes() {
        let backend = NtfsBackend::new();
        let result = backend.list_volumes();

        assert!(result.is_ok(), "Should enumerate volumes: {:?}", result.err());

        let volumes = result.unwrap();
        println!("Found {} volumes", volumes.len());

        for vol in &volumes {
            println!("  {} - {} ({})", vol.mount_point, vol.filesystem_type, vol.id);
        }
    }

    #[test]
    #[ignore] // Requires admin privileges or takes a long time
    fn test_full_scan() {
        let backend = NtfsBackend::new();
        let volumes = backend.list_volumes().unwrap();

        if let Some(c_drive) = volumes.iter().find(|v| v.mount_point.starts_with("C")) {
            let result = backend.full_scan(c_drive, None);

            match result {
                Ok(records) => {
                    println!("Scanned {} records", records.len());
                    for record in records.iter().take(10) {
                        println!("  {}", record.path);
                    }
                }
                Err(e) => {
                    println!("Scan failed (may require admin): {}", e);
                }
            }
        }
    }
}

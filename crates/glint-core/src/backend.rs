//! Filesystem backend traits.
//!
//! This module defines the abstract interface that filesystem-specific backends
//! must implement. The core index and search logic interacts only through these
//! traits, enabling clean separation of platform-specific code.
//!
//! ## Implementing a New Backend
//!
//! To add support for a new filesystem or platform:
//!
//! 1. Create a new crate (e.g., `glint-backend-linux`)
//! 2. Implement `FileSystemBackend` for your filesystem
//! 3. Encapsulate all unsafe code within that crate
//! 4. Register your backend with the Glint core during initialization

use crate::types::{FileId, FileRecord, VolumeId};
use std::fmt;
use std::sync::Arc;

/// Information about a volume/filesystem that can be indexed.
///
/// This is returned by `FileSystemBackend::list_volumes()` and used to
/// identify which volumes to scan and monitor.
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    /// Unique identifier for this volume
    pub id: VolumeId,

    /// Mount point or drive letter (e.g., "C:" on Windows, "/home" on Linux)
    pub mount_point: String,

    /// Human-readable label (e.g., "System", "Data")
    pub label: Option<String>,

    /// Filesystem type (e.g., "NTFS", "ext4", "APFS")
    pub filesystem_type: String,

    /// Total capacity in bytes
    pub total_bytes: Option<u64>,

    /// Free space in bytes
    pub free_bytes: Option<u64>,

    /// Whether this volume supports change notifications
    pub supports_change_journal: bool,

    /// Backend-specific state for USN journal tracking
    /// On NTFS, this stores the last processed USN and journal ID
    pub journal_state: Option<JournalState>,
}

impl VolumeInfo {
    /// Create a new VolumeInfo with required fields
    pub fn new(
        id: VolumeId,
        mount_point: impl Into<String>,
        filesystem_type: impl Into<String>,
    ) -> Self {
        VolumeInfo {
            id,
            mount_point: mount_point.into(),
            label: None,
            filesystem_type: filesystem_type.into(),
            total_bytes: None,
            free_bytes: None,
            supports_change_journal: false,
            journal_state: None,
        }
    }

    /// Set the volume label
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set capacity information
    pub fn with_capacity(mut self, total: u64, free: u64) -> Self {
        self.total_bytes = Some(total);
        self.free_bytes = Some(free);
        self
    }

    /// Mark this volume as supporting change notifications
    pub fn with_change_journal_support(mut self, supported: bool) -> Self {
        self.supports_change_journal = supported;
        self
    }
}

/// State for tracking journal position (used for USN journal on NTFS)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JournalState {
    /// Journal ID (changes if journal is deleted and recreated)
    pub journal_id: u64,

    /// Last processed USN (sequence number)
    pub last_usn: i64,
}

impl JournalState {
    /// Create a new journal state
    pub fn new(journal_id: u64, last_usn: i64) -> Self {
        JournalState {
            journal_id,
            last_usn,
        }
    }
}

/// The kind of change that occurred to a file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// A new file or directory was created
    Created,

    /// A file or directory was deleted
    Deleted,

    /// A file or directory was renamed (includes move operations)
    Renamed,

    /// File contents were modified
    Modified,

    /// File attributes or metadata changed
    AttributeChanged,

    /// Security/permissions changed
    SecurityChanged,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeKind::Created => write!(f, "created"),
            ChangeKind::Deleted => write!(f, "deleted"),
            ChangeKind::Renamed => write!(f, "renamed"),
            ChangeKind::Modified => write!(f, "modified"),
            ChangeKind::AttributeChanged => write!(f, "attribute_changed"),
            ChangeKind::SecurityChanged => write!(f, "security_changed"),
        }
    }
}

/// A filesystem change event
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    /// The kind of change
    pub kind: ChangeKind,

    /// Volume where the change occurred
    pub volume_id: VolumeId,

    /// File ID of the affected file
    pub file_id: FileId,

    /// Parent directory's file ID
    pub parent_id: Option<FileId>,

    /// Current filename (or previous name for deletes)
    pub name: String,

    /// For rename operations, the new name
    pub new_name: Option<String>,

    /// For rename operations, the new parent ID (if moved)
    pub new_parent_id: Option<FileId>,

    /// Whether this is a directory
    pub is_dir: bool,

    /// USN (Update Sequence Number) for NTFS, or other sequence marker
    pub sequence: i64,
}

impl ChangeEvent {
    /// Create a simple create event
    pub fn created(
        volume_id: VolumeId,
        file_id: FileId,
        parent_id: Option<FileId>,
        name: String,
        is_dir: bool,
        sequence: i64,
    ) -> Self {
        ChangeEvent {
            kind: ChangeKind::Created,
            volume_id,
            file_id,
            parent_id,
            name,
            new_name: None,
            new_parent_id: None,
            is_dir,
            sequence,
        }
    }

    /// Create a delete event
    pub fn deleted(
        volume_id: VolumeId,
        file_id: FileId,
        parent_id: Option<FileId>,
        name: String,
        is_dir: bool,
        sequence: i64,
    ) -> Self {
        ChangeEvent {
            kind: ChangeKind::Deleted,
            volume_id,
            file_id,
            parent_id,
            name,
            new_name: None,
            new_parent_id: None,
            is_dir,
            sequence,
        }
    }

    /// Create a rename event
    pub fn renamed(
        volume_id: VolumeId,
        file_id: FileId,
        parent_id: Option<FileId>,
        old_name: String,
        new_name: String,
        new_parent_id: Option<FileId>,
        is_dir: bool,
        sequence: i64,
    ) -> Self {
        ChangeEvent {
            kind: ChangeKind::Renamed,
            volume_id,
            file_id,
            parent_id,
            name: old_name,
            new_name: Some(new_name),
            new_parent_id,
            is_dir,
            sequence,
        }
    }
}

/// Handler for filesystem change events.
///
/// Backends call methods on this handler when filesystem changes occur.
/// The handler is responsible for updating the index accordingly.
pub trait ChangeHandler: Send + Sync {
    /// Called when a filesystem change is detected
    fn on_change(&self, event: ChangeEvent);

    /// Called when the journal is truncated or reset, requiring a full rescan
    fn on_journal_reset(&self, volume_id: VolumeId, reason: String);

    /// Called when an error occurs during monitoring
    fn on_error(&self, volume_id: VolumeId, error: String);
}

/// A channel-based change handler implementation
pub struct ChannelChangeHandler {
    sender: crossbeam_channel::Sender<ChangeHandlerMessage>,
}

/// Messages sent by the change handler
pub enum ChangeHandlerMessage {
    /// A change event
    Change(ChangeEvent),
    /// Journal was reset, need rescan
    JournalReset { volume_id: VolumeId, reason: String },
    /// An error occurred
    Error { volume_id: VolumeId, error: String },
}

impl ChannelChangeHandler {
    /// Create a new channel-based handler
    pub fn new() -> (Self, crossbeam_channel::Receiver<ChangeHandlerMessage>) {
        let (sender, receiver) = crossbeam_channel::unbounded();
        (ChannelChangeHandler { sender }, receiver)
    }
}

impl Default for ChannelChangeHandler {
    fn default() -> Self {
        Self::new().0
    }
}

impl ChangeHandler for ChannelChangeHandler {
    fn on_change(&self, event: ChangeEvent) {
        let _ = self.sender.send(ChangeHandlerMessage::Change(event));
    }

    fn on_journal_reset(&self, volume_id: VolumeId, reason: String) {
        let _ = self
            .sender
            .send(ChangeHandlerMessage::JournalReset { volume_id, reason });
    }

    fn on_error(&self, volume_id: VolumeId, error: String) {
        let _ = self
            .sender
            .send(ChangeHandlerMessage::Error { volume_id, error });
    }
}

/// Abstract trait for filesystem backends.
///
/// Each supported filesystem/platform implements this trait to provide:
/// - Volume enumeration
/// - Full filesystem scanning
/// - Change monitoring
///
/// ## Thread Safety
///
/// Implementations must be `Send + Sync` to allow use from multiple threads.
/// The `watch_changes` method may spawn background threads for monitoring.
///
/// ## Error Handling
///
/// Backends should return `GlintError` variants that allow the caller to
/// determine appropriate recovery actions (e.g., triggering a rescan).
pub trait FileSystemBackend: Send + Sync {
    /// Enumerate all volumes handled by this backend.
    ///
    /// Returns information about all volumes that this backend can index.
    /// On Windows/NTFS, this would return all NTFS drives.
    fn list_volumes(&self) -> anyhow::Result<Vec<VolumeInfo>>;

    /// Perform a full scan of a volume and return all file records.
    ///
    /// This is used for initial indexing and for rescans when the change
    /// journal is unavailable or truncated.
    ///
    /// ## Performance
    ///
    /// This operation should use efficient filesystem APIs (e.g., MFT on NTFS)
    /// rather than recursive directory traversal where possible.
    ///
    /// ## Progress Reporting
    ///
    /// For large volumes, implementations may report progress through the
    /// tracing infrastructure.
    fn full_scan(
        &self,
        volume: &VolumeInfo,
        progress: Option<Arc<dyn ScanProgress>>,
    ) -> anyhow::Result<Vec<FileRecord>>;

    /// Start monitoring a volume for changes.
    ///
    /// This method starts a background monitoring loop that:
    /// - Reads the change journal (USN journal on NTFS)
    /// - Calls the handler for each change detected
    /// - Handles journal truncation by calling `on_journal_reset`
    ///
    /// The monitoring continues until the returned `WatchHandle` is dropped.
    ///
    /// ## Journal State
    ///
    /// If `volume.journal_state` is `Some`, monitoring resumes from that point.
    /// Otherwise, it starts from the current position (missing any changes
    /// that occurred while not monitoring).
    fn watch_changes(
        &self,
        volume: VolumeInfo,
        handler: Arc<dyn ChangeHandler>,
    ) -> anyhow::Result<WatchHandle>;

    /// Get the current journal state for a volume.
    ///
    /// This is used to save the position for later resumption.
    fn get_journal_state(&self, volume: &VolumeInfo) -> anyhow::Result<Option<JournalState>>;

    /// Get the backend name (e.g., "ntfs", "ext4")
    fn name(&self) -> &'static str;
}

/// Handle for a running change watcher.
///
/// When dropped, the watcher is stopped. Implementations should ensure
/// clean shutdown when the handle is dropped.
pub struct WatchHandle {
    /// Internal state, if needed by the backend
    _inner: Box<dyn std::any::Any + Send>,
    /// Channel to signal shutdown
    shutdown: Option<crossbeam_channel::Sender<()>>,
}

impl WatchHandle {
    /// Create a new watch handle
    pub fn new<T: Send + 'static>(inner: T, shutdown: crossbeam_channel::Sender<()>) -> Self {
        WatchHandle {
            _inner: Box::new(inner),
            shutdown: Some(shutdown),
        }
    }

    /// Create a dummy watch handle (for testing)
    pub fn dummy() -> Self {
        WatchHandle {
            _inner: Box::new(()),
            shutdown: None,
        }
    }

    /// Signal the watcher to stop
    pub fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Progress reporting for scan operations
pub trait ScanProgress: Send + Sync {
    /// Called periodically during scanning with the current count
    fn on_progress(&self, files_scanned: u64, dirs_scanned: u64);

    /// Called when scanning is complete
    fn on_complete(&self, total_files: u64, total_dirs: u64);
}

/// A simple progress reporter that logs to tracing
pub struct LoggingProgress {
    volume: String,
}

impl LoggingProgress {
    pub fn new(volume: impl Into<String>) -> Self {
        LoggingProgress {
            volume: volume.into(),
        }
    }
}

impl ScanProgress for LoggingProgress {
    fn on_progress(&self, files_scanned: u64, dirs_scanned: u64) {
        tracing::debug!(
            volume = %self.volume,
            files = files_scanned,
            dirs = dirs_scanned,
            "Scanning progress"
        );
    }

    fn on_complete(&self, total_files: u64, total_dirs: u64) {
        tracing::info!(
            volume = %self.volume,
            files = total_files,
            dirs = total_dirs,
            "Scan complete"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_info() {
        let vol = VolumeInfo::new(VolumeId::new("C"), "C:", "NTFS")
            .with_label("System")
            .with_capacity(500_000_000_000, 100_000_000_000)
            .with_change_journal_support(true);

        assert_eq!(vol.id.as_str(), "C");
        assert_eq!(vol.mount_point, "C:");
        assert_eq!(vol.label, Some("System".to_string()));
        assert!(vol.supports_change_journal);
    }

    #[test]
    fn test_change_event() {
        let event = ChangeEvent::created(
            VolumeId::new("C"),
            FileId::new(100),
            Some(FileId::new(5)),
            "newfile.txt".to_string(),
            false,
            12345,
        );

        assert_eq!(event.kind, ChangeKind::Created);
        assert_eq!(event.name, "newfile.txt");
        assert!(!event.is_dir);
    }

    #[test]
    fn test_channel_handler() {
        let (handler, receiver) = ChannelChangeHandler::new();

        handler.on_change(ChangeEvent::created(
            VolumeId::new("C"),
            FileId::new(1),
            None,
            "test.txt".to_string(),
            false,
            1,
        ));

        let msg = receiver.try_recv().unwrap();
        assert!(matches!(msg, ChangeHandlerMessage::Change(_)));
    }
}

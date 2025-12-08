//! USN Change Journal monitoring for NTFS.
//!
//! The USN (Update Sequence Number) Change Journal is a persistent log of
//! changes to files and directories on an NTFS volume. This module provides
//! functionality to:
//!
//! - Query journal status and position
//! - Read change records from the journal
//! - Monitor for new changes in real-time
//!
//! ## How It Works
//!
//! The USN journal records every change to the filesystem, including:
//! - File/directory creation
//! - Deletion
//! - Rename/move
//! - Data modification
//! - Attribute changes
//!
//! Each record has a unique USN (sequence number) that increases monotonically.
//! By tracking the last processed USN, we can efficiently catch up on changes.
//!
//! ## Permissions
//!
//! Requires elevated privileges (Administrator or "Perform Volume Maintenance Tasks").

use crate::error::NtfsError;
use crate::winapi_utils::{open_volume_for_usn, SafeHandle};
use glint_core::backend::{ChangeEvent, ChangeHandler, ChangeKind, JournalState};
use glint_core::types::{FileId, VolumeId};
use crossbeam_channel::{Receiver, Sender};
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, error, info, warn};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
};

/// USN Journal data returned by FSCTL_QUERY_USN_JOURNAL
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UsnJournalData {
    pub usn_journal_id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
    pub lowest_valid_usn: i64,
    pub max_usn: i64,
    pub maximum_size: u64,
    pub allocation_delta: u64,
    pub min_supported_major_version: u16,
    pub max_supported_major_version: u16,
}

/// Input for FSCTL_READ_USN_JOURNAL
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ReadUsnJournalData {
    start_usn: i64,
    reason_mask: u32,
    return_only_on_close: u32,
    timeout: u64,
    bytes_to_wait_for: u64,
    usn_journal_id: u64,
    min_major_version: u16,
    max_major_version: u16,
}

/// USN record structure (version 2)
#[repr(C)]
#[derive(Debug)]
struct UsnRecordV2 {
    record_length: u32,
    major_version: u16,
    minor_version: u16,
    file_reference_number: u64,
    parent_file_reference_number: u64,
    usn: i64,
    timestamp: i64,
    reason: u32,
    source_info: u32,
    security_id: u32,
    file_attributes: u32,
    file_name_length: u16,
    file_name_offset: u16,
    // file_name follows
}

// USN reason flags
const USN_REASON_DATA_OVERWRITE: u32 = 0x00000001;
const USN_REASON_DATA_EXTEND: u32 = 0x00000002;
const USN_REASON_DATA_TRUNCATION: u32 = 0x00000004;
const USN_REASON_FILE_CREATE: u32 = 0x00000100;
const USN_REASON_FILE_DELETE: u32 = 0x00000200;
const USN_REASON_RENAME_OLD_NAME: u32 = 0x00001000;
const USN_REASON_RENAME_NEW_NAME: u32 = 0x00002000;
const USN_REASON_CLOSE: u32 = 0x80000000;

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// Query the USN journal status for a volume.
pub fn query_usn_journal(device_path: &str) -> Result<UsnJournalData, NtfsError> {
    let handle = open_volume_for_usn(device_path)?;
    query_usn_journal_handle(&handle, device_path)
}

fn query_usn_journal_handle(handle: &SafeHandle, device_path: &str) -> Result<UsnJournalData, NtfsError> {
    let mut journal_data: UsnJournalData = unsafe { mem::zeroed() };
    let mut bytes_returned = 0u32;

    let result = unsafe {
        DeviceIoControl(
            handle.as_raw(),
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(&mut journal_data as *mut _ as *mut _),
            mem::size_of::<UsnJournalData>() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        let error = unsafe { windows::Win32::Foundation::GetLastError().0 };
        if error == 5 {
            return Err(NtfsError::AccessDenied {
                operation: "FSCTL_QUERY_USN_JOURNAL".to_string(),
            });
        }
        // ERROR_JOURNAL_NOT_ACTIVE (1179) or ERROR_JOURNAL_DELETE_IN_PROGRESS (1178)
        if error == 1179 || error == 1178 {
            return Err(NtfsError::UsnJournalNotEnabled {
                volume: device_path.to_string(),
            });
        }
        return Err(NtfsError::from_win32("FSCTL_QUERY_USN_JOURNAL"));
    }

    debug!(
        journal_id = journal_data.usn_journal_id,
        first_usn = journal_data.first_usn,
        next_usn = journal_data.next_usn,
        "Queried USN journal"
    );

    Ok(journal_data)
}

/// Create a JournalState from USN journal data.
pub fn get_journal_state(device_path: &str) -> Result<JournalState, NtfsError> {
    let journal_data = query_usn_journal(device_path)?;
    Ok(JournalState::new(journal_data.usn_journal_id, journal_data.next_usn))
}

/// Read USN records starting from a given USN.
///
/// Returns the records and the next USN to read from.
pub fn read_usn_records(
    handle: &SafeHandle,
    journal_data: &UsnJournalData,
    start_usn: i64,
    volume_id: &VolumeId,
) -> Result<(Vec<ChangeEvent>, i64), NtfsError> {
    const BUFFER_SIZE: usize = 64 * 1024;
    let mut buffer = vec![0u8; BUFFER_SIZE];

    // Reason mask: we want most change types
    let reason_mask = USN_REASON_DATA_OVERWRITE
        | USN_REASON_DATA_EXTEND
        | USN_REASON_DATA_TRUNCATION
        | USN_REASON_FILE_CREATE
        | USN_REASON_FILE_DELETE
        | USN_REASON_RENAME_OLD_NAME
        | USN_REASON_RENAME_NEW_NAME
        | USN_REASON_CLOSE;

    let read_data = ReadUsnJournalData {
        start_usn,
        reason_mask,
        return_only_on_close: 0, // Get all records, not just on file close
        timeout: 0,
        bytes_to_wait_for: 0,
        usn_journal_id: journal_data.usn_journal_id,
        min_major_version: 2,
        max_major_version: 3,
    };

    let mut bytes_returned = 0u32;

    let result = unsafe {
        DeviceIoControl(
            handle.as_raw(),
            FSCTL_READ_USN_JOURNAL,
            Some(&read_data as *const _ as *const _),
            mem::size_of::<ReadUsnJournalData>() as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        let error = unsafe { windows::Win32::Foundation::GetLastError().0 };
        // ERROR_JOURNAL_ENTRY_DELETED (1181) means the start USN is too old
        if error == 1181 {
            return Err(NtfsError::UsnJournalTruncated {
                volume: volume_id.as_str().to_string(),
            });
        }
        return Err(NtfsError::from_win32("FSCTL_READ_USN_JOURNAL"));
    }

    if bytes_returned < 8 {
        // Just the next USN, no records
        let next_usn = if bytes_returned >= 8 {
            i64::from_ne_bytes(buffer[0..8].try_into().unwrap())
        } else {
            start_usn
        };
        return Ok((Vec::new(), next_usn));
    }

    // First 8 bytes are the next USN
    let next_usn = i64::from_ne_bytes(buffer[0..8].try_into().unwrap());

    // Parse records
    let mut events = Vec::new();
    let mut offset = 8usize;

    while offset + mem::size_of::<UsnRecordV2>() <= bytes_returned as usize {
        let record_ptr = buffer.as_ptr().wrapping_add(offset) as *const UsnRecordV2;
        let record = unsafe { &*record_ptr };

        if record.record_length == 0 {
            break;
        }

        // Extract filename
        let name_offset = record.file_name_offset as usize;
        let name_len = record.file_name_length as usize;

        if offset + name_offset + name_len <= bytes_returned as usize {
            let name_ptr = buffer.as_ptr().wrapping_add(offset + name_offset) as *const u16;
            let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len / 2) };
            let name = String::from_utf16_lossy(name_slice);

            // Skip system files
            if !name.starts_with('$') {
                let event = parse_usn_record(record, name, volume_id);
                if let Some(e) = event {
                    events.push(e);
                }
            }
        }

        offset += record.record_length as usize;
    }

    Ok((events, next_usn))
}

/// Parse a USN record into a ChangeEvent.
fn parse_usn_record(record: &UsnRecordV2, name: String, volume_id: &VolumeId) -> Option<ChangeEvent> {
    let file_id = FileId::new(record.file_reference_number & 0x0000FFFFFFFFFFFF);
    let parent_id = {
        let pid = record.parent_file_reference_number & 0x0000FFFFFFFFFFFF;
        if pid == 0 {
            None
        } else {
            Some(FileId::new(pid))
        }
    };
    let is_dir = (record.file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;

    // Determine the change kind based on reason flags
    // We only process certain combinations to avoid duplicate events

    let kind = if record.reason & USN_REASON_FILE_DELETE != 0 {
        // File was deleted
        Some(ChangeKind::Deleted)
    } else if record.reason & USN_REASON_FILE_CREATE != 0 && record.reason & USN_REASON_CLOSE != 0 {
        // File was created and closed - this is a completed creation
        Some(ChangeKind::Created)
    } else if record.reason & USN_REASON_RENAME_NEW_NAME != 0 && record.reason & USN_REASON_CLOSE != 0 {
        // File was renamed and closed
        Some(ChangeKind::Renamed)
    } else if (record.reason & (USN_REASON_DATA_OVERWRITE | USN_REASON_DATA_EXTEND | USN_REASON_DATA_TRUNCATION) != 0)
        && record.reason & USN_REASON_CLOSE != 0
    {
        // Data was modified and file closed
        Some(ChangeKind::Modified)
    } else {
        None
    };

    kind.map(|k| match k {
        ChangeKind::Created => ChangeEvent::created(
            volume_id.clone(),
            file_id,
            parent_id,
            name,
            is_dir,
            record.usn,
        ),
        ChangeKind::Deleted => ChangeEvent::deleted(
            volume_id.clone(),
            file_id,
            parent_id,
            name,
            is_dir,
            record.usn,
        ),
        ChangeKind::Renamed => ChangeEvent::renamed(
            volume_id.clone(),
            file_id,
            parent_id,
            String::new(), // Old name not available in single record
            name,
            parent_id,
            is_dir,
            record.usn,
        ),
        ChangeKind::Modified => ChangeEvent {
            kind: ChangeKind::Modified,
            volume_id: volume_id.clone(),
            file_id,
            parent_id,
            name,
            new_name: None,
            new_parent_id: None,
            is_dir,
            sequence: record.usn,
        },
        _ => unreachable!(),
    })
}

/// USN journal watcher that monitors for changes.
pub struct UsnWatcher {
    /// Thread handle for the watcher
    thread: Option<JoinHandle<()>>,
    /// Signal to stop the watcher
    stop_signal: Arc<AtomicBool>,
    /// Shutdown sender
    shutdown_tx: Sender<()>,
}

impl UsnWatcher {
    /// Start watching a volume for changes.
    pub fn start(
        device_path: String,
        volume_id: VolumeId,
        handler: Arc<dyn ChangeHandler>,
        initial_state: Option<JournalState>,
    ) -> Result<Self, NtfsError> {
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);

        let thread = thread::Builder::new()
            .name(format!("usn-watcher-{}", volume_id))
            .spawn(move || {
                watch_loop(
                    device_path,
                    volume_id,
                    handler,
                    initial_state,
                    stop_signal_clone,
                    shutdown_rx,
                );
            })
            .map_err(|e| NtfsError::Io(e.into()))?;

        Ok(UsnWatcher {
            thread: Some(thread),
            stop_signal,
            shutdown_tx,
        })
    }

    /// Stop the watcher.
    pub fn stop(&mut self) {
        self.stop_signal.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(());

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for UsnWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Main watch loop that polls for USN changes.
fn watch_loop(
    device_path: String,
    volume_id: VolumeId,
    handler: Arc<dyn ChangeHandler>,
    initial_state: Option<JournalState>,
    stop_signal: Arc<AtomicBool>,
    shutdown_rx: Receiver<()>,
) {
    info!(volume = %volume_id, "Starting USN watcher");

    // Open volume handle
    let handle = match open_volume_for_usn(&device_path) {
        Ok(h) => h,
        Err(e) => {
            error!(volume = %volume_id, error = %e, "Failed to open volume for USN watching");
            handler.on_error(volume_id, format!("Failed to open volume: {}", e));
            return;
        }
    };

    // Query journal
    let journal_data = match query_usn_journal_handle(&handle, &device_path) {
        Ok(data) => data,
        Err(e) => {
            error!(volume = %volume_id, error = %e, "Failed to query USN journal");
            handler.on_error(volume_id, format!("Failed to query journal: {}", e));
            return;
        }
    };

    // Determine starting USN
    let mut current_usn = match initial_state {
        Some(state) => {
            // Check if journal ID matches
            if state.journal_id != journal_data.usn_journal_id {
                warn!(
                    volume = %volume_id,
                    old_id = state.journal_id,
                    new_id = journal_data.usn_journal_id,
                    "Journal ID changed, rescan required"
                );
                handler.on_journal_reset(
                    volume_id.clone(),
                    "Journal ID changed".to_string(),
                );
                // Start from current position
                journal_data.next_usn
            } else if state.last_usn < journal_data.first_usn {
                warn!(
                    volume = %volume_id,
                    last = state.last_usn,
                    first_valid = journal_data.first_usn,
                    "Journal truncated, rescan required"
                );
                handler.on_journal_reset(
                    volume_id.clone(),
                    "Journal truncated, some changes may have been missed".to_string(),
                );
                // Start from current position
                journal_data.next_usn
            } else {
                state.last_usn
            }
        }
        None => {
            // Start from current position
            journal_data.next_usn
        }
    };

    debug!(volume = %volume_id, start_usn = current_usn, "USN watcher starting from");

    // Poll loop
    let poll_interval = Duration::from_millis(500);

    while !stop_signal.load(Ordering::Acquire) {
        // Check for shutdown signal
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        // Read new records
        match read_usn_records(&handle, &journal_data, current_usn, &volume_id) {
            Ok((events, next_usn)) => {
                for event in events {
                    debug!(
                        kind = %event.kind,
                        file = %event.name,
                        usn = event.sequence,
                        "USN change event"
                    );
                    handler.on_change(event);
                }
                current_usn = next_usn;
            }
            Err(NtfsError::UsnJournalTruncated { .. }) => {
                warn!(volume = %volume_id, "Journal truncated during watch");
                handler.on_journal_reset(
                    volume_id.clone(),
                    "Journal truncated".to_string(),
                );
                break;
            }
            Err(e) => {
                error!(volume = %volume_id, error = %e, "Error reading USN journal");
                // Don't exit on transient errors
                thread::sleep(poll_interval * 2);
            }
        }

        thread::sleep(poll_interval);
    }

    info!(volume = %volume_id, "USN watcher stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires admin privileges
    fn test_query_journal() {
        let result = query_usn_journal("\\\\.\\C:");

        match result {
            Ok(data) => {
                println!("Journal ID: {:016X}", data.usn_journal_id);
                println!("First USN: {}", data.first_usn);
                println!("Next USN: {}", data.next_usn);
            }
            Err(NtfsError::AccessDenied { .. }) => {
                println!("Test skipped: requires administrator privileges");
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }
}

//! MFT (Master File Table) enumeration for NTFS.
//!
//! This module provides functionality to enumerate all files on an NTFS volume
//! by reading the MFT directly. This is much faster than recursive directory
//! traversal for initial indexing.
//!
//! ## How It Works
//!
//! The MFT is a special file on NTFS volumes that contains a record for every
//! file and directory. By reading the MFT directly, we can enumerate millions
//! of files in seconds rather than minutes.
//!
//! ## Permissions
//!
//! Reading the MFT requires elevated privileges. The process must be running
//! as Administrator or have the "Perform Volume Maintenance Tasks" privilege.
//!
//! ## Fallback
//!
//! If MFT access is denied, we fall back to using the USN journal's enumeration
//! capabilities or recursive directory traversal.

use crate::error::NtfsError;
use crate::volume::NtfsVolumeInfo;
use crate::winapi_utils::{filetime_to_datetime, open_volume, SafeHandle};
use glint_core::backend::ScanProgress;
use glint_core::types::{FileId, FileRecord, VolumeId};
use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use tracing::{debug, info, warn};
use windows::Win32::System::Ioctl::{FSCTL_ENUM_USN_DATA, FSCTL_GET_NTFS_VOLUME_DATA};
use windows::Win32::System::IO::DeviceIoControl;

/// USN record header for reading MFT data
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct MftEnumData {
    start_file_reference_number: u64,
    low_usn: i64,
    high_usn: i64,
    min_major_version: u16,
    max_major_version: u16,
}

/// NTFS volume data returned by FSCTL_GET_NTFS_VOLUME_DATA
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct NtfsVolumeData {
    volume_serial_number: u64,
    number_sectors: u64,
    total_clusters: u64,
    free_clusters: u64,
    total_reserved: u64,
    bytes_per_sector: u32,
    bytes_per_cluster: u32,
    bytes_per_file_record_segment: u32,
    clusters_per_file_record_segment: u32,
    mft_valid_data_length: u64,
    mft_start_lcn: u64,
    mft2_start_lcn: u64,
    mft_zone_start: u64,
    mft_zone_end: u64,
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
    // file_name follows (variable length UTF-16)
}

/// USN record structure (version 3) - uses 128-bit file IDs
#[repr(C)]
#[derive(Debug)]
struct UsnRecordV3 {
    record_length: u32,
    major_version: u16,
    minor_version: u16,
    file_reference_number: [u8; 16],        // FILE_ID_128
    parent_file_reference_number: [u8; 16], // FILE_ID_128
    usn: i64,
    timestamp: i64,
    reason: u32,
    source_info: u32,
    security_id: u32,
    file_attributes: u32,
    file_name_length: u16,
    file_name_offset: u16,
    // file_name follows (variable length UTF-16)
}

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// Scan an NTFS volume by reading the MFT.
///
/// Returns all file records found on the volume.
pub fn scan_mft(
    volume_info: &NtfsVolumeInfo,
    volume_id: &VolumeId,
    progress: Option<Arc<dyn ScanProgress>>,
) -> Result<Vec<FileRecord>, NtfsError> {
    let device_path = volume_info.device_path();
    info!(volume = %device_path, "Starting MFT scan");

    let handle = open_volume(&device_path)?;

    // Get NTFS volume data to understand MFT structure
    let _vol_data = get_ntfs_volume_data(&handle)?;

    // Enumerate all files using FSCTL_ENUM_USN_DATA
    let records = enumerate_usn_records(&handle, volume_info, volume_id, progress)?;

    Ok(records)
}

/// Get NTFS volume data.
fn get_ntfs_volume_data(handle: &SafeHandle) -> Result<NtfsVolumeData, NtfsError> {
    let mut vol_data: NtfsVolumeData = unsafe { mem::zeroed() };
    let mut bytes_returned = 0u32;

    let result = unsafe {
        DeviceIoControl(
            handle.as_raw(),
            FSCTL_GET_NTFS_VOLUME_DATA,
            None,
            0,
            Some(&mut vol_data as *mut _ as *mut _),
            mem::size_of::<NtfsVolumeData>() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        return Err(NtfsError::from_win32("FSCTL_GET_NTFS_VOLUME_DATA"));
    }

    debug!(
        mft_size = vol_data.mft_valid_data_length,
        bytes_per_record = vol_data.bytes_per_file_record_segment,
        "Got NTFS volume data"
    );

    Ok(vol_data)
}

/// Enumerate files using FSCTL_ENUM_USN_DATA.
///
/// This is the primary enumeration method. It reads through the MFT
/// using the USN journal infrastructure, which is efficient and works
/// on all NTFS volumes.
fn enumerate_usn_records(
    handle: &SafeHandle,
    volume_info: &NtfsVolumeInfo,
    volume_id: &VolumeId,
    progress: Option<Arc<dyn ScanProgress>>,
) -> Result<Vec<FileRecord>, NtfsError> {
    // Buffer for USN records
    const BUFFER_SIZE: usize = 64 * 1024;
    let mut buffer = vec![0u8; BUFFER_SIZE];

    // Enumeration input data
    let mut enum_data = MftEnumData {
        start_file_reference_number: 0,
        low_usn: 0,
        high_usn: i64::MAX,
        min_major_version: 2,
        max_major_version: 3,
    };

    // Store raw records first, then build paths
    let mut raw_records: Vec<RawFileRecord> = Vec::with_capacity(100_000);

    let mut files_scanned = 0u64;
    let mut dirs_scanned = 0u64;
    let mut last_progress_report = 0u64;

    info!(volume = %volume_info.mount_point, "Enumerating MFT records");

    loop {
        let mut bytes_returned = 0u32;

        let result = unsafe {
            DeviceIoControl(
                handle.as_raw(),
                FSCTL_ENUM_USN_DATA,
                Some(&enum_data as *const _ as *const _),
                mem::size_of::<MftEnumData>() as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                buffer.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if result.is_err() {
            // ERROR_HANDLE_EOF (38) means we've reached the end
            let error = unsafe { windows::Win32::Foundation::GetLastError().0 };
            if error == 38 {
                break;
            }
            // Check for access denied
            if error == 5 {
                return Err(NtfsError::AccessDenied {
                    operation: "FSCTL_ENUM_USN_DATA".to_string(),
                });
            }
            return Err(NtfsError::from_win32("FSCTL_ENUM_USN_DATA"));
        }

        if bytes_returned < 8 {
            break;
        }

        // First 8 bytes are the next file reference number
        let next_ref = u64::from_ne_bytes(buffer[0..8].try_into().unwrap());

        // Parse USN records from the buffer
        let mut offset = 8usize;
        while offset + 8 <= bytes_returned as usize {
            // At least need record_length + major_version
            // Peek at the record length and version
            let record_length = u32::from_ne_bytes(buffer[offset..offset + 4].try_into().unwrap());
            let major_version =
                u16::from_ne_bytes(buffer[offset + 4..offset + 6].try_into().unwrap());

            if record_length == 0 || offset + record_length as usize > bytes_returned as usize {
                break;
            }

            // Parse based on version
            let (file_ref, parent_ref, timestamp, file_attrs, name_offset, name_len) =
                if major_version == 2 {
                    if offset + mem::size_of::<UsnRecordV2>() > bytes_returned as usize {
                        break;
                    }
                    let record =
                        unsafe { &*(buffer.as_ptr().wrapping_add(offset) as *const UsnRecordV2) };
                    (
                        record.file_reference_number,
                        record.parent_file_reference_number,
                        record.timestamp,
                        record.file_attributes,
                        record.file_name_offset as usize,
                        record.file_name_length as usize,
                    )
                } else if major_version == 3 {
                    if offset + mem::size_of::<UsnRecordV3>() > bytes_returned as usize {
                        break;
                    }
                    let record =
                        unsafe { &*(buffer.as_ptr().wrapping_add(offset) as *const UsnRecordV3) };
                    // FILE_ID_128: use lower 64 bits for compatibility
                    let file_ref =
                        u64::from_ne_bytes(record.file_reference_number[0..8].try_into().unwrap());
                    let parent_ref = u64::from_ne_bytes(
                        record.parent_file_reference_number[0..8]
                            .try_into()
                            .unwrap(),
                    );
                    (
                        file_ref,
                        parent_ref,
                        record.timestamp,
                        record.file_attributes,
                        record.file_name_offset as usize,
                        record.file_name_length as usize,
                    )
                } else {
                    // Skip unknown versions
                    offset += record_length as usize;
                    continue;
                };

            // Debug: dump raw record info for first few
            if raw_records.len() < 5 {
                debug!(
                    record_length = record_length,
                    major_version = major_version,
                    file_ref = file_ref,
                    parent_ref = parent_ref,
                    name_offset = name_offset,
                    name_len = name_len,
                    file_attrs = file_attrs,
                    "Raw USN record fields"
                );
            }

            if name_len > 0 && offset + name_offset + name_len <= bytes_returned as usize {
                let name_ptr = buffer.as_ptr().wrapping_add(offset + name_offset) as *const u16;
                let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len / 2) };
                let name = String::from_utf16_lossy(name_slice);

                // Extract file ID (lower 48 bits of reference number)
                let file_id = FileId::new(file_ref & 0x0000FFFFFFFFFFFF);
                let parent_id = parent_ref & 0x0000FFFFFFFFFFFF;

                let is_dir = (file_attrs & FILE_ATTRIBUTE_DIRECTORY) != 0;

                // Debug: log first few records to see what we're getting
                if raw_records.len() < 10 {
                    debug!(
                        name = %name,
                        file_id = file_id.as_u64(),
                        parent_id = parent_id,
                        attrs = file_attrs,
                        is_dir = is_dir,
                        "Sample MFT record"
                    );
                }

                raw_records.push(RawFileRecord {
                    file_id,
                    parent_id: if parent_id == 0 {
                        None
                    } else {
                        Some(FileId::new(parent_id))
                    },
                    name,
                    is_dir,
                    timestamp,
                });

                if is_dir {
                    dirs_scanned += 1;
                } else {
                    files_scanned += 1;
                }

                // Report progress periodically
                if let Some(ref p) = progress {
                    let total = files_scanned + dirs_scanned;
                    if total - last_progress_report >= 10000 {
                        p.on_progress(files_scanned, dirs_scanned);
                        last_progress_report = total;
                    }
                }
            }

            offset += record_length as usize;
        }

        // Update starting point for next iteration
        enum_data.start_file_reference_number = next_ref;
    }

    info!(
        files = files_scanned,
        dirs = dirs_scanned,
        "MFT enumeration complete, building paths"
    );

    // Build full paths
    let records = build_paths(raw_records, volume_id, &volume_info.mount_point);

    if let Some(ref p) = progress {
        p.on_complete(files_scanned, dirs_scanned);
    }

    Ok(records)
}

/// Intermediate structure for raw MFT data before path building
struct RawFileRecord {
    file_id: FileId,
    parent_id: Option<FileId>,
    name: String,
    is_dir: bool,
    timestamp: i64,
}

/// Build full paths from raw records.
///
/// This uses the parent-child relationships to construct full paths
/// for all files.
fn build_paths(
    raw_records: Vec<RawFileRecord>,
    volume_id: &VolumeId,
    mount_point: &str,
) -> Vec<FileRecord> {
    let total_raw = raw_records.len();

    // Build a map from file ID to record index
    let mut id_to_index: HashMap<u64, usize> = HashMap::with_capacity(raw_records.len());
    for (i, record) in raw_records.iter().enumerate() {
        id_to_index.insert(record.file_id.as_u64(), i);
    }

    // Count how many have $ prefix or are empty
    let mut dollar_count = 0;
    let mut empty_count = 0;
    for r in &raw_records {
        if r.name.is_empty() {
            empty_count += 1;
        } else if r.name.starts_with('$') {
            dollar_count += 1;
        }
    }
    debug!(
        total = total_raw,
        dollar_prefix = dollar_count,
        empty_names = empty_count,
        "Raw records before filtering"
    );

    // Build paths for all records
    let mut result = Vec::with_capacity(raw_records.len());

    for raw in &raw_records {
        // Skip system files with empty names or special names
        if raw.name.is_empty() || raw.name.starts_with('$') || raw.name == "." || raw.name == ".." {
            continue;
        }

        // Build the path by walking up the tree
        let path = build_single_path(&raw_records, &id_to_index, raw, mount_point);

        let record = FileRecord::new(
            raw.file_id,
            raw.parent_id,
            volume_id.clone(),
            raw.name.clone(),
            path,
            raw.is_dir,
        )
        .with_modified(filetime_to_datetime(raw.timestamp));

        result.push(record);
    }

    info!(
        raw_count = total_raw,
        filtered_count = result.len(),
        "Path building complete"
    );

    result
}

/// Build a path for a single record.
fn build_single_path(
    records: &[RawFileRecord],
    id_to_index: &HashMap<u64, usize>,
    record: &RawFileRecord,
    mount_point: &str,
) -> String {
    let mut path_parts = vec![record.name.clone()];
    let mut current_parent = record.parent_id;

    // Walk up the tree (with loop detection)
    let mut depth = 0;
    const MAX_DEPTH: usize = 256;

    while let Some(parent_id) = current_parent {
        if depth >= MAX_DEPTH {
            warn!(file = %record.name, "Path depth exceeded maximum, possible loop");
            break;
        }

        if let Some(&idx) = id_to_index.get(&parent_id.as_u64()) {
            let parent = &records[idx];
            if !parent.name.is_empty() && !parent.name.starts_with('$') && parent.name != "." {
                path_parts.push(parent.name.clone());
            }
            current_parent = parent.parent_id;
        } else {
            break;
        }

        depth += 1;
    }

    // Reverse to get root-to-file order
    path_parts.reverse();

    // Build the full path
    let mount = mount_point.trim_end_matches('\\');
    format!("{}\\{}", mount, path_parts.join("\\"))
}

/// Fallback: scan using recursive directory enumeration.
///
/// This is used when MFT access is denied.
pub fn scan_recursive(
    volume_info: &NtfsVolumeInfo,
    volume_id: &VolumeId,
    progress: Option<Arc<dyn ScanProgress>>,
) -> Result<Vec<FileRecord>, NtfsError> {
    use std::fs;

    info!(
        volume = %volume_info.mount_point,
        "Falling back to recursive directory scan"
    );

    let mut records = Vec::new();
    let mut file_id_counter = 1000u64; // Start after reserved MFT entries
    let mut files_scanned = 0u64;
    let mut dirs_scanned = 0u64;

    let root = &volume_info.mount_point;
    let mut stack = vec![root.to_string()];

    while let Some(dir_path) = stack.pop() {
        let entries = match fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(e) => {
                debug!(path = %dir_path, error = %e, "Failed to read directory");
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let is_dir = metadata.is_dir();
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip system files
            if name.starts_with('$') {
                continue;
            }

            let file_id = FileId::new(file_id_counter);
            file_id_counter += 1;

            let mut record = FileRecord::new(
                file_id,
                None, // We don't track parent IDs in fallback mode
                volume_id.clone(),
                name,
                path_str,
                is_dir,
            );

            if !is_dir {
                record = record.with_size(metadata.len());
            }

            if let Ok(modified) = metadata.modified() {
                record = record.with_modified(chrono::DateTime::from(modified));
            }

            records.push(record);

            if is_dir {
                dirs_scanned += 1;
                stack.push(path.to_string_lossy().to_string());
            } else {
                files_scanned += 1;
            }

            // Report progress
            if let Some(ref p) = progress {
                if (files_scanned + dirs_scanned) % 10000 == 0 {
                    p.on_progress(files_scanned, dirs_scanned);
                }
            }
        }
    }

    if let Some(ref p) = progress {
        p.on_complete(files_scanned, dirs_scanned);
    }

    info!(
        files = files_scanned,
        dirs = dirs_scanned,
        "Recursive scan complete"
    );

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require administrative privileges to run successfully

    #[test]
    #[ignore] // Requires admin privileges
    fn test_scan_c_drive() {
        use crate::volume::get_volume_info;

        let vol_info = get_volume_info("C:").unwrap();
        let volume_id = VolumeId::new(format!("{:08X}", vol_info.serial_number));

        let result = scan_mft(&vol_info, &volume_id, None);

        match result {
            Ok(records) => {
                println!("Found {} records", records.len());
                assert!(!records.is_empty());

                // Print some sample records
                for record in records.iter().take(10) {
                    println!(
                        "  {} ({})",
                        record.path,
                        if record.is_dir { "dir" } else { "file" }
                    );
                }
            }
            Err(NtfsError::AccessDenied { .. }) => {
                println!("Test skipped: requires administrator privileges");
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }
}

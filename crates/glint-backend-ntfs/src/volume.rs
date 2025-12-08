//! Volume enumeration for Windows NTFS.
//!
//! This module provides functionality to discover and query NTFS volumes
//! on the system.

use crate::error::NtfsError;
use crate::winapi_utils::{normalize_volume_path, to_wide_string};
use glint_core::backend::VolumeInfo;
use glint_core::types::VolumeId;
use std::mem::MaybeUninit;
use tracing::{debug, warn};
use windows::core::PCWSTR;
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDiskFreeSpaceExW, GetDriveTypeW,
    GetVolumeInformationW, GetVolumePathNamesForVolumeNameW,
};

// DRIVE_FIXED constant value (3)
const DRIVE_FIXED: u32 = 3;

/// Information about an NTFS volume.
#[derive(Debug, Clone)]
pub struct NtfsVolumeInfo {
    /// Volume GUID path (e.g., "\\?\Volume{guid}\")
    pub volume_guid: String,

    /// Mount point (e.g., "C:\")
    pub mount_point: String,

    /// Volume label
    pub label: Option<String>,

    /// Volume serial number (used as VolumeId)
    pub serial_number: u32,

    /// Filesystem name (should be "NTFS")
    pub filesystem: String,

    /// Total capacity in bytes
    pub total_bytes: u64,

    /// Free space in bytes
    pub free_bytes: u64,
}

impl NtfsVolumeInfo {
    /// Convert to the generic VolumeInfo type.
    pub fn to_volume_info(&self) -> VolumeInfo {
        let id = VolumeId::new(format!("{:08X}", self.serial_number));

        let mut info = VolumeInfo::new(id, &self.mount_point, &self.filesystem)
            .with_capacity(self.total_bytes, self.free_bytes)
            .with_change_journal_support(self.filesystem == "NTFS");

        if let Some(ref label) = self.label {
            info = info.with_label(label);
        }

        info
    }

    /// Get the device path for this volume (e.g., "\\.\C:")
    pub fn device_path(&self) -> String {
        normalize_volume_path(&self.mount_point)
    }
}

/// Enumerate all NTFS volumes on the system.
///
/// Returns information about all fixed NTFS drives.
pub fn enumerate_ntfs_volumes() -> Result<Vec<NtfsVolumeInfo>, NtfsError> {
    let mut volumes = Vec::new();

    // Buffer for volume GUID path
    let mut volume_name = [0u16; MAX_PATH as usize];

    // Find first volume
    let find_handle = unsafe { FindFirstVolumeW(&mut volume_name) };

    let find_handle = match find_handle {
        Ok(h) => h,
        Err(_) => return Err(NtfsError::from_win32("FindFirstVolumeW")),
    };

    loop {
        // Convert volume GUID to string
        let volume_guid = String::from_utf16_lossy(
            &volume_name[..volume_name
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(volume_name.len())],
        );

        // Get mount points for this volume
        if let Some(mount_point) = get_volume_mount_point(&volume_guid) {
            // Check if it's a fixed drive
            if is_fixed_drive(&mount_point) {
                // Get volume information
                if let Some(vol_info) = get_volume_details(&volume_guid, &mount_point) {
                    if vol_info.filesystem == "NTFS" {
                        debug!(
                            mount_point = %vol_info.mount_point,
                            label = ?vol_info.label,
                            serial = %format!("{:08X}", vol_info.serial_number),
                            "Found NTFS volume"
                        );
                        volumes.push(vol_info);
                    }
                }
            }
        }

        // Find next volume
        volume_name = [0u16; MAX_PATH as usize];
        let result = unsafe { FindNextVolumeW(find_handle, &mut volume_name) };

        if result.is_err() {
            break;
        }
    }

    // Close the search handle
    unsafe {
        let _ = FindVolumeClose(find_handle);
    }

    Ok(volumes)
}

/// Get the first mount point for a volume GUID.
fn get_volume_mount_point(volume_guid: &str) -> Option<String> {
    let wide_guid = to_wide_string(volume_guid);
    let mut path_names = [0u16; MAX_PATH as usize];
    let mut return_length = 0u32;

    let result = unsafe {
        GetVolumePathNamesForVolumeNameW(
            PCWSTR(wide_guid.as_ptr()),
            Some(&mut path_names),
            &mut return_length,
        )
    };

    if result.is_err() {
        return None;
    }

    // Path names are null-terminated, multi-string format
    // Get the first path
    let first_null = path_names.iter().position(|&c| c == 0)?;
    if first_null == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(&path_names[..first_null]))
}

/// Check if a path is on a fixed drive.
fn is_fixed_drive(path: &str) -> bool {
    let wide_path = to_wide_string(path);
    let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide_path.as_ptr())) };
    drive_type == DRIVE_FIXED
}

/// Get detailed information about a volume.
fn get_volume_details(volume_guid: &str, mount_point: &str) -> Option<NtfsVolumeInfo> {
    let wide_path = to_wide_string(mount_point);

    // Get volume information
    let mut volume_name = [0u16; MAX_PATH as usize];
    let mut serial_number = 0u32;
    let mut max_component_length = 0u32;
    let mut fs_flags = 0u32;
    let mut fs_name = [0u16; MAX_PATH as usize];

    let result = unsafe {
        GetVolumeInformationW(
            PCWSTR(wide_path.as_ptr()),
            Some(&mut volume_name),
            Some(&mut serial_number),
            Some(&mut max_component_length),
            Some(&mut fs_flags),
            Some(&mut fs_name),
        )
    };

    if result.is_err() {
        warn!(mount_point = %mount_point, "Failed to get volume information");
        return None;
    }

    // Get disk space
    let mut total_bytes = MaybeUninit::<u64>::uninit();
    let mut free_bytes = MaybeUninit::<u64>::uninit();

    let space_result = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide_path.as_ptr()),
            None,
            Some(total_bytes.as_mut_ptr()),
            Some(free_bytes.as_mut_ptr()),
        )
    };

    let (total, free) = if space_result.is_ok() {
        unsafe { (total_bytes.assume_init(), free_bytes.assume_init()) }
    } else {
        (0, 0)
    };

    // Convert strings
    let label = {
        let len = volume_name.iter().position(|&c| c == 0).unwrap_or(0);
        if len > 0 {
            Some(String::from_utf16_lossy(&volume_name[..len]))
        } else {
            None
        }
    };

    let filesystem = {
        let len = fs_name.iter().position(|&c| c == 0).unwrap_or(0);
        String::from_utf16_lossy(&fs_name[..len])
    };

    Some(NtfsVolumeInfo {
        volume_guid: volume_guid.to_string(),
        mount_point: mount_point.to_string(),
        label,
        serial_number,
        filesystem,
        total_bytes: total,
        free_bytes: free,
    })
}

/// Get volume information for a specific mount point.
pub fn get_volume_info(mount_point: &str) -> Result<NtfsVolumeInfo, NtfsError> {
    // Normalize the mount point
    let mount_point = if mount_point.ends_with('\\') {
        mount_point.to_string()
    } else {
        format!("{}\\", mount_point)
    };

    get_volume_details("", &mount_point).ok_or_else(|| NtfsError::VolumeOpen {
        volume: mount_point,
        reason: "Failed to get volume information".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_volumes() {
        // This test requires running on Windows with NTFS volumes
        let result = enumerate_ntfs_volumes();
        assert!(result.is_ok(), "Should enumerate volumes without error");

        let volumes = result.unwrap();
        println!("Found {} NTFS volumes", volumes.len());

        for vol in &volumes {
            println!(
                "  {} ({}) - {} bytes free",
                vol.mount_point,
                vol.label.as_deref().unwrap_or("No label"),
                vol.free_bytes
            );
        }
    }
}

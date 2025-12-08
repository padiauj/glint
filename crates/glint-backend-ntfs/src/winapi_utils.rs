//! Low-level Windows API utilities.
//!
//! This module contains helper functions for working with Windows APIs.
//! All unsafe code for Windows API calls is concentrated here.

use crate::error::NtfsError;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// RAII wrapper for Windows HANDLE.
///
/// Automatically closes the handle when dropped.
pub struct SafeHandle(pub HANDLE);

impl SafeHandle {
    /// Create a new SafeHandle, returning an error if the handle is invalid.
    pub fn new(handle: HANDLE) -> Result<Self, NtfsError> {
        if handle == INVALID_HANDLE_VALUE || handle.0 == ptr::null_mut() {
            Err(NtfsError::from_win32("CreateFile"))
        } else {
            Ok(SafeHandle(handle))
        }
    }

    /// Get the raw handle value.
    pub fn as_raw(&self) -> HANDLE {
        self.0
    }

    /// Check if the handle is valid.
    pub fn is_valid(&self) -> bool {
        self.0 != INVALID_HANDLE_VALUE && self.0 .0 != ptr::null_mut()
    }
}

impl Drop for SafeHandle {
    fn drop(&mut self) {
        if self.is_valid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

// Note: SafeHandle is !Clone and !Copy by default since it doesn't derive them

/// Convert a Rust string to a null-terminated wide string (UTF-16).
pub fn to_wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Open a volume for direct access.
///
/// This opens the volume with read access for querying filesystem data.
///
/// # Safety
///
/// This function uses unsafe Windows API calls but is itself safe as it
/// properly handles the returned handle.
pub fn open_volume(volume_path: &str) -> Result<SafeHandle, NtfsError> {
    let wide_path = to_wide_string(volume_path);

    // SAFETY: We're calling a well-documented Windows API function with valid parameters.
    // The resulting handle is wrapped in SafeHandle for proper cleanup.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };

    match handle {
        Ok(h) => SafeHandle::new(h),
        Err(_) => Err(NtfsError::from_win32("CreateFileW")),
    }
}

/// Open a volume for reading change journals (requires elevated privileges).
pub fn open_volume_for_usn(volume_path: &str) -> Result<SafeHandle, NtfsError> {
    let wide_path = to_wide_string(volume_path);

    // SAFETY: Standard Windows API call with proper parameter handling.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };

    match handle {
        Ok(h) => SafeHandle::new(h),
        Err(_) => Err(NtfsError::from_win32("CreateFileW (USN)")),
    }
}

/// Convert a FILETIME value to a chrono DateTime.
pub fn filetime_to_datetime(ft: i64) -> chrono::DateTime<chrono::Utc> {
    use chrono::{TimeZone, Utc};

    // FILETIME is 100-nanosecond intervals since January 1, 1601
    // Convert to Unix timestamp (seconds since January 1, 1970)
    const FILETIME_UNIX_DIFF: i64 = 116444736000000000;
    const TICKS_PER_SECOND: i64 = 10_000_000;

    let unix_ticks = ft - FILETIME_UNIX_DIFF;
    let seconds = unix_ticks / TICKS_PER_SECOND;
    let nanos = ((unix_ticks % TICKS_PER_SECOND) * 100) as u32;

    Utc.timestamp_opt(seconds, nanos)
        .single()
        .unwrap_or_else(Utc::now)
}

/// Get the drive letter from a volume path like "\\?\C:" or "C:".
pub fn extract_drive_letter(path: &str) -> Option<char> {
    // Handle paths like "\\?\C:" or "\\.\C:"
    if path.starts_with("\\\\?\\") || path.starts_with("\\\\.\\") {
        path.chars().nth(4)
    } else if path.len() >= 2 && path.as_bytes()[1] == b':' {
        path.chars().next()
    } else {
        None
    }
}

/// Normalize a volume path to the format "\\.\X:" for device access.
pub fn normalize_volume_path(path: &str) -> String {
    if let Some(letter) = extract_drive_letter(path) {
        format!("\\\\.\\{}:", letter.to_ascii_uppercase())
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_wide_string() {
        let wide = to_wide_string("Hello");
        assert_eq!(wide, vec![72, 101, 108, 108, 111, 0]);
    }

    #[test]
    fn test_extract_drive_letter() {
        assert_eq!(extract_drive_letter("C:"), Some('C'));
        assert_eq!(extract_drive_letter("\\\\?\\C:"), Some('C'));
        assert_eq!(extract_drive_letter("\\\\.\\D:"), Some('D'));
        assert_eq!(extract_drive_letter(""), None);
    }

    #[test]
    fn test_normalize_volume_path() {
        assert_eq!(normalize_volume_path("C:"), "\\\\.\\C:");
        assert_eq!(normalize_volume_path("\\\\?\\c:"), "\\\\.\\C:");
        assert_eq!(normalize_volume_path("d:"), "\\\\.\\D:");
    }

    #[test]
    fn test_filetime_to_datetime() {
        // Test with a known timestamp
        // January 1, 2020 00:00:00 UTC in FILETIME
        let ft: i64 = 132224352000000000;
        let dt = filetime_to_datetime(ft);
        assert_eq!(dt.year(), 2020);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 1);
    }

    use chrono::Datelike;
}

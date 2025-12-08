//! Error types for the NTFS backend.

use thiserror::Error;

/// Errors specific to NTFS backend operations.
#[derive(Error, Debug)]
pub enum NtfsError {
    /// Failed to open a volume
    #[error("failed to open volume {volume}: {reason}")]
    VolumeOpen { volume: String, reason: String },

    /// Volume is not NTFS
    #[error("volume {volume} is not NTFS (found: {found})")]
    NotNtfs { volume: String, found: String },

    /// Failed to read MFT
    #[error("failed to read MFT on volume {volume}: {reason}")]
    MftRead { volume: String, reason: String },

    /// Failed to query USN journal
    #[error("failed to query USN journal on volume {volume}: {reason}")]
    UsnJournalQuery { volume: String, reason: String },

    /// USN journal not enabled
    #[error("USN journal not enabled on volume {volume}")]
    UsnJournalNotEnabled { volume: String },

    /// USN journal truncated
    #[error("USN journal truncated on volume {volume}")]
    UsnJournalTruncated { volume: String },

    /// Access denied
    #[error("access denied: {operation} (try running as administrator)")]
    AccessDenied { operation: String },

    /// Windows API error
    #[error("Windows API error: {function} failed with code {code}: {message}")]
    WinApi {
        function: String,
        code: u32,
        message: String,
    },

    /// Generic I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl NtfsError {
    /// Create a WinAPI error from the last Windows error
    #[cfg(windows)]
    pub fn from_win32(function: &str) -> Self {
        use windows::Win32::Foundation::GetLastError;

        let code = unsafe { GetLastError().0 };
        let message = format_win32_error(code);

        // Check for access denied
        if code == 5 {
            return NtfsError::AccessDenied {
                operation: function.to_string(),
            };
        }

        NtfsError::WinApi {
            function: function.to_string(),
            code,
            message,
        }
    }

    /// Check if this error indicates access was denied
    pub fn is_access_denied(&self) -> bool {
        matches!(self, NtfsError::AccessDenied { .. })
            || matches!(self, NtfsError::WinApi { code: 5, .. })
    }

    /// Check if this error indicates the journal needs a rescan
    pub fn requires_rescan(&self) -> bool {
        matches!(
            self,
            NtfsError::UsnJournalTruncated { .. } | NtfsError::UsnJournalNotEnabled { .. }
        )
    }
}

/// Format a Win32 error code to a human-readable message
#[cfg(windows)]
fn format_win32_error(code: u32) -> String {
    use windows::core::PWSTR;
    use windows::Win32::System::Diagnostics::Debug::{
        FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
    };

    let mut buffer = [0u16; 512];
    let len = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            None,
            code,
            0,
            PWSTR(buffer.as_mut_ptr()),
            buffer.len() as u32,
            None,
        )
    };

    if len == 0 {
        return format!("Unknown error ({})", code);
    }

    String::from_utf16_lossy(&buffer[..len as usize])
        .trim()
        .to_string()
}

#[cfg(not(windows))]
fn format_win32_error(_code: u32) -> String {
    "Windows API not available".to_string()
}

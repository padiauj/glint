//! # Glint Windows NTFS Backend
//!
//! This crate provides the Windows/NTFS-specific implementation of the
//! `FileSystemBackend` trait. It uses:
//!
//! - **MFT (Master File Table)** enumeration for fast initial indexing
//! - **USN Change Journal** for real-time incremental updates
//!
//! ## Architecture
//!
//! The backend is structured to isolate all Windows API calls and unsafe code:
//!
//! - `volume.rs`: Volume enumeration and information
//! - `mft.rs`: MFT reading and file enumeration
//! - `usn.rs`: USN Change Journal monitoring
//! - `winapi_utils.rs`: Low-level Windows API wrappers
//!
//! ## Permissions
//!
//! Reading the MFT and USN journal requires elevated privileges:
//! - The process should be run as Administrator, OR
//! - The user should have "Perform Volume Maintenance Tasks" privilege
//!
//! The backend will attempt to work with available permissions and report
//! errors appropriately when access is denied.

#[cfg(windows)]
mod mft;
#[cfg(windows)]
mod usn;
#[cfg(windows)]
mod volume;
#[cfg(windows)]
mod winapi_utils;

#[cfg(windows)]
mod backend;

#[cfg(windows)]
pub use backend::NtfsBackend;

#[cfg(not(windows))]
mod stub;

#[cfg(not(windows))]
pub use stub::NtfsBackend;

/// Error types specific to the NTFS backend
pub mod error;
pub use error::NtfsError;

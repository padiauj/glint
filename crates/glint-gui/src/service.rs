//! Background service management for Glint.
//!
//! This module handles:
//! - Installing/uninstalling the Glint background service
//! - Starting/stopping the service
//! - Checking service status
//!
//! The service monitors USN journals for real-time index updates.

#[cfg(windows)]
mod windows_service {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr;
    use std::time::Duration;
    use tracing::{debug, error, info, warn};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_SERVICE_DOES_NOT_EXIST, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Services::{
        CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW,
        OpenServiceW, QueryServiceStatus, StartServiceW, SC_HANDLE, SC_MANAGER_ALL_ACCESS,
        SERVICE_ALL_ACCESS, SERVICE_AUTO_START, SERVICE_CONTROL_STOP, SERVICE_ERROR_NORMAL,
        SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS, SERVICE_STOP,
        SERVICE_STOPPED, SERVICE_WIN32_OWN_PROCESS,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const SERVICE_NAME: &str = "GlintIndexService";
    const SERVICE_DISPLAY_NAME: &str = "Glint Index Service";
    const SERVICE_DESCRIPTION: &str =
        "Monitors file system changes to keep Glint search index up to date";

    /// Service status
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ServiceStatus {
        NotInstalled,
        Stopped,
        Running,
        Unknown,
    }

    impl std::fmt::Display for ServiceStatus {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                ServiceStatus::NotInstalled => write!(f, "Not Installed"),
                ServiceStatus::Stopped => write!(f, "Stopped"),
                ServiceStatus::Running => write!(f, "Running"),
                ServiceStatus::Unknown => write!(f, "Unknown"),
            }
        }
    }

    /// Convert string to wide null-terminated
    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Check if running with admin privileges
    pub fn is_elevated() -> bool {
        unsafe {
            let mut token_handle: HANDLE = HANDLE::default();
            let process = GetCurrentProcess();

            if OpenProcessToken(process, TOKEN_QUERY, &mut token_handle).is_err() {
                return false;
            }

            let mut elevation = TOKEN_ELEVATION::default();
            let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;

            let result = GetTokenInformation(
                token_handle,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                size,
                &mut size,
            );

            let _ = CloseHandle(token_handle);

            result.is_ok() && elevation.TokenIsElevated != 0
        }
    }

    /// Get the service executable path
    fn get_service_exe_path() -> io::Result<PathBuf> {
        let install_dir = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("Programs")
            .join("Glint");

        // For now, use the CLI exe for the service
        // In the future, we might have a dedicated service exe
        Ok(install_dir.join("glint.exe"))
    }

    /// Get current service status
    pub fn get_service_status() -> ServiceStatus {
        unsafe {
            let sc_manager = OpenSCManagerW(
                PCWSTR(ptr::null()),
                PCWSTR(ptr::null()),
                SC_MANAGER_ALL_ACCESS,
            );
            if sc_manager.is_err() {
                return ServiceStatus::Unknown;
            }
            let sc_manager = sc_manager.unwrap();

            let service_name = to_wide(SERVICE_NAME);
            let service = OpenServiceW(
                sc_manager,
                PCWSTR(service_name.as_ptr()),
                SERVICE_QUERY_STATUS,
            );

            if service.is_err() {
                let _ = CloseServiceHandle(sc_manager);
                return ServiceStatus::NotInstalled;
            }
            let service = service.unwrap();

            let mut status = SERVICE_STATUS::default();
            let result = QueryServiceStatus(service, &mut status);

            let _ = CloseServiceHandle(service);
            let _ = CloseServiceHandle(sc_manager);

            if result.is_err() {
                return ServiceStatus::Unknown;
            }

            match status.dwCurrentState {
                SERVICE_RUNNING => ServiceStatus::Running,
                SERVICE_STOPPED => ServiceStatus::Stopped,
                _ => ServiceStatus::Unknown,
            }
        }
    }

    /// Install the background service
    pub fn install_service() -> io::Result<()> {
        if !is_elevated() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Administrator privileges required to install service",
            ));
        }

        let exe_path = get_service_exe_path()?;
        if !exe_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Service executable not found: {:?}", exe_path),
            ));
        }

        // Service command: glint.exe watch --service
        let service_command = format!("\"{}\" watch --service", exe_path.to_string_lossy());

        unsafe {
            let sc_manager = OpenSCManagerW(
                PCWSTR(ptr::null()),
                PCWSTR(ptr::null()),
                SC_MANAGER_ALL_ACCESS,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

            let service_name = to_wide(SERVICE_NAME);
            let display_name = to_wide(SERVICE_DISPLAY_NAME);
            let binary_path = to_wide(&service_command);

            let service = CreateServiceW(
                sc_manager,
                PCWSTR(service_name.as_ptr()),
                PCWSTR(display_name.as_ptr()),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_AUTO_START, // Start automatically on boot
                SERVICE_ERROR_NORMAL,
                PCWSTR(binary_path.as_ptr()),
                PCWSTR(ptr::null()),
                None,
                PCWSTR(ptr::null()),
                PCWSTR(ptr::null()), // LocalSystem account
                PCWSTR(ptr::null()),
            );

            if let Err(e) = service {
                let _ = CloseServiceHandle(sc_manager);
                return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
            }
            let service = service.unwrap();

            // Set service description via registry
            let _ = set_service_description();

            let _ = CloseServiceHandle(service);
            let _ = CloseServiceHandle(sc_manager);
        }

        info!("Installed service: {}", SERVICE_NAME);
        Ok(())
    }

    /// Set service description in registry
    fn set_service_description() -> io::Result<()> {
        use winreg::enums::*;
        use winreg::RegKey;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = hklm.open_subkey_with_flags(
            format!(r"SYSTEM\CurrentControlSet\Services\{}", SERVICE_NAME),
            KEY_WRITE,
        )?;
        key.set_value("Description", &SERVICE_DESCRIPTION)?;
        Ok(())
    }

    /// Uninstall the background service
    pub fn uninstall_service() -> io::Result<()> {
        if !is_elevated() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Administrator privileges required to uninstall service",
            ));
        }

        // Stop service first if running
        let _ = stop_service();

        unsafe {
            let sc_manager = OpenSCManagerW(
                PCWSTR(ptr::null()),
                PCWSTR(ptr::null()),
                SC_MANAGER_ALL_ACCESS,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

            let service_name = to_wide(SERVICE_NAME);
            let service = OpenServiceW(
                sc_manager,
                PCWSTR(service_name.as_ptr()),
                SERVICE_ALL_ACCESS,
            );

            if let Err(e) = service {
                let _ = CloseServiceHandle(sc_manager);
                // If service doesn't exist, that's fine
                return Ok(());
            }
            let service = service.unwrap();

            let result = DeleteService(service);

            let _ = CloseServiceHandle(service);
            let _ = CloseServiceHandle(sc_manager);

            if let Err(e) = result {
                return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
            }
        }

        info!("Uninstalled service: {}", SERVICE_NAME);
        Ok(())
    }

    /// Start the background service
    pub fn start_service() -> io::Result<()> {
        unsafe {
            let sc_manager = OpenSCManagerW(
                PCWSTR(ptr::null()),
                PCWSTR(ptr::null()),
                SC_MANAGER_ALL_ACCESS,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

            let service_name = to_wide(SERVICE_NAME);
            let service = OpenServiceW(sc_manager, PCWSTR(service_name.as_ptr()), SERVICE_START)
                .map_err(|e| {
                    let _ = CloseServiceHandle(sc_manager);
                    io::Error::new(io::ErrorKind::Other, e.to_string())
                })?;

            let result = StartServiceW(service, None);

            let _ = CloseServiceHandle(service);
            let _ = CloseServiceHandle(sc_manager);

            if let Err(e) = result {
                return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
            }
        }

        info!("Started service: {}", SERVICE_NAME);
        Ok(())
    }

    /// Stop the background service
    pub fn stop_service() -> io::Result<()> {
        unsafe {
            let sc_manager = OpenSCManagerW(
                PCWSTR(ptr::null()),
                PCWSTR(ptr::null()),
                SC_MANAGER_ALL_ACCESS,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

            let service_name = to_wide(SERVICE_NAME);
            let service = OpenServiceW(sc_manager, PCWSTR(service_name.as_ptr()), SERVICE_STOP)
                .map_err(|e| {
                    let _ = CloseServiceHandle(sc_manager);
                    io::Error::new(io::ErrorKind::Other, e.to_string())
                })?;

            let mut status = SERVICE_STATUS::default();
            let result = ControlService(service, SERVICE_CONTROL_STOP, &mut status);

            let _ = CloseServiceHandle(service);
            let _ = CloseServiceHandle(sc_manager);

            if let Err(e) = result {
                // Ignore error if service is already stopped
                warn!("Stop service result: {}", e);
            }
        }

        info!("Stopped service: {}", SERVICE_NAME);
        Ok(())
    }

    /// Toggle service state
    pub fn toggle_service() -> io::Result<ServiceStatus> {
        match get_service_status() {
            ServiceStatus::NotInstalled => {
                install_service()?;
                start_service()?;
                Ok(ServiceStatus::Running)
            }
            ServiceStatus::Stopped => {
                start_service()?;
                Ok(ServiceStatus::Running)
            }
            ServiceStatus::Running => {
                stop_service()?;
                Ok(ServiceStatus::Stopped)
            }
            ServiceStatus::Unknown => Err(io::Error::new(
                io::ErrorKind::Other,
                "Unknown service state",
            )),
        }
    }

    /// Request elevation and restart for service operations
    pub fn request_elevation_for_service(operation: &str) -> io::Result<()> {
        use std::process::Command;
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let current_exe = std::env::current_exe()?;
        let exe_path = to_wide(&current_exe.to_string_lossy());
        let params = to_wide(&format!("--service-{}", operation));
        let verb = to_wide("runas");

        unsafe {
            let result = ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(exe_path.as_ptr()),
                PCWSTR(params.as_ptr()),
                PCWSTR(ptr::null()),
                SW_SHOWNORMAL,
            );

            // ShellExecuteW returns > 32 on success
            if result.0 as usize <= 32 {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to request elevation",
                ));
            }
        }

        Ok(())
    }
}

#[cfg(windows)]
pub use windows_service::*;

// Stub implementations for non-Windows
#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    NotInstalled,
    Stopped,
    Running,
    Unknown,
}

#[cfg(not(windows))]
impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Not supported on this platform")
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(not(windows))]
pub fn get_service_status() -> ServiceStatus {
    ServiceStatus::NotInstalled
}

#[cfg(not(windows))]
pub fn install_service() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Service not supported on this platform",
    ))
}

#[cfg(not(windows))]
pub fn uninstall_service() -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn start_service() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Service not supported on this platform",
    ))
}

#[cfg(not(windows))]
pub fn stop_service() -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn toggle_service() -> std::io::Result<ServiceStatus> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Service not supported on this platform",
    ))
}

#[cfg(not(windows))]
pub fn request_elevation_for_service(_operation: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Service not supported on this platform",
    ))
}

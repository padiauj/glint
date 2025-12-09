//! Self-installer functionality for Glint.
//!
//! This module handles:
//! - Silent self-installation to Program Files
//! - Start Menu shortcut creation
//! - Windows Registry entries for Add/Remove Programs
//! - Self-update when running a newer version

#[cfg(windows)]
mod windows_installer {
    use std::env;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use tracing::{debug, error, info, warn};
    use winreg::enums::*;
    use winreg::RegKey;

    const APP_NAME: &str = "Glint";
    const APP_PUBLISHER: &str = "Glint Contributors";
    const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Glint";

    /// Installation paths
    pub struct InstallPaths {
        pub install_dir: PathBuf,
        pub exe_path: PathBuf,
        pub start_menu_dir: PathBuf,
        pub shortcut_path: PathBuf,
    }

    impl InstallPaths {
        pub fn new() -> io::Result<Self> {
            // Use LocalAppData for per-user install (no admin required)
            let local_app_data = env::var("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    directories::BaseDirs::new()
                        .map(|d| d.data_local_dir().to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."))
                });

            let install_dir = local_app_data.join("Programs").join("Glint");
            let exe_path = install_dir.join("glint-gui.exe");

            // Start Menu in user's AppData
            let start_menu_dir = local_app_data
                .parent()
                .unwrap_or(&local_app_data)
                .join("Roaming")
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs");
            let shortcut_path = start_menu_dir.join("Glint.lnk");

            Ok(Self {
                install_dir,
                exe_path,
                start_menu_dir,
                shortcut_path,
            })
        }
    }

    /// Check if we're running from the installed location
    pub fn is_installed_instance() -> bool {
        if let (Ok(current_exe), Ok(paths)) = (env::current_exe(), InstallPaths::new()) {
            // Normalize paths for comparison
            let current = current_exe.canonicalize().unwrap_or(current_exe);
            let installed = paths.exe_path.canonicalize().unwrap_or(paths.exe_path);
            current == installed
        } else {
            false
        }
    }

    /// Check if installation exists
    pub fn is_installed() -> bool {
        InstallPaths::new()
            .map(|p| p.exe_path.exists())
            .unwrap_or(false)
    }

    /// Get installed version from registry
    pub fn get_installed_version() -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey(UNINSTALL_KEY)
            .ok()
            .and_then(|key| key.get_value("DisplayVersion").ok())
    }

    /// Check if current version is newer than installed
    pub fn needs_update() -> bool {
        match get_installed_version() {
            Some(installed) => {
                // Simple version comparison (works for semver)
                APP_VERSION > installed.as_str()
            }
            None => true, // Not installed = needs install
        }
    }

    /// Perform silent installation or update
    pub fn install_or_update() -> io::Result<bool> {
        let paths = InstallPaths::new()?;
        let current_exe = env::current_exe()?;

        // Don't reinstall if running from installed location
        if is_installed_instance() {
            debug!("Already running from installed location");
            return Ok(false);
        }

        // Check if we need to install/update
        if !needs_update() && paths.exe_path.exists() {
            debug!("Already up to date");
            return Ok(false);
        }

        info!(
            "Installing Glint v{} to {:?}",
            APP_VERSION, paths.install_dir
        );

        // Create installation directory
        fs::create_dir_all(&paths.install_dir)?;

        // Copy executable (handle "in use" by renaming old first)
        if paths.exe_path.exists() {
            let backup = paths.install_dir.join("glint-gui.exe.old");
            let _ = fs::remove_file(&backup); // Remove old backup if exists
            if let Err(e) = fs::rename(&paths.exe_path, &backup) {
                warn!("Could not rename old exe: {}", e);
                // Try direct overwrite
            }
        }

        // Copy current exe to install location
        fs::copy(&current_exe, &paths.exe_path)?;
        info!("Copied executable to {:?}", paths.exe_path);

        // Create Start Menu shortcut
        if let Err(e) = create_shortcut(&paths) {
            warn!("Failed to create Start Menu shortcut: {}", e);
        }

        // Register in Add/Remove Programs
        if let Err(e) = register_uninstall(&paths) {
            warn!("Failed to register uninstall: {}", e);
        }

        info!("Installation complete");
        Ok(true)
    }

    /// Create Start Menu shortcut using PowerShell (simpler and more reliable)
    fn create_shortcut(paths: &InstallPaths) -> io::Result<()> {
        use std::process::Command;

        // Ensure Start Menu directory exists
        fs::create_dir_all(&paths.start_menu_dir)?;

        // Use PowerShell to create the shortcut
        let ps_script = format!(
            r#"
            $WshShell = New-Object -ComObject WScript.Shell
            $Shortcut = $WshShell.CreateShortcut("{}")
            $Shortcut.TargetPath = "{}"
            $Shortcut.WorkingDirectory = "{}"
            $Shortcut.Description = "Fast File Search"
            $Shortcut.Save()
            "#,
            paths.shortcut_path.to_string_lossy().replace("\\", "\\\\"),
            paths.exe_path.to_string_lossy().replace("\\", "\\\\"),
            paths.install_dir.to_string_lossy().replace("\\", "\\\\"),
        );

        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &ps_script,
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("PowerShell shortcut creation warning: {}", stderr);
            // Don't fail - shortcut is not critical
        }

        info!("Created Start Menu shortcut at {:?}", paths.shortcut_path);
        Ok(())
    }

    /// Register application in Add/Remove Programs
    fn register_uninstall(paths: &InstallPaths) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(UNINSTALL_KEY)?;

        key.set_value("DisplayName", &APP_NAME)?;
        key.set_value("DisplayVersion", &APP_VERSION)?;
        key.set_value("Publisher", &APP_PUBLISHER)?;
        key.set_value(
            "InstallLocation",
            &paths.install_dir.to_string_lossy().to_string(),
        )?;
        key.set_value(
            "DisplayIcon",
            &format!("{},0", paths.exe_path.to_string_lossy()),
        )?;
        key.set_value(
            "UninstallString",
            &format!("\"{}\" --uninstall", paths.exe_path.to_string_lossy()),
        )?;
        key.set_value("NoModify", &1u32)?;
        key.set_value("NoRepair", &1u32)?;

        // Estimate size (in KB)
        let size_kb = paths
            .exe_path
            .metadata()
            .map(|m| m.len() / 1024)
            .unwrap_or(0) as u32;
        key.set_value("EstimatedSize", &size_kb)?;

        info!("Registered in Add/Remove Programs");
        Ok(())
    }

    /// Uninstall the application
    pub fn uninstall() -> io::Result<()> {
        let paths = InstallPaths::new()?;

        info!("Uninstalling Glint...");

        // Stop service if running
        let _ = crate::service::stop_service();
        let _ = crate::service::uninstall_service();

        // Remove Start Menu shortcut
        if paths.shortcut_path.exists() {
            fs::remove_file(&paths.shortcut_path)?;
            info!("Removed Start Menu shortcut");
        }

        // Remove registry entry
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
        info!("Removed registry entries");

        // Remove installation directory
        // Note: Can't delete ourselves while running, so schedule deletion
        if paths.install_dir.exists() {
            // Try to remove non-exe files first
            for entry in fs::read_dir(&paths.install_dir)? {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path != paths.exe_path {
                        let _ = fs::remove_file(&path);
                    }
                }
            }

            // Schedule the exe and dir for deletion on reboot
            schedule_delete_on_reboot(&paths.exe_path);
            schedule_delete_on_reboot(&paths.install_dir);
            info!("Scheduled removal of installation directory on reboot");
        }

        info!("Uninstall complete (some files will be removed on reboot)");
        Ok(())
    }

    /// Schedule a file for deletion on next reboot
    fn schedule_delete_on_reboot(path: &Path) {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::MoveFileExW;
        use windows::Win32::Storage::FileSystem::MOVEFILE_DELAY_UNTIL_REBOOT;

        let path_wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let _ = MoveFileExW(
                PCWSTR(path_wide.as_ptr()),
                PCWSTR(ptr::null()),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            );
        }
    }

    use std::ptr;
}

#[cfg(windows)]
pub use windows_installer::*;

#[cfg(not(windows))]
pub fn is_installed_instance() -> bool {
    false
}

#[cfg(not(windows))]
pub fn is_installed() -> bool {
    false
}

#[cfg(not(windows))]
pub fn install_or_update() -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn uninstall() -> std::io::Result<()> {
    Ok(())
}

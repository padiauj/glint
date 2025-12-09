//! Main application state and logic.

use crate::search::SearchState;
use crate::service::{self, ServiceStatus};
use crate::settings::Settings;
use crate::ui;
use eframe::egui;
use glint_core::{Config, Index, IndexStore};
use glint_core::archive_view::ArchivedView;
use crossbeam_channel::{unbounded, Receiver};
use std::time::{Duration, Instant};
use std::sync::Arc;

/// Information about a volume (for UI selection)
#[derive(Clone)]
pub struct VolumeInfo {
    pub letter: char,
    pub label: String,
    pub size: u64,
    pub selected: bool,
}

/// Main application state
pub struct GlintApp {
    pub search: SearchState,
    pub index: Arc<Index>,
    pub store: IndexStore,
    pub config: Config,
    pub settings: Settings,
    pub available_volumes: Vec<VolumeInfo>,
    pub dark_mode: bool,
    pub show_settings: bool,
    pub show_about: bool,
    pub show_index_builder: bool,
    pub status_message: String,
    pub service_status: ServiceStatus,
    pub enable_service_on_index: bool,

    // Async index loading
    loading_index: bool,
    load_started_at: Instant,
    load_rx: Option<Receiver<Arc<Index>>>,

    // Async index building
    building_index: bool,
    build_started_at: Instant,
    build_rx: Option<Receiver<Result<Arc<Index>, String>>>,
    saving_index: bool,
    save_rx: Option<Receiver<Result<(), String>>>,
}

impl GlintApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);

        let config = Config::load().unwrap_or_default();
        let settings = Settings::load().unwrap_or_default();

        let available_volumes = detect_ntfs_volumes(&settings.indexed_volumes);

        let data_dir = config.index_dir().unwrap_or_else(|_| {
            directories::ProjectDirs::from("org", "glint", "glint")
                .map(|p| p.data_dir().to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        });
        let store = IndexStore::new(&data_dir);
        // Start with empty index and load asynchronously so UI is instant
        let index = Arc::new(Index::new());
        let (tx, rx) = unbounded::<Arc<Index>>();
        let data_dir_clone = data_dir.clone();
        std::thread::spawn(move || {
            let s = IndexStore::new(&data_dir_clone);
            let loaded = s.load_or_new();
            let _ = tx.send(Arc::new(loaded));
        });
        let status_message = "Loading index from disk...".to_string();

        let service_status = service::get_service_status();

        Self {
            search: SearchState::new(Arc::clone(&index)),
            index,
            store,
            config,
            settings,
            available_volumes,
            dark_mode: true,
            show_settings: false,
            show_about: false,
            show_index_builder: false,
            status_message,
            service_status,
            enable_service_on_index: true,
            loading_index: true,
            load_started_at: Instant::now(),
            load_rx: Some(rx),
            building_index: false,
            build_started_at: Instant::now(),
            build_rx: None,
            saving_index: false,
            save_rx: None,
        }
    }

    pub fn reload_index(&mut self) {
        self.index = Arc::new(self.store.load_or_new());
        self.search.set_index(Arc::clone(&self.index));
        let count = self.index.len();
        self.status_message = format!("Index reloaded: {} files", format_number(count));
        self.search.clear();
    }

    pub fn refresh_service_status(&mut self) {
        self.service_status = service::get_service_status();
    }

    pub fn toggle_service(&mut self) {
        if !service::is_elevated() {
            let operation = match self.service_status {
                ServiceStatus::NotInstalled => "install",
                ServiceStatus::Stopped => "start",
                ServiceStatus::Running => "stop",
                ServiceStatus::Unknown => return,
            };

            if let Err(e) = service::request_elevation_for_service(operation) {
                self.status_message = format!("Failed to request elevation: {}", e);
            } else {
                self.status_message = "Requesting administrator privileges...".to_string();
            }
        } else {
            match service::toggle_service() {
                Ok(new_status) => {
                    self.service_status = new_status;
                    self.status_message = format!("Service is now {}", new_status);
                }
                Err(e) => {
                    self.status_message = format!("Service toggle failed: {}", e);
                }
            }
        }
    }

    /// Index selected volumes (Windows NTFS)
    pub fn index_volumes(&mut self) {
        let volumes: Vec<char> = self
            .available_volumes
            .iter()
            .filter(|v| v.selected)
            .map(|v| v.letter)
            .collect();

        if volumes.is_empty() {
            self.status_message = "No volumes selected".to_string();
            return;
        }

        self.status_message = format!("Indexing volumes: {:?}...", volumes);

        #[cfg(windows)]
        {
            use glint_backend_ntfs::NtfsBackend;
            use glint_core::backend::FileSystemBackend;

            let backend = NtfsBackend::new();
            let new_index = Index::new();
            let mut total_records = 0usize;

            match backend.list_volumes() {
                Ok(all_volumes) => {
                    for volume in all_volumes {
                        let mount_letter = volume
                            .mount_point
                            .chars()
                            .next()
                            .map(|c| c.to_ascii_uppercase());

                        if let Some(letter) = mount_letter {
                            if volumes.contains(&letter) {
                                match backend.full_scan(&volume, None) {
                                    Ok(records) => {
                                        total_records += records.len();
                                        new_index.add_volume_records(&volume, records);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            volume = %volume.mount_point,
                                            error = %e,
                                            "Failed to scan volume"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    self.status_message = format!("Failed to enumerate volumes: {}", e);
                    return;
                }
            }

            self.index = Arc::new(new_index);
            self.search.set_index(Arc::clone(&self.index));
            if let Err(e) = self.store.save(&self.index) {
                self.status_message = format!(
                    "Indexed {} files but failed to save: {}",
                    format_number(total_records),
                    e
                );
            } else {
                self.status_message =
                    format!("Successfully indexed {} files", format_number(total_records));
            }
        }

        #[cfg(not(windows))]
        {
            self.status_message = "NTFS indexing only available on Windows".to_string();
        }
    }
}

impl eframe::App for GlintApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll async search results first
        self.search.poll_results();

        // Poll async index loader and update status bar with progress
        if self.loading_index {
            if let Some(rx) = &self.load_rx {
                match rx.try_recv() {
                    Ok(new_index) => {
                        self.index = new_index;
                        self.search.set_index(Arc::clone(&self.index));
                        // Try to open zero-copy archived view (if v3 exists)
                        if let Ok(view) = ArchivedView::open(self.store.index_path()) {
                            self.search.set_archived_view(Arc::new(view));
                        }
                        let count = self.index.len();
                        self.status_message = if count > 0 {
                            format!("{} files indexed", format_number(count))
                        } else {
                            "No index found. Click 'Build Index' to get started.".to_string()
                        };
                        self.show_index_builder = count == 0;
                        self.loading_index = false;
                        self.load_rx = None;
                    }
                    Err(_) => {
                        let secs = self.load_started_at.elapsed().as_secs_f32();
                        self.status_message = format!("Loading index... {:.1}s", secs);
                        ctx.request_repaint_after(Duration::from_millis(150));
                    }
                }
            }
        }
        if self.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }

        handle_shortcuts(ctx, self);

        ui::menu_bar(ctx, self);
        ui::top_panel(ctx, self);
        ui::bottom_panel(ctx, self);
        ui::central_panel(ctx, self);

        if self.show_settings {
            ui::settings_window(ctx, self);
        }
        if self.show_about {
            ui::about_window(ctx, self);
        }
        if self.show_index_builder {
            ui::index_builder_window(ctx, self);
        }

        // Poll async index build
        if self.building_index {
            if let Some(rx) = &self.build_rx {
                match rx.try_recv() {
                    Ok(Ok(new_index)) => {
                        self.index = new_index;
                        self.search.set_index(Arc::clone(&self.index));
                        let count = self.index.len();
                        self.status_message = format!("Indexed {} files. Saving...", format_number(count));
                        self.building_index = false;

                        // Save asynchronously
                        let index_for_save = Arc::clone(&self.index);
                        let base_dir = self.store.index_path().parent().map(|p| p.to_path_buf());
                        if let Some(dir) = base_dir {
                            let (stx, srx) = unbounded::<Result<(), String>>();
                            self.save_rx = Some(srx);
                            self.saving_index = true;
                            std::thread::spawn(move || {
                                let store = IndexStore::new(&dir);
                                let res = store.save(&index_for_save).map_err(|e| e.to_string());
                                let _ = stx.send(res);
                            });
                        } else {
                            self.status_message = "Indexed, but failed to resolve save path".to_string();
                        }
                    }
                    Ok(Err(msg)) => {
                        self.status_message = msg;
                        self.building_index = false;
                    }
                    Err(_) => {
                        let secs = self.build_started_at.elapsed().as_secs_f32();
                        self.status_message = format!("Indexing... {:.1}s", secs);
                        ctx.request_repaint_after(Duration::from_millis(150));
                    }
                }
            }
        }

        // Poll async save
        if self.saving_index {
            if let Some(rx) = &self.save_rx {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        self.status_message = "Index saved".to_string();
                        self.saving_index = false;
                        self.save_rx = None;
                    }
                    Ok(Err(msg)) => {
                        self.status_message = format!("Save failed: {}", msg);
                        self.saving_index = false;
                        self.save_rx = None;
                    }
                    Err(_) => {
                        // throttle repaint
                        ctx.request_repaint_after(Duration::from_millis(150));
                    }
                }
            }
        }
    }
}

impl GlintApp {
    /// Start building index asynchronously for selected volumes
    pub fn start_index_build(&mut self) {
        let volumes: Vec<char> = self
            .available_volumes
            .iter()
            .filter(|v| v.selected)
            .map(|v| v.letter)
            .collect();
        if volumes.is_empty() {
            self.status_message = "Please select at least one volume".to_string();
            return;
        }

        // Persist selected volumes
        self.settings.indexed_volumes = volumes.clone();
        if let Err(e) = self.settings.save() {
            self.status_message = format!("Failed to save settings: {}", e);
        }

        let (tx, rx) = unbounded::<Result<Arc<Index>, String>>();
        self.build_rx = Some(rx);
        self.building_index = true;
        self.build_started_at = Instant::now();
        self.status_message = format!("Indexing volumes: {:?}...", volumes);

        std::thread::spawn(move || {
            #[cfg(windows)]
            {
                use glint_backend_ntfs::NtfsBackend;
                use glint_core::{backend::FileSystemBackend, Index};

                let backend = NtfsBackend::new();
                let new_index = Index::new();
                match backend.list_volumes() {
                    Ok(all) => {
                        for volume in all {
                            let mount_letter = volume
                                .mount_point
                                .chars()
                                .next()
                                .map(|c| c.to_ascii_uppercase());
                            if let Some(letter) = mount_letter {
                                if volumes.contains(&letter) {
                                    match backend.full_scan(&volume, None) {
                                        Ok(records) => {
                                            new_index.add_volume_records(&volume, records);
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Err(format!(
                                                "Failed to scan {}: {}",
                                                volume.mount_point, e
                                            )));
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                        let _ = tx.send(Ok(Arc::new(new_index)));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("Failed to enumerate volumes: {}", e)));
                    }
                }
            }
            #[cfg(not(windows))]
            {
                let _ = tx.send(Err("NTFS indexing only available on Windows".to_string()));
            }
        });
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let fonts = egui::FontDefinitions::default();
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    ctx.set_style(style);
}

fn handle_shortcuts(ctx: &egui::Context, app: &mut GlintApp) {
    if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
        app.reload_index();
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        if !app.search.query.is_empty() {
            app.search.query.clear();
            app.search.clear();
        }
    }
    if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Comma)) {
        app.show_settings = !app.show_settings;
    }
}

pub fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(windows)]
fn detect_ntfs_volumes(previously_selected: &[char]) -> Vec<VolumeInfo> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW};

    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOVABLE: u32 = 2;

    let mut volumes = Vec::new();

    for letter in 'A'..='Z' {
        let root: Vec<u16> = OsStr::new(&format!("{}:\\", letter))
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let drive_type = GetDriveTypeW(windows::core::PCWSTR(root.as_ptr()));
            if drive_type != DRIVE_FIXED && drive_type != DRIVE_REMOVABLE {
                continue;
            }

            let mut volume_name = [0u16; 261];
            let mut fs_name = [0u16; 261];
            let mut serial = 0u32;
            let mut max_component = 0u32;
            let mut flags = 0u32;

            let success = GetVolumeInformationW(
                windows::core::PCWSTR(root.as_ptr()),
                Some(&mut volume_name),
                Some(&mut serial),
                Some(&mut max_component),
                Some(&mut flags),
                Some(&mut fs_name),
            );
            if success.is_err() {
                continue;
            }

            let fs_string = String::from_utf16_lossy(
                &fs_name[..fs_name.iter().position(|&c| c == 0).unwrap_or(0)],
            );
            if fs_string != "NTFS" {
                continue;
            }

            let mut free_bytes = 0u64;
            let mut total_bytes = 0u64;
            let mut free_avail = 0u64;
            let _ = GetDiskFreeSpaceExW(
                windows::core::PCWSTR(root.as_ptr()),
                Some(&mut free_avail),
                Some(&mut total_bytes),
                Some(&mut free_bytes),
            );

            let label = String::from_utf16_lossy(
                &volume_name[..volume_name.iter().position(|&c| c == 0).unwrap_or(0)],
            );
            let label = if label.is_empty() {
                "Local Disk".to_string()
            } else {
                label
            };

            volumes.push(VolumeInfo {
                letter,
                label,
                size: total_bytes,
                selected: previously_selected.is_empty() || previously_selected.contains(&letter),
            });
        }
    }

    volumes
}

#[cfg(not(windows))]
fn detect_ntfs_volumes(_previously_selected: &[char]) -> Vec<VolumeInfo> {
    Vec::new()
}

//! Qt/QML bridge module using cxx-qt.
//!
//! This module defines the QObject types that bridge between Rust and QML.

use cxx_qt::CxxQtType;
use std::pin::Pin;
use std::sync::mpsc;

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, query)]
        #[qproperty(QString, status_message)]
        #[qproperty(i32, result_count)]
        #[qproperty(i32, index_count)]
        #[qproperty(bool, is_indexing)]
        #[qproperty(bool, service_running)]
        #[qproperty(bool, dark_mode)]
        #[qproperty(i32, index_progress)]
        #[qproperty(QString, index_progress_text)]
        #[qproperty(i32, index_total_volumes)]
        #[qproperty(i32, index_current_volume)]
        type GlintController = super::GlintControllerRust;
    }
    
    unsafe extern "RustQt" {
        #[qinvokable]
        fn search(self: Pin<&mut GlintController>);
        
        #[qinvokable]
        fn clear_search(self: Pin<&mut GlintController>);
        
        #[qinvokable]
        fn open_item(self: &GlintController, path: &QString);
        
        #[qinvokable]
        fn open_folder(self: &GlintController, path: &QString);
        
        #[qinvokable]
        fn copy_path(self: &GlintController, path: &QString);
        
        #[qinvokable]
        fn start_indexing(self: Pin<&mut GlintController>, volumes: &QString);
        
        #[qinvokable]
        fn check_indexing_progress(self: Pin<&mut GlintController>);
        
        #[qinvokable]
        fn toggle_service(self: Pin<&mut GlintController>);
        
        #[qinvokable]
        fn reload_index(self: Pin<&mut GlintController>);
        
        #[qinvokable]
        fn get_available_volumes(self: &GlintController) -> QString;
        
        #[qinvokable]
        fn get_result(self: &GlintController, index: i32) -> QString;
        
        #[qinvokable]
        fn needs_initial_setup(self: &GlintController) -> bool;
        
        #[qinvokable]
        fn get_configured_volumes(self: &GlintController) -> QString;
    }
}

use cxx_qt_lib::QString;
use parking_lot::RwLock;
use std::sync::Arc;

/// Progress message from indexing thread
#[derive(Debug, Clone)]
pub enum IndexingProgress {
    Starting { total_volumes: i32 },
    Volume { current: i32, letter: String, status: String },
    VolumeComplete { current: i32, letter: String, files: usize },
    Complete { total_files: usize },
    Error(String),
}

/// Rust implementation of GlintController QObject
pub struct GlintControllerRust {
    query: QString,
    status_message: QString,
    result_count: i32,
    index_count: i32,
    is_indexing: bool,
    service_running: bool,
    dark_mode: bool,
    index_progress: i32,
    index_progress_text: QString,
    index_total_volumes: i32,
    index_current_volume: i32,
    
    // Internal state (not exposed to QML)
    index: Arc<RwLock<glint_core::Index>>,
    store: glint_core::IndexStore,
    results: Vec<glint_core::search::SearchResult>,
    config: glint_core::Config,
    progress_receiver: Option<mpsc::Receiver<IndexingProgress>>,
    indexing_result: Option<Arc<RwLock<Option<glint_core::Index>>>>,
}

impl Default for GlintControllerRust {
    fn default() -> Self {
        // Load configuration
        let config = glint_core::Config::load().unwrap_or_default();
        
        // Check if volumes are configured
        let has_configured_volumes = !config.volumes.include.is_empty();
        
        // Load index only if we have configured volumes
        let data_dir = config.index_dir().unwrap_or_else(|_| {
            directories::ProjectDirs::from("org", "glint", "glint")
                .map(|p| p.data_dir().to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        });
        let store = glint_core::IndexStore::new(&data_dir);
        
        // Only load existing index if volumes are configured
        let index = if has_configured_volumes {
            store.load_or_new()
        } else {
            glint_core::Index::new()
        };
        let index_count = index.len() as i32;
        
        let status_message = if !has_configured_volumes {
            "Welcome! Please select volumes to index.".to_string()
        } else if index_count > 0 {
            format!("{} files indexed", format_number(index_count as usize))
        } else {
            "No index found. Build index to get started.".to_string()
        };
        
        // Check service status
        #[cfg(windows)]
        let service_running = crate::service::get_service_status() == crate::service::ServiceStatus::Running;
        #[cfg(not(windows))]
        let service_running = false;
        
        Self {
            query: QString::default(),
            status_message: QString::from(&status_message),
            result_count: 0,
            index_count,
            is_indexing: false,
            service_running,
            dark_mode: true,
            index_progress: 0,
            index_progress_text: QString::default(),
            index_total_volumes: 0,
            index_current_volume: 0,
            index: Arc::new(RwLock::new(index)),
            store,
            results: Vec::new(),
            config,
            progress_receiver: None,
            indexing_result: None,
        }
    }
}

impl ffi::GlintController {
    fn search(self: Pin<&mut Self>) {
        let mut rust = self.rust_mut();
        let query_str = rust.query.to_string();
        
        if query_str.is_empty() {
            rust.results.clear();
            rust.result_count = 0;
            rust.status_message = QString::from("Enter a search term");
            return;
        }
        
        let search_query = glint_core::SearchQuery::substring(&query_str);
        let index_arc = rust.index.clone();
        let index = index_arc.read();
        
        let results: Vec<_> = index.search(&search_query).into_iter().take(100).collect();
        let result_count = results.len() as i32;
        drop(index);
        
        rust.results = results;
        rust.result_count = result_count;
        rust.status_message = QString::from(&format!(
            "{} results for \"{}\"",
            result_count,
            query_str
        ));
    }
    
    fn clear_search(self: Pin<&mut Self>) {
        let mut rust = self.rust_mut();
        rust.query = QString::default();
        rust.results.clear();
        rust.result_count = 0;
        rust.status_message = QString::from(&format!(
            "{} files indexed",
            format_number(rust.index_count as usize)
        ));
    }
    
    fn open_item(&self, path: &QString) {
        let path_str = path.to_string();
        let _ = open::that(&path_str);
    }
    
    fn open_folder(&self, path: &QString) {
        let path_str = path.to_string();
        if let Some(parent) = std::path::Path::new(&path_str).parent() {
            let _ = open::that(parent);
        }
    }
    
    fn copy_path(&self, path: &QString) {
        #[cfg(windows)]
        {
            use std::process::Command;
            let path_str = path.to_string();
            let _ = Command::new("cmd")
                .args(["/C", &format!("echo {} | clip", path_str)])
                .output();
        }
    }
    
    fn start_indexing(self: Pin<&mut Self>, volumes: &QString) {
        let mut rust = self.rust_mut();
        
        // Don't start if already indexing
        if rust.is_indexing {
            return;
        }
        
        rust.is_indexing = true;
        rust.index_progress = 0;
        rust.index_progress_text = QString::from("Starting indexing...");
        rust.status_message = QString::from("Indexing in progress...");
        
        let selected_letters: Vec<String> = volumes.to_string()
            .split(',')
            .filter_map(|s| {
                let trimmed = s.trim();
                if trimmed.is_empty() { None } else { Some(trimmed.to_uppercase()) }
            })
            .collect();
        
        // Save selected volumes to config
        rust.config.volumes.include = selected_letters.clone();
        if let Err(e) = rust.config.save() {
            tracing::warn!("Failed to save config: {}", e);
        }
        
        // Create channel for progress updates
        let (tx, rx) = mpsc::channel();
        rust.progress_receiver = Some(rx);
        
        // Create shared result storage
        let result_index = Arc::new(RwLock::new(None));
        rust.indexing_result = Some(result_index.clone());
        
        // Get store base directory for saving in background thread
        let store_base_dir = rust.store.index_path().parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        
        // Spawn background thread
        std::thread::spawn(move || {
            #[cfg(windows)]
            {
                use glint_backend_ntfs::NtfsBackend;
                use glint_core::backend::FileSystemBackend;
                
                let backend = NtfsBackend::new();
                let new_index = glint_core::Index::new();
                let mut total_records = 0;
                
                let all_volumes = match backend.list_volumes() {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = tx.send(IndexingProgress::Error(format!("Failed to list volumes: {}", e)));
                        return;
                    }
                };
                
                let volumes_to_index: Vec<_> = all_volumes.iter()
                    .filter(|v| {
                        v.mount_point.chars().next()
                            .map(|c| selected_letters.contains(&c.to_ascii_uppercase().to_string()))
                            .unwrap_or(false)
                    })
                    .collect();
                
                let total_volumes = volumes_to_index.len() as i32;
                let _ = tx.send(IndexingProgress::Starting { total_volumes });
                
                for (i, volume) in volumes_to_index.iter().enumerate() {
                    let letter = volume.mount_point.chars().next()
                        .map(|c| c.to_ascii_uppercase().to_string())
                        .unwrap_or_default();
                    
                    let _ = tx.send(IndexingProgress::Volume {
                        current: i as i32 + 1,
                        letter: letter.clone(),
                        status: format!("Scanning {}:...", letter),
                    });
                    
                    match backend.full_scan(volume, None) {
                        Ok(records) => {
                            let count = records.len();
                            new_index.add_volume_records(volume, records);
                            total_records += count;
                            
                            let _ = tx.send(IndexingProgress::VolumeComplete {
                                current: i as i32 + 1,
                                letter: letter.clone(),
                                files: count,
                            });
                        }
                        Err(e) => {
                            tracing::warn!("Failed to scan volume {}: {}", letter, e);
                        }
                    }
                }
                
                // Save the index
                let store = glint_core::IndexStore::new(&store_base_dir);
                if let Err(e) = store.save(&new_index) {
                    tracing::warn!("Failed to save index: {}", e);
                }
                
                // Store the result
                *result_index.write() = Some(new_index);
                
                let _ = tx.send(IndexingProgress::Complete { total_files: total_records });
            }
            
            #[cfg(not(windows))]
            {
                let _ = tx.send(IndexingProgress::Error("Indexing only available on Windows".to_string()));
            }
        });
    }
    
    fn check_indexing_progress(self: Pin<&mut Self>) {
        let mut rust = self.rust_mut();
        
        if let Some(ref rx) = rust.progress_receiver {
            // Process all available messages
            while let Ok(progress) = rx.try_recv() {
                match progress {
                    IndexingProgress::Starting { total_volumes } => {
                        rust.index_total_volumes = total_volumes;
                        rust.index_current_volume = 0;
                        rust.index_progress = 0;
                        rust.index_progress_text = QString::from("Starting...");
                    }
                    IndexingProgress::Volume { current, letter, status } => {
                        rust.index_current_volume = current;
                        rust.index_progress = ((current - 1) * 100) / rust.index_total_volumes.max(1);
                        rust.index_progress_text = QString::from(&status);
                        rust.status_message = QString::from(&format!("Indexing {}:...", letter));
                    }
                    IndexingProgress::VolumeComplete { current, letter, files } => {
                        rust.index_progress = (current * 100) / rust.index_total_volumes.max(1);
                        rust.index_progress_text = QString::from(&format!(
                            "Completed {}: ({} files)",
                            letter,
                            format_number(files)
                        ));
                    }
                    IndexingProgress::Complete { total_files } => {
                        rust.is_indexing = false;
                        rust.index_progress = 100;
                        rust.index_count = total_files as i32;
                        rust.index_progress_text = QString::from("Complete!");
                        rust.status_message = QString::from(&format!(
                            "Indexed {} files",
                            format_number(total_files)
                        ));
                        
                        // Load the new index from result
                        if let Some(ref result) = rust.indexing_result {
                            if let Some(new_index) = result.write().take() {
                                *rust.index.write() = new_index;
                            }
                        }
                        
                        // Clean up
                        rust.progress_receiver = None;
                        rust.indexing_result = None;
                    }
                    IndexingProgress::Error(msg) => {
                        rust.is_indexing = false;
                        rust.index_progress = 0;
                        rust.index_progress_text = QString::from(&format!("Error: {}", msg));
                        rust.status_message = QString::from(&format!("Indexing failed: {}", msg));
                        rust.progress_receiver = None;
                        rust.indexing_result = None;
                    }
                }
            }
        }
    }
    
    fn toggle_service(self: Pin<&mut Self>) {
        #[cfg(windows)]
        {
            let mut rust = self.rust_mut();
            match crate::service::toggle_service() {
                Ok(status) => {
                    rust.service_running = status == crate::service::ServiceStatus::Running;
                    rust.status_message = QString::from(&format!("Service: {}", status));
                }
                Err(e) => {
                    rust.status_message = QString::from(&format!("Service error: {}", e));
                }
            }
        }
    }
    
    fn reload_index(self: Pin<&mut Self>) {
        let mut rust = self.rust_mut();
        let index = rust.store.load_or_new();
        rust.index_count = index.len() as i32;
        *rust.index.write() = index;
        rust.results.clear();
        rust.result_count = 0;
        rust.status_message = QString::from(&format!(
            "Index reloaded: {} files",
            format_number(rust.index_count as usize)
        ));
    }
    
    fn get_available_volumes(&self) -> QString {
        let mut result = String::new();
        
        #[cfg(windows)]
        {
            use glint_backend_ntfs::NtfsBackend;
            use glint_core::backend::FileSystemBackend;
            
            let backend = NtfsBackend::new();
            if let Ok(volumes) = backend.list_volumes() {
                for (i, vol) in volumes.iter().enumerate() {
                    if i > 0 {
                        result.push(';');
                    }
                    let letter = vol.mount_point.chars().next().unwrap_or('?');
                    let label = vol.label.as_deref().unwrap_or("Local Disk");
                    let size = format_size(vol.total_bytes.unwrap_or(0));
                    result.push_str(&format!("{}|{}|{}", letter, label, size));
                }
            }
        }
        
        QString::from(&result)
    }
    
    fn get_result(&self, index: i32) -> QString {
        let rust = self.rust();
        if let Some(result) = rust.results.get(index as usize) {
            let record = &result.record;
            let size = record.size.map(|s| format_size(s)).unwrap_or_default();
            let modified = record.modified
                .map(|m| m.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();
            QString::from(&format!(
                "{}|{}|{}|{}|{}",
                record.name,
                record.path,
                size,
                modified,
                record.is_dir
            ))
        } else {
            QString::default()
        }
    }
    
    fn needs_initial_setup(&self) -> bool {
        let rust = self.rust();
        // Need setup if no volumes are configured
        rust.config.volumes.include.is_empty()
    }
    
    fn get_configured_volumes(&self) -> QString {
        let rust = self.rust();
        // Return comma-separated list of configured volume letters
        QString::from(&rust.config.volumes.include.join(","))
    }
}

fn format_number(n: usize) -> String {
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

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

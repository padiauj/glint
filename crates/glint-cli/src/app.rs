//! Application state management.

use glint_backend_ntfs::NtfsBackend;
use glint_core::{Config, FileSystemBackend, Index, IndexStore};
use std::sync::Arc;
use tracing::info;

/// Shared application state.
pub struct App {
    /// Configuration
    pub config: Config,

    /// The file index
    pub index: Arc<Index>,

    /// Index persistence
    pub store: IndexStore,

    /// Filesystem backend
    pub backend: Arc<NtfsBackend>,
}

impl App {
    /// Create a new application instance.
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let data_dir = config.index_dir()?;
        let store = IndexStore::new(&data_dir);
        let index = Arc::new(store.load_or_new());
        let backend = Arc::new(NtfsBackend::new());

        info!(
            data_dir = %data_dir.display(),
            records = index.len(),
            "Application initialized"
        );

        Ok(App {
            config,
            index,
            store,
            backend,
        })
    }

    /// Save the current index to disk.
    pub fn save_index(&self) -> anyhow::Result<()> {
        self.store.save(&self.index)?;
        Ok(())
    }

    /// Rebuild the index from scratch.
    pub fn rebuild_index(&self, volumes: &[String]) -> anyhow::Result<()> {
        use glint_core::backend::LoggingProgress;

        self.index.clear();

        let available_volumes = self.backend.list_volumes()?;

        let volumes_to_index: Vec<_> = if volumes.is_empty() {
            available_volumes
                .into_iter()
                .filter(|v| self.config.should_index_volume(&v.mount_point))
                .collect()
        } else {
            available_volumes
                .into_iter()
                .filter(|v| {
                    volumes.iter().any(|requested| {
                        v.mount_point
                            .to_lowercase()
                            .starts_with(&requested.to_lowercase())
                    })
                })
                .collect()
        };

        for volume in volumes_to_index {
            info!(volume = %volume.mount_point, "Indexing volume");

            let progress = Arc::new(LoggingProgress::new(&volume.mount_point));
            let records = self.backend.full_scan(&volume, Some(progress))?;

            self.index.add_volume_records(&volume, records);
        }

        self.save_index()?;

        Ok(())
    }
}

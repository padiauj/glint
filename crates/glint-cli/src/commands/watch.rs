//! Watch command - monitor for file changes.

use crate::app::App;
use glint_core::backend::{ChangeHandler, ChangeHandlerMessage, ChannelChangeHandler};
use glint_core::{Config, FileSystemBackend};
use std::sync::Arc;
use tracing::{error, info, warn};

/// Run the watch command.
pub fn run(config: Config, _foreground: bool) -> anyhow::Result<()> {
    let app = App::new(config)?;

    if app.index.is_empty() {
        eprintln!("Index is empty. Run 'glint index' first.");
        return Ok(());
    }

    println!("Starting file change monitoring...");
    println!("Press Ctrl+C to stop.");
    println!();

    // Get volumes to watch
    let volumes = app.index.volume_states();

    if volumes.is_empty() {
        eprintln!("No volumes to watch.");
        return Ok(());
    }

    // Create change handler
    let (handler, receiver) = ChannelChangeHandler::new();
    let handler: Arc<dyn ChangeHandler> = Arc::new(handler);

    // Start watchers for each volume
    let mut watch_handles = Vec::new();

    for vol_state in &volumes {
        let mut volume_info = vol_state.info.clone();
        volume_info.journal_state = vol_state.journal_state.clone();

        match app
            .backend
            .watch_changes(volume_info.clone(), handler.clone())
        {
            Ok(handle) => {
                println!("✓ Watching {}", vol_state.info.mount_point);
                watch_handles.push(handle);
            }
            Err(e) => {
                eprintln!("⚠ Cannot watch {} ({})", vol_state.info.mount_point, e);
            }
        }
    }

    if watch_handles.is_empty() {
        eprintln!("No volumes could be watched. Try running as Administrator.");
        return Ok(());
    }

    println!();
    println!("Monitoring for changes...");

    // Process changes
    let index = app.index.clone();

    loop {
        match receiver.recv() {
            Ok(ChangeHandlerMessage::Change(event)) => {
                info!(
                    kind = %event.kind,
                    file = %event.name,
                    "Change detected"
                );

                // Apply change to index
                index.apply_change(event);

                // Periodically save index
                // In production, this would be debounced
            }
            Ok(ChangeHandlerMessage::JournalReset { volume_id, reason }) => {
                warn!(
                    volume = %volume_id,
                    reason = %reason,
                    "Journal reset, index may be stale"
                );
                index.mark_needs_rescan(&volume_id, &reason);
            }
            Ok(ChangeHandlerMessage::Error { volume_id, error }) => {
                error!(volume = %volume_id, error = %error, "Watch error");
            }
            Err(_) => {
                // Channel closed, all watchers stopped
                break;
            }
        }
    }

    // Save index on exit
    app.save_index()?;

    println!("Monitoring stopped.");
    Ok(())
}

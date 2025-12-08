//! Status command - show index status and statistics.

use crate::app::App;
use glint_core::Config;

/// Run the status command.
pub fn run(config: Config) -> anyhow::Result<()> {
    let app = App::new(config)?;

    let stats = app.index.stats();
    let volumes = app.index.volume_states();

    println!("Glint Index Status");
    println!("==================");
    println!();

    if app.index.is_empty() {
        println!("Index is empty. Run 'glint index' to build the index.");
        return Ok(());
    }

    println!("Summary:");
    println!("  Total files:       {}", stats.total_files);
    println!("  Total directories: {}", stats.total_dirs);
    println!("  Total entries:     {}", stats.total_entries());
    println!(
        "  Total size:        {} bytes ({:.2} GB)",
        stats.total_size,
        stats.total_size as f64 / (1024.0 * 1024.0 * 1024.0)
    );
    println!("  Index version:     {}", stats.version);

    if let Some(updated) = stats.last_updated {
        println!(
            "  Last updated:      {}",
            updated.format("%Y-%m-%d %H:%M:%S")
        );
    }

    println!();
    println!("Indexed Volumes:");

    for vol in &volumes {
        let status = if vol.needs_rescan {
            "⚠ needs rescan"
        } else {
            "✓"
        };
        println!(
            "  {} {} ({} entries) {}",
            vol.info.mount_point,
            vol.info.label.as_deref().unwrap_or(""),
            vol.record_count,
            status
        );

        if let Some(ref js) = vol.journal_state {
            println!("    Journal ID: {:016X}", js.journal_id);
            println!("    Last USN:   {}", js.last_usn);
        }
    }

    // Show data directory
    println!();
    println!("Data directory: {}", app.config.index_dir()?.display());

    Ok(())
}

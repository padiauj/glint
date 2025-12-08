//! Index command - build or rebuild the file index.

use crate::app::App;
use glint_core::Config;
use std::time::Instant;

/// Run the index command.
pub fn run(config: Config, force: bool, volumes: Vec<String>) -> anyhow::Result<()> {
    let app = App::new(config)?;

    // Check if we need to rebuild
    let needs_rebuild = force || app.index.is_empty();

    if !needs_rebuild {
        println!("Index already exists with {} entries.", app.index.len());
        println!("Use --force to rebuild from scratch.");
        return Ok(());
    }

    println!("Building file index...");
    println!();

    // Check for admin privileges
    if glint_backend_ntfs::NtfsBackend::has_elevated_privileges() {
        println!("✓ Running with elevated privileges (MFT access available)");
    } else {
        println!("⚠ Not running as administrator - using fallback scan method");
        println!("  For faster indexing, run as Administrator");
    }
    println!();

    let start = Instant::now();

    app.rebuild_index(&volumes)?;

    let elapsed = start.elapsed();
    let stats = app.index.stats();

    println!();
    println!("Indexing complete!");
    println!("  Files:       {}", stats.total_files);
    println!("  Directories: {}", stats.total_dirs);
    println!("  Volumes:     {}", stats.volume_count);
    println!("  Time:        {:.2}s", elapsed.as_secs_f64());
    println!(
        "  Rate:        {:.0} entries/sec",
        stats.total_entries() as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}

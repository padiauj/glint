//! Query command - search for files.

use crate::app::App;
use crate::OutputFormat;
use glint_core::{search::parse_query, Config, SearchFilter};
use std::time::Instant;

/// Run the query command.
pub fn run(
    config: Config,
    pattern: &str,
    limit: usize,
    files_only: bool,
    dirs_only: bool,
    extensions: Vec<String>,
    search_path: bool,
    output: OutputFormat,
) -> anyhow::Result<()> {
    let app = App::new(config)?;

    if app.index.is_empty() {
        eprintln!("Index is empty. Run 'glint index' first.");
        return Ok(());
    }

    // Parse and build query
    let mut query = parse_query(pattern)?;

    if files_only {
        query = query.with_filter(SearchFilter::FilesOnly);
    } else if dirs_only {
        query = query.with_filter(SearchFilter::DirsOnly);
    }

    if !extensions.is_empty() {
        query = query.with_filter(SearchFilter::Extensions(extensions));
    }

    if search_path {
        query = query.search_in_path(true);
    }

    let start = Instant::now();
    let results = app.index.search_limited(&query, limit);
    let elapsed = start.elapsed();

    match output {
        OutputFormat::Text => {
            for result in &results {
                let record = &result.record;
                let type_indicator = if record.is_dir { "📁" } else { "📄" };

                if let Some(size) = record.size {
                    println!("{} {} ({} bytes)", type_indicator, record.path, size);
                } else {
                    println!("{} {}", type_indicator, record.path);
                }
            }

            eprintln!();
            eprintln!(
                "Found {} results in {:.3}ms",
                results.len(),
                elapsed.as_secs_f64() * 1000.0
            );
        }
        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "name": r.record.name,
                        "path": r.record.path,
                        "is_dir": r.record.is_dir,
                        "size": r.record.size,
                        "modified": r.record.modified.map(|t| t.to_rfc3339()),
                    })
                })
                .collect();

            println!("{}", serde_json::to_string_pretty(&json_results)?);
        }
    }

    Ok(())
}

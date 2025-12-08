//! Clear command - remove all index data.

use glint_core::{Config, IndexStore};
use std::io::{self, Write};

/// Run the clear command.
pub fn run(config: Config, skip_confirm: bool) -> anyhow::Result<()> {
    let data_dir = config.index_dir()?;
    let store = IndexStore::new(&data_dir);

    if !store.exists() {
        println!("No index found. Nothing to clear.");
        return Ok(());
    }

    if !skip_confirm {
        print!("This will delete all index data. Are you sure? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    store.clear()?;
    println!("Index cleared.");

    Ok(())
}

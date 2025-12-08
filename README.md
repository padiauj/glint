# Glint 🔍

**Extremely fast file name search across local NTFS volumes**

Glint is an open source Rust-based file search tool inspired by [Voidtools Everything](https://www.voidtools.com/)'s functionality, designed to provide near-instant file searches across millions of files.

## Features

- ⚡ **Lightning Fast Search**: Indexes millions of files in seconds using NTFS MFT (Master File Table)
- 🔄 **Real-time Updates**: Monitors the USN Change Journal for instant index updates
- 🎯 **Flexible Queries**: Supports substring, wildcard, and regex patterns
- 🖥️ **Interactive TUI**: Beautiful terminal interface with real-time search
- 📦 **Portable Index**: Compressed index file for quick startup
- 🔌 **Extensible Architecture**: Clean abstraction for adding new filesystem backends

## Installation

### From Source

```bash
git clone https://github.com/glint-search/glint.git
cd glint
cargo build --release
```

The binary will be at `target/release/glint.exe`.

### Requirements

- Windows 10/11 (NTFS backend)

## Usage

### Build the Index

First, build the file index. For best performance, run as Administrator:

```powershell
# Run as Administrator for MFT access (fastest)
glint index

# Or without admin (slower, uses directory traversal)
glint index
```

### Search Files

```bash
# Simple substring search
glint query readme

# Wildcard patterns
glint query "*.rs"
glint query "test?.txt"

# Regex patterns
glint query "r/test_\d+\.rs/"

# Filter by type
glint query --files-only "*.log"
glint query --dirs-only src

# Filter by extension
glint query -e rs -e toml config

# Limit results
glint query --limit 50 document
```

### Interactive Mode

Start the interactive TUI for real-time searching:

```bash
glint interactive
# or
glint i
```

**TUI Shortcuts:**
- `↑/↓` - Navigate results
- `PgUp/PgDn` - Page through results
- `Enter` - Open in Explorer
- `F2` - Copy path to clipboard
- `Ctrl+F` - Toggle files only
- `Ctrl+D` - Toggle directories only
- `Esc` - Exit

### Monitor for Changes

Keep the index up-to-date with real-time monitoring:

```bash
# Run as Administrator for USN journal access
glint watch --foreground
```

### Other Commands

```bash
# Show index status
glint status

# Clear the index
glint clear
```

## Query Syntax

| Pattern | Description | Example |
|---------|-------------|---------|
| `text` | Substring match | `readme` matches "README.md" |
| `*` | Match any characters | `*.rs` matches "main.rs" |
| `?` | Match single character | `test?.txt` matches "test1.txt" |
| `r/pattern/` | Regex pattern | `r/test_\d+/` matches "test_123" |
| `ext:rs` | Filter by extension | `config ext:toml` |
| `ext:rs,txt` | Multiple extensions | `doc ext:md,txt` |
| `file:` | Files only | `file: *.log` |
| `dir:` | Directories only | `dir: src` |
| `path:` | Search in full path | `path: users` |
| `in:C:\Users` | Path prefix filter | `in:C:\Projects *.rs` |

## Configuration

Configuration is stored in `%APPDATA%\glint\glint.toml`:

```toml
[general]
auto_start_usn = true
max_results = 10000
log_level = "info"

[exclude]
paths = ["C:\\Windows\\Temp", "C:\\$Recycle.Bin"]
patterns = ["*.tmp", "~$*", "Thumbs.db"]

[performance]
compress_index = true
parallel_search = true

[volumes]
# Empty = index all NTFS volumes
include = []
exclude = ["D:"]
```

## Architecture

Glint is designed with extensibility in mind:

```
crates/
├── glint-core/           # Platform-agnostic core
│   ├── backend.rs        # FileSystemBackend trait
│   ├── index.rs          # In-memory index
│   ├── search.rs         # Query parsing and matching
│   ├── persistence.rs    # Index serialization
│   └── config.rs         # Configuration management
│
├── glint-backend-ntfs/   # Windows NTFS backend
│   ├── mft.rs            # MFT enumeration
│   ├── usn.rs            # USN journal monitoring
│   └── volume.rs         # Volume discovery
│
└── glint-cli/            # CLI and TUI
    ├── commands/         # CLI commands
    └── tui/              # Terminal UI
```

### Adding a New Backend

To add support for a new filesystem (e.g., ext4 on Linux):

1. Create a new crate: `glint-backend-ext4`
2. Implement the `FileSystemBackend` trait
3. Register it in the CLI

```rust
use glint_core::backend::{FileSystemBackend, VolumeInfo, FileRecord};

pub struct Ext4Backend;

impl FileSystemBackend for Ext4Backend {
    fn list_volumes(&self) -> anyhow::Result<Vec<VolumeInfo>> {
        // Enumerate ext4 volumes
    }

    fn full_scan(&self, volume: &VolumeInfo, ...) -> anyhow::Result<Vec<FileRecord>> {
        // Scan using /proc/mounts or similar
    }

    fn watch_changes(&self, volume: VolumeInfo, ...) -> anyhow::Result<WatchHandle> {
        // Use inotify or fanotify
    }
    
    // ...
}
```

## Permissions

For best performance, run Glint as Administrator or grant "Perform Volume Maintenance Tasks" privilege. This enables:

- Direct MFT reading (10x faster indexing)
- USN Change Journal access (real-time updates)

Without elevation, Glint falls back to slower but functional methods.

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run tests: `cargo test --all`
5. Submit a pull request

## License

- MIT license 


## Acknowledgments
- Inspired by [Voidtools Everything](https://www.voidtools.com/)
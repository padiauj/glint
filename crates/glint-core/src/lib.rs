//! # Glint Core Library
//!
//! This crate provides the core indexing, persistence, and search functionality
//! for the Glint file search tool. It is designed to be platform-agnostic, with
//! all filesystem-specific operations abstracted behind traits.
//!
//! ## Architecture
//!
//! - **Traits** (`backend`): Define the interface for filesystem backends
//! - **Types** (`types`): Core data types for file records and volume info
//! - **Index** (`index`): In-memory index with fast search capabilities
//! - **Search** (`search`): Query parsing and matching logic
//! - **Persistence** (`persistence`): On-disk storage of the index
//! - **Config** (`config`): Configuration management
//!
//! ## Example
//!
//! ```rust,ignore
//! use glint_core::{Index, SearchQuery, FileSystemBackend};
//!
//! // Create or load an index
//! let mut index = Index::new();
//!
//! // Perform a search
//! let query = SearchQuery::substring("myfile");
//! for result in index.search(&query).take(100) {
//!     println!("{}", result.path);
//! }
//! ```

pub mod backend;
pub mod config;
pub mod error;
pub mod index;
pub mod persistence;
pub mod search;
pub mod types;

// Re-export commonly used types
pub use backend::{ChangeEvent, ChangeHandler, ChangeKind, FileSystemBackend, VolumeInfo};
pub use config::Config;
pub use error::{GlintError, Result};
pub use index::Index;
pub use persistence::IndexStore;
pub use search::{SearchFilter, SearchQuery, SearchResult};
pub use types::{FileId, FileRecord, VolumeId};

//! Search functionality for Glint.
//!
//! This module provides fast search capabilities over the file index, including:
//! - Substring matching (case-insensitive)
//! - Wildcard/glob patterns (*, ?)
//! - Regular expression matching
//! - Filtering by type (file/directory) and extension
//!
//! ## Performance
//!
//! The search implementation is designed for efficiency over millions of entries:
//! - Uses parallel iteration via Rayon for multi-core scaling
//! - Returns results as an iterator for incremental display
//! - Pre-computes lowercase names for fast case-insensitive matching

use crate::error::{GlintError, Result};
use crate::types::FileRecord;
use regex::Regex;
use std::sync::Arc;

/// A compiled search query ready for matching.
///
/// Queries are compiled once and can be reused for multiple searches.
/// The compilation validates patterns and creates optimized matchers.
#[derive(Clone)]
pub struct SearchQuery {
    /// The matcher implementation
    matcher: Arc<dyn Matcher>,

    /// Optional filters to apply after matching
    filters: Vec<SearchFilter>,

    /// Whether to search in paths (true) or just filenames (false)
    search_path: bool,
}

impl std::fmt::Debug for SearchQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchQuery")
            .field("filters", &self.filters)
            .field("search_path", &self.search_path)
            .finish()
    }
}

impl SearchQuery {
    /// Create a substring search query (case-insensitive).
    ///
    /// This is the most common search type, matching any file whose name
    /// contains the given string.
    ///
    /// # Example
    /// ```
    /// use glint_core::SearchQuery;
    /// let query = SearchQuery::substring("readme");
    /// ```
    pub fn substring(pattern: &str) -> Self {
        SearchQuery {
            matcher: Arc::new(SubstringMatcher::new(pattern)),
            filters: Vec::new(),
            search_path: false,
        }
    }

    /// Create a wildcard/glob pattern search query.
    ///
    /// Supports `*` (match any sequence) and `?` (match single character).
    ///
    /// # Example
    /// ```
    /// use glint_core::SearchQuery;
    /// let query = SearchQuery::wildcard("*.rs").unwrap();
    /// ```
    pub fn wildcard(pattern: &str) -> Result<Self> {
        let matcher = WildcardMatcher::new(pattern)?;
        Ok(SearchQuery {
            matcher: Arc::new(matcher),
            filters: Vec::new(),
            search_path: false,
        })
    }

    /// Create a regex search query.
    ///
    /// Uses the `regex` crate for pattern matching.
    ///
    /// # Example
    /// ```
    /// use glint_core::SearchQuery;
    /// let query = SearchQuery::regex(r"test_\d+\.rs").unwrap();
    /// ```
    pub fn regex(pattern: &str) -> Result<Self> {
        let re =
            Regex::new(&format!("(?i){}", pattern)).map_err(|e| GlintError::InvalidPattern {
                pattern: pattern.to_string(),
                reason: e.to_string(),
            })?;
        Ok(SearchQuery {
            matcher: Arc::new(RegexMatcher { regex: re }),
            filters: Vec::new(),
            search_path: false,
        })
    }

    /// Create an "exact name" search (case-insensitive).
    pub fn exact(name: &str) -> Self {
        SearchQuery {
            matcher: Arc::new(ExactMatcher::new(name)),
            filters: Vec::new(),
            search_path: false,
        }
    }

    /// Add a filter to the query.
    ///
    /// Filters are applied after pattern matching to further narrow results.
    pub fn with_filter(mut self, filter: SearchFilter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Set whether to search in full paths instead of just filenames.
    pub fn search_in_path(mut self, search_path: bool) -> Self {
        self.search_path = search_path;
        self
    }

    /// Check if a record matches this query.
    ///
    /// First applies the pattern matcher, then all filters.
    pub fn matches(&self, record: &FileRecord) -> bool {
        // Get the text to search in
        let text = if self.search_path {
            &record.path
        } else {
            &record.name_lower
        };

        // Apply pattern matcher
        if !self.matcher.matches(text, record) {
            return false;
        }

        // Apply all filters
        self.filters.iter().all(|f| f.matches(record))
    }

    /// Check if this query would match everything (empty pattern)
    pub fn matches_all(&self) -> bool {
        self.matcher.matches_all() && self.filters.is_empty()
    }
}

/// Filters to narrow search results.
#[derive(Debug, Clone)]
pub enum SearchFilter {
    /// Only match files (not directories)
    FilesOnly,

    /// Only match directories (not files)
    DirsOnly,

    /// Only match files with specific extensions
    Extensions(Vec<String>),

    /// Exclude files with specific extensions
    ExcludeExtensions(Vec<String>),

    /// Only match files larger than this size
    MinSize(u64),

    /// Only match files smaller than this size
    MaxSize(u64),

    /// Only match files in this path prefix
    PathPrefix(String),

    /// Exclude files in this path prefix
    ExcludePath(String),
}

impl SearchFilter {
    /// Check if a record matches this filter.
    pub fn matches(&self, record: &FileRecord) -> bool {
        match self {
            SearchFilter::FilesOnly => !record.is_dir,
            SearchFilter::DirsOnly => record.is_dir,
            SearchFilter::Extensions(exts) => record.extension().map_or(false, |e| {
                exts.iter().any(|ext| e.eq_ignore_ascii_case(ext))
            }),
            SearchFilter::ExcludeExtensions(exts) => record.extension().map_or(true, |e| {
                !exts.iter().any(|ext| e.eq_ignore_ascii_case(ext))
            }),
            SearchFilter::MinSize(size) => record.size.map_or(false, |s| s >= *size),
            SearchFilter::MaxSize(size) => record.size.map_or(true, |s| s <= *size),
            SearchFilter::PathPrefix(prefix) => record
                .path
                .to_lowercase()
                .starts_with(&prefix.to_lowercase()),
            SearchFilter::ExcludePath(prefix) => !record
                .path
                .to_lowercase()
                .starts_with(&prefix.to_lowercase()),
        }
    }
}

/// A search result with relevance scoring.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matching file record
    pub record: FileRecord,

    /// Relevance score (higher is more relevant)
    pub score: u32,
}

impl SearchResult {
    /// Create a new search result
    pub fn new(record: FileRecord, score: u32) -> Self {
        SearchResult { record, score }
    }
}

// === Matcher Implementations ===

/// Trait for pattern matching implementations.
trait Matcher: Send + Sync {
    /// Check if the given text matches this pattern.
    ///
    /// The `record` parameter is provided for matchers that need additional
    /// context (though most just use `text`).
    fn matches(&self, text: &str, record: &FileRecord) -> bool;

    /// Returns true if this matcher matches everything
    fn matches_all(&self) -> bool {
        false
    }
}

/// Case-insensitive substring matcher.
struct SubstringMatcher {
    pattern_lower: String,
}

impl SubstringMatcher {
    fn new(pattern: &str) -> Self {
        SubstringMatcher {
            pattern_lower: pattern.to_lowercase(),
        }
    }
}

impl Matcher for SubstringMatcher {
    fn matches(&self, text: &str, _record: &FileRecord) -> bool {
        if self.pattern_lower.is_empty() {
            return true;
        }
        text.to_lowercase().contains(&self.pattern_lower)
    }

    fn matches_all(&self) -> bool {
        self.pattern_lower.is_empty()
    }
}

/// Exact name matcher (case-insensitive).
struct ExactMatcher {
    pattern_lower: String,
}

impl ExactMatcher {
    fn new(pattern: &str) -> Self {
        ExactMatcher {
            pattern_lower: pattern.to_lowercase(),
        }
    }
}

impl Matcher for ExactMatcher {
    fn matches(&self, text: &str, _record: &FileRecord) -> bool {
        text.to_lowercase() == self.pattern_lower
    }
}

/// Wildcard pattern matcher.
///
/// Converts glob patterns to regex for matching.
struct WildcardMatcher {
    regex: Regex,
}

impl WildcardMatcher {
    fn new(pattern: &str) -> Result<Self> {
        // Convert glob pattern to regex
        let mut regex_pattern = String::with_capacity(pattern.len() * 2 + 4);
        regex_pattern.push_str("(?i)^");

        for c in pattern.chars() {
            match c {
                '*' => regex_pattern.push_str(".*"),
                '?' => regex_pattern.push('.'),
                // Escape regex special characters
                '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(c);
                }
                _ => regex_pattern.push(c),
            }
        }

        regex_pattern.push('$');

        let regex = Regex::new(&regex_pattern).map_err(|e| GlintError::InvalidPattern {
            pattern: pattern.to_string(),
            reason: e.to_string(),
        })?;

        Ok(WildcardMatcher { regex })
    }
}

impl Matcher for WildcardMatcher {
    fn matches(&self, text: &str, _record: &FileRecord) -> bool {
        self.regex.is_match(text)
    }
}

/// Regular expression matcher.
struct RegexMatcher {
    regex: Regex,
}

impl Matcher for RegexMatcher {
    fn matches(&self, text: &str, _record: &FileRecord) -> bool {
        self.regex.is_match(text)
    }
}

// === Query Parsing ===

/// Parse a query string into a SearchQuery.
///
/// Supports various query formats:
/// - Simple text: `readme` (substring search)
/// - Wildcard: `*.rs` (glob pattern)
/// - Regex: `r/pattern/` (regex search)
/// - With filters: `*.rs ext:rs,txt file:`
///
/// # Query Syntax
///
/// - `pattern` - Search for files containing "pattern" (case-insensitive)
/// - `*.txt` - Wildcard pattern (matches files ending in .txt)
/// - `r/regex/` - Regular expression pattern
/// - `ext:rs` - Filter by extension
/// - `ext:rs,txt,md` - Filter by multiple extensions
/// - `file:` - Only show files (not directories)
/// - `dir:` - Only show directories
/// - `path:` - Search in full path, not just filename
pub fn parse_query(input: &str) -> Result<SearchQuery> {
    let input = input.trim();

    if input.is_empty() {
        return Ok(SearchQuery::substring(""));
    }

    let mut search_path = false;
    let mut filters = Vec::new();
    let mut pattern_parts = Vec::new();

    // Parse the query into parts
    for part in input.split_whitespace() {
        if let Some(exts) = part.strip_prefix("ext:") {
            let extensions: Vec<String> = exts
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !extensions.is_empty() {
                filters.push(SearchFilter::Extensions(extensions));
            }
        } else if part == "file:" || part == "files:" {
            filters.push(SearchFilter::FilesOnly);
        } else if part == "dir:" || part == "dirs:" || part == "folder:" {
            filters.push(SearchFilter::DirsOnly);
        } else if part == "path:" {
            search_path = true;
        } else if let Some(prefix) = part.strip_prefix("in:") {
            filters.push(SearchFilter::PathPrefix(prefix.to_string()));
        } else {
            pattern_parts.push(part);
        }
    }

    let pattern = pattern_parts.join(" ");

    // Determine query type from pattern
    let mut query = if pattern.starts_with("r/") && pattern.ends_with('/') && pattern.len() > 3 {
        // Regex pattern
        let regex_pattern = &pattern[2..pattern.len() - 1];
        SearchQuery::regex(regex_pattern)?
    } else if pattern.contains('*') || pattern.contains('?') {
        // Wildcard pattern
        SearchQuery::wildcard(&pattern)?
    } else {
        // Default: substring search
        SearchQuery::substring(&pattern)
    };

    // Apply filters
    for filter in filters {
        query = query.with_filter(filter);
    }

    query = query.search_in_path(search_path);

    Ok(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileId, VolumeId};

    fn make_record(name: &str, is_dir: bool) -> FileRecord {
        FileRecord::new(
            FileId::new(1),
            None,
            VolumeId::new("C"),
            name.to_string(),
            format!("C:\\{}", name),
            is_dir,
        )
    }

    #[test]
    fn test_substring_search() {
        let query = SearchQuery::substring("readme");

        assert!(query.matches(&make_record("README.md", false)));
        assert!(query.matches(&make_record("readme.txt", false)));
        assert!(query.matches(&make_record("MyReadmeFile.txt", false)));
        assert!(!query.matches(&make_record("other.txt", false)));
    }

    #[test]
    fn test_empty_substring() {
        let query = SearchQuery::substring("");
        assert!(query.matches_all());
        assert!(query.matches(&make_record("anything.txt", false)));
    }

    #[test]
    fn test_wildcard_search() {
        let query = SearchQuery::wildcard("*.rs").unwrap();

        assert!(query.matches(&make_record("main.rs", false)));
        assert!(query.matches(&make_record("lib.RS", false))); // Case insensitive
        assert!(!query.matches(&make_record("main.rs.bak", false)));
        assert!(!query.matches(&make_record("readme.md", false)));
    }

    #[test]
    fn test_wildcard_question_mark() {
        let query = SearchQuery::wildcard("test?.txt").unwrap();

        assert!(query.matches(&make_record("test1.txt", false)));
        assert!(query.matches(&make_record("testA.txt", false)));
        assert!(!query.matches(&make_record("test.txt", false)));
        assert!(!query.matches(&make_record("test12.txt", false)));
    }

    #[test]
    fn test_regex_search() {
        let query = SearchQuery::regex(r"test_\d+\.rs").unwrap();

        assert!(query.matches(&make_record("test_123.rs", false)));
        assert!(query.matches(&make_record("TEST_1.RS", false))); // Case insensitive
        assert!(!query.matches(&make_record("test_abc.rs", false)));
    }

    #[test]
    fn test_filter_files_only() {
        let query = SearchQuery::substring("").with_filter(SearchFilter::FilesOnly);

        assert!(query.matches(&make_record("file.txt", false)));
        assert!(!query.matches(&make_record("folder", true)));
    }

    #[test]
    fn test_filter_dirs_only() {
        let query = SearchQuery::substring("").with_filter(SearchFilter::DirsOnly);

        assert!(!query.matches(&make_record("file.txt", false)));
        assert!(query.matches(&make_record("folder", true)));
    }

    #[test]
    fn test_filter_extensions() {
        let query = SearchQuery::substring("").with_filter(SearchFilter::Extensions(vec![
            "rs".to_string(),
            "toml".to_string(),
        ]));

        assert!(query.matches(&make_record("main.rs", false)));
        assert!(query.matches(&make_record("Cargo.toml", false)));
        assert!(query.matches(&make_record("test.RS", false))); // Case insensitive
        assert!(!query.matches(&make_record("readme.md", false)));
    }

    #[test]
    fn test_filter_size() {
        let mut record = make_record("file.txt", false);
        record.size = Some(1000);

        let query = SearchQuery::substring("").with_filter(SearchFilter::MinSize(500));
        assert!(query.matches(&record));

        let query = SearchQuery::substring("").with_filter(SearchFilter::MinSize(2000));
        assert!(!query.matches(&record));
    }

    #[test]
    fn test_parse_query_simple() {
        let query = parse_query("readme").unwrap();
        assert!(query.matches(&make_record("README.md", false)));
    }

    #[test]
    fn test_parse_query_with_extension() {
        let query = parse_query("test ext:rs").unwrap();

        assert!(query.matches(&make_record("test.rs", false)));
        assert!(!query.matches(&make_record("test.txt", false)));
    }

    #[test]
    fn test_parse_query_files_only() {
        let query = parse_query("file:").unwrap();

        assert!(query.matches(&make_record("anything.txt", false)));
        assert!(!query.matches(&make_record("folder", true)));
    }

    #[test]
    fn test_parse_query_wildcard() {
        let query = parse_query("*.rs").unwrap();

        assert!(query.matches(&make_record("main.rs", false)));
        assert!(!query.matches(&make_record("main.txt", false)));
    }

    #[test]
    fn test_parse_query_regex() {
        let query = parse_query("r/test_\\d+/").unwrap();

        assert!(query.matches(&make_record("test_123.rs", false)));
        assert!(!query.matches(&make_record("test_abc.rs", false)));
    }

    #[test]
    fn test_exact_match() {
        let query = SearchQuery::exact("README.md");

        assert!(query.matches(&make_record("README.md", false)));
        assert!(query.matches(&make_record("readme.md", false))); // Case insensitive
        assert!(!query.matches(&make_record("README.md.bak", false)));
        assert!(!query.matches(&make_record("my-README.md", false)));
    }

    #[test]
    fn test_invalid_regex() {
        let result = SearchQuery::regex("[invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_search_in_path() {
        let query = SearchQuery::substring("users").search_in_path(true);

        let mut record = make_record("file.txt", false);
        record.path = "C:\\Users\\test\\file.txt".to_string();

        assert!(query.matches(&record));
    }
}

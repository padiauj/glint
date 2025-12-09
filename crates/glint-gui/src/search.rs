//! GUI search state wrapper around glint_core search.

use glint_core::{Index, SearchQuery};
use glint_core::archive_view::ArchivedView;
use glint_core::search::SearchResult;
use std::sync::Arc;
use std::time::{Duration, Instant};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::thread;
use arc_swap::ArcSwap;

struct SearchRequest {
    id: u64,
    query: SearchQuery,
    max_results: usize,
    archived: Option<Arc<ArchivedView>>,
}

struct SearchDone {
    id: u64,
    results: Vec<SearchResult>,
    took: Duration,
}

pub struct SearchState {
    pub query: String,
    pub files_only: bool,
    pub dirs_only: bool,
    pub case_sensitive: bool,
    pub use_regex: bool,
    pub max_results: usize,
    pub results: Vec<SearchResult>,
    pub selected: usize,
    pub search_time: Duration,
    pub scroll_to_selected: bool,
    pub error: Option<String>,
    shared_index: Arc<ArcSwap<Arc<Index>>>,
    archived_view: Option<Arc<ArchivedView>>,

    // Change detection and debounce
    dirty: bool,
    last_input_at: Instant,
    debounce: Duration,
    last_query: String,
    last_files_only: bool,
    last_dirs_only: bool,
    last_use_regex: bool,
    last_index_generation: u64,

    // Async search worker
    req_tx: Sender<SearchRequest>,
    done_rx: Receiver<SearchDone>,
    in_flight: bool,
    last_request_id: u64,
    latest_applied_id: u64,

    // Incremental narrowing cache
    prev_query: String,
    prev_results: Vec<SearchResult>,
}

impl SearchState {
    pub fn new(index: Arc<Index>) -> Self {
        // Spawn background search worker
        let (req_tx, req_rx) = unbounded::<SearchRequest>();
        let (done_tx, done_rx) = unbounded::<SearchDone>();
        let shared_index = Arc::new(ArcSwap::from(Arc::new(Arc::clone(&index))));
        let worker_shared = Arc::clone(&shared_index);
        thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let start = Instant::now();
                // Load the current index snapshot
                let idx = worker_shared.load_full();
                // Prefer archived view if provided
                let mut results = if let Some(view) = req.archived.clone() {
                    // Unsafe root reference lives as long as mmap
                    let root = unsafe { view.root() };
                    let mut out = Vec::with_capacity(req.max_results);
                    // Simple linear scan over names (lowercased)
                    for i in 0..root.is_dir.len() {
                        let noff = root.name_offsets[i] as usize;
                        // read cstr from names_blob
                        let name = cstr_from_bytes_local(&root.names_blob[noff..]);
                        // Build minimal record for matcher
                        let rec = glint_core::types::FileRecord::new(
                            glint_core::types::FileId::new(i as u64 + 1),
                            None,
                            glint_core::types::VolumeId::new("V"),
                            name.to_string(),
                            String::new(),
                            root.is_dir[i] != 0,
                        );
                        if req.query.matches(&rec) {
                            out.push(glint_core::search::SearchResult::new(rec, 0));
                            if out.len() >= req.max_results { break; }
                        }
                    }
                    out
                } else {
                    // Default path: use in-memory index
                    idx.search_limited(&req.query, req.max_results)
                };
                let _ = done_tx.send(SearchDone {
                    id: req.id,
                    results,
                    took: start.elapsed(),
                });
            }
        });

        Self {
            query: String::new(),
            files_only: false,
            dirs_only: false,
            case_sensitive: false,
            use_regex: false,
            max_results: 5000,
            results: Vec::new(),
            selected: 0,
            search_time: Duration::from_millis(0),
            scroll_to_selected: false,
            error: None,
            shared_index,
            dirty: false,
            last_input_at: Instant::now(),
            debounce: Duration::from_millis(120),
            last_query: String::new(),
            last_files_only: false,
            last_dirs_only: false,
            last_use_regex: false,
            last_index_generation: 0,
            req_tx,
            done_rx,
            in_flight: false,
            last_request_id: 0,
            latest_applied_id: 0,
            prev_query: String::new(),
            prev_results: Vec::new(),
            archived_view: None,
        }
    }

    pub fn set_index(&mut self, index: Arc<Index>) {
        self.shared_index.store(Arc::new(index));
        self.mark_dirty();
    }

    pub fn set_archived_view(&mut self, view: Arc<ArchivedView>) {
        self.archived_view = Some(view);
        self.mark_dirty();
    }

    fn current_generation(&self) -> u64 {
        self.shared_index.load().generation()
    }

    pub fn is_in_flight(&self) -> bool {
        self.in_flight
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_input_at = Instant::now();
    }

    pub fn should_search(&self, index_generation: u64) -> bool {
        if self.in_flight {
            return false;
        }
        if self.query.len() < 2 {
            return false;
        }

        if !self.dirty {
            return false;
        }

        if self.last_input_at.elapsed() < self.debounce {
            return false;
        }

        // If the index changed since last run, allow search
        if index_generation != self.last_index_generation {
            return true;
        }

        // If inputs changed since last run, allow search
        if self.query != self.last_query
            || self.files_only != self.last_files_only
            || self.dirs_only != self.last_dirs_only
            || self.use_regex != self.last_use_regex
        {
            return true;
        }

        false
    }

    pub fn search(&mut self) {
        self.error = None;

        // Build query
        let mut query = if self.use_regex {
            match glint_core::search::parse_query(&format!("r/{}/", self.query)) {
                Ok(q) => q,
                Err(e) => {
                    self.error = Some(format!("Invalid regex: {}", e));
                    self.results.clear();
                    return;
                }
            }
        } else if self.query.contains('*') || self.query.contains('?') {
            match SearchQuery::wildcard(&self.query) {
                Ok(q) => q,
                Err(e) => {
                    self.error = Some(format!("Invalid pattern: {}", e));
                    self.results.clear();
                    return;
                }
            }
        } else {
            SearchQuery::substring(&self.query)
        };

        if self.files_only {
            query = query.with_filter(glint_core::search::SearchFilter::FilesOnly);
        }
        if self.dirs_only {
            query = query.with_filter(glint_core::search::SearchFilter::DirsOnly);
        }

        // If the new query is a simple extension of the previous query and filters are unchanged,
        // try incremental narrowing by filtering previous results on the UI thread for snappy feedback.
        if !self.prev_query.is_empty()
            && self.query.len() > self.prev_query.len()
            && self.query.starts_with(&self.prev_query)
            && self.files_only == self.last_files_only
            && self.dirs_only == self.last_dirs_only
            && self.use_regex == self.last_use_regex
        {
            let start = Instant::now();
            // Build matcher for the new query
            let narrowed_query = if self.use_regex {
                match glint_core::search::parse_query(&format!("r/{}/", self.query)) {
                    Ok(q) => q,
                    Err(e) => {
                        self.error = Some(format!("Invalid regex: {}", e));
                        return;
                    }
                }
            } else if self.query.contains('*') || self.query.contains('?') {
                match SearchQuery::wildcard(&self.query) {
                    Ok(q) => q,
                    Err(e) => {
                        self.error = Some(format!("Invalid pattern: {}", e));
                        return;
                    }
                }
            } else {
                SearchQuery::substring(&self.query)
            };

            let mut filtered = Vec::with_capacity(self.max_results.min(self.prev_results.len()));
            for r in self.prev_results.iter() {
                if narrowed_query.matches(&r.record) {
                    filtered.push(r.clone());
                    if filtered.len() >= self.max_results { break; }
                }
            }
            self.results = filtered;
            self.selected = 0.min(self.results.len().saturating_sub(1));
            self.search_time = start.elapsed();
            // Keep in_flight false; still dispatch a background full search for correctness
        }

        // Dispatch async search request (authoritative)
        self.last_request_id = self.last_request_id.wrapping_add(1);
        let id = self.last_request_id;
        let max_results = self.max_results;
        let archived = self.archived_view.clone();
        if self.req_tx.send(SearchRequest { id, query, max_results, archived }).is_ok() {
            self.in_flight = true;
        }
    }
}

fn cstr_from_bytes_local(bytes: &[u8]) -> &str {
    let mut end = 0;
    while end < bytes.len() && bytes[end] != 0 { end += 1; }
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

impl SearchState {
    pub fn poll_results(&mut self) {
        while let Ok(done) = self.done_rx.try_recv() {
            if done.id >= self.latest_applied_id {
                self.results = done.results;
                self.selected = 0.min(self.results.len().saturating_sub(1));
                self.search_time = done.took;
                self.latest_applied_id = done.id;
                self.in_flight = false;
                self.prev_query = self.last_query.clone();
                self.prev_results = self.results.clone();

                // Update last-run snapshot
                self.last_query = self.query.clone();
                self.last_files_only = self.files_only;
                self.last_dirs_only = self.dirs_only;
                self.last_use_regex = self.use_regex;
                self.last_index_generation = self.current_generation();
                self.dirty = false;
            }
        }
    }

    pub fn clear(&mut self) {
        self.results.clear();
        self.selected = 0;
        self.error = None;
    }

    pub fn select_previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.scroll_to_selected = true;
        }
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.results.len() {
            self.selected += 1;
            self.scroll_to_selected = true;
        }
    }

    pub fn page_up(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows);
        self.scroll_to_selected = true;
    }

    pub fn page_down(&mut self, rows: usize) {
        self.selected = (self.selected + rows).min(self.results.len().saturating_sub(1));
        self.scroll_to_selected = true;
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
        self.scroll_to_selected = true;
    }

    pub fn select_last(&mut self) {
        if !self.results.is_empty() {
            self.selected = self.results.len() - 1;
            self.scroll_to_selected = true;
        }
    }

    pub fn open_selected(&self) {
        if let Some(result) = self.results.get(self.selected) {
            let _ = open::that(&result.record.path);
        }
    }

    pub fn copy_selected_path(&self) -> Result<(), String> {
        if let Some(result) = self.results.get(self.selected) {
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard
                .set_text(result.record.path.clone())
                .map_err(|e| e.to_string())?;
            Ok(())
        } else {
            Err("No selection".into())
        }
    }
}

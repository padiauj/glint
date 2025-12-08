//! Interactive TUI (Terminal User Interface) for Glint.
//!
//! Provides a responsive search interface with:
//! - Real-time search as you type
//! - Navigation through results
//! - Quick actions (open in Explorer, copy path)

use crate::app::App;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use glint_core::{search::parse_query, Config, SearchFilter, SearchResult};
use ratatui::{prelude::*, widgets::*};
use std::io;
use std::time::{Duration, Instant};

/// TUI application state.
struct TuiApp {
    /// The main application
    app: App,

    /// Current search query string
    query_string: String,

    /// Current search results
    results: Vec<SearchResult>,

    /// Selected result index
    selected: usize,

    /// Vertical scroll offset
    scroll_offset: usize,

    /// Whether we should quit
    should_quit: bool,

    /// Last search time
    last_search_time: Duration,

    /// Status message
    status_message: Option<String>,

    /// Show files only
    files_only: bool,

    /// Show dirs only
    dirs_only: bool,
}

impl TuiApp {
    fn new(app: App) -> Self {
        TuiApp {
            app,
            query_string: String::new(),
            results: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            should_quit: false,
            last_search_time: Duration::ZERO,
            status_message: None,
            files_only: false,
            dirs_only: false,
        }
    }

    /// Perform a search with the current query.
    fn search(&mut self) {
        let start = Instant::now();

        let result = parse_query(&self.query_string);
        let mut query = match result {
            Ok(q) => q,
            Err(e) => {
                self.status_message = Some(format!("Invalid query: {}", e));
                self.results.clear();
                return;
            }
        };

        // Apply filters
        if self.files_only {
            query = query.with_filter(SearchFilter::FilesOnly);
        } else if self.dirs_only {
            query = query.with_filter(SearchFilter::DirsOnly);
        }

        self.results = self.app.index.search_limited(&query, 1000);
        self.last_search_time = start.elapsed();

        // Reset selection
        self.selected = 0;
        self.scroll_offset = 0;
        self.status_message = None;
    }

    /// Handle input character.
    fn on_char(&mut self, c: char) {
        self.query_string.push(c);
        self.search();
    }

    /// Handle backspace.
    fn on_backspace(&mut self) {
        self.query_string.pop();
        self.search();
    }

    /// Move selection up.
    fn select_previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down.
    fn select_next(&mut self) {
        if self.selected + 1 < self.results.len() {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    /// Page up.
    fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
        self.ensure_visible();
    }

    /// Page down.
    fn page_down(&mut self, page_size: usize) {
        self.selected = (self.selected + page_size).min(self.results.len().saturating_sub(1));
        self.ensure_visible();
    }

    /// Ensure selected item is visible.
    fn ensure_visible(&mut self) {
        // This will be set properly based on visible area
        let visible_height = 20; // Approximate

        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected - visible_height + 1;
        }
    }

    /// Open selected file in Explorer.
    fn open_selected(&self) {
        if let Some(result) = self.results.get(self.selected) {
            let path = &result.record.path;
            // Open in Explorer and select the file
            let _ = std::process::Command::new("explorer")
                .arg("/select,")
                .arg(path)
                .spawn();
        }
    }

    /// Copy path to clipboard.
    fn copy_path(&mut self) {
        if let Some(result) = self.results.get(self.selected) {
            // On Windows, use clip command
            let path = &result.record.path;
            let _ = std::process::Command::new("cmd")
                .args(["/C", "echo", path, "|", "clip"])
                .spawn();
            self.status_message = Some("Path copied to clipboard".to_string());
        }
    }

    /// Toggle files-only filter.
    fn toggle_files_only(&mut self) {
        self.files_only = !self.files_only;
        self.dirs_only = false;
        self.search();
    }

    /// Toggle dirs-only filter.
    fn toggle_dirs_only(&mut self) {
        self.dirs_only = !self.dirs_only;
        self.files_only = false;
        self.search();
    }
}

/// Run the TUI application.
pub fn run(config: Config) -> anyhow::Result<()> {
    let app = App::new(config)?;

    if app.index.is_empty() {
        eprintln!("Index is empty. Run 'glint index' first.");
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut tui_app = TuiApp::new(app);

    // Initial search (empty = show some results)
    tui_app.search();

    // Main loop
    let result = run_loop(&mut terminal, &mut tui_app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Main event loop.
fn run_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut TuiApp) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Esc => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        KeyCode::Char(c) => {
                            if key.modifiers.contains(KeyModifiers::CONTROL) {
                                match c {
                                    'f' => app.toggle_files_only(),
                                    'd' => app.toggle_dirs_only(),
                                    _ => {}
                                }
                            } else {
                                app.on_char(c);
                            }
                        }
                        KeyCode::Backspace => {
                            app.on_backspace();
                        }
                        KeyCode::Up => {
                            app.select_previous();
                        }
                        KeyCode::Down => {
                            app.select_next();
                        }
                        KeyCode::PageUp => {
                            app.page_up(10);
                        }
                        KeyCode::PageDown => {
                            app.page_down(10);
                        }
                        KeyCode::Home => {
                            app.selected = 0;
                            app.scroll_offset = 0;
                        }
                        KeyCode::End => {
                            if !app.results.is_empty() {
                                app.selected = app.results.len() - 1;
                                app.ensure_visible();
                            }
                        }
                        KeyCode::Enter => {
                            app.open_selected();
                        }
                        KeyCode::F(2) => {
                            app.copy_path();
                        }
                        _ => {}
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

mod ui {
    use super::*;

    /// Draw the UI.
    pub fn draw(f: &mut Frame, app: &mut TuiApp) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Search box
                Constraint::Min(10),   // Results
                Constraint::Length(2), // Status bar
            ])
            .split(f.area());

        draw_search_box(f, app, chunks[0]);
        draw_results(f, app, chunks[1]);
        draw_status_bar(f, app, chunks[2]);
    }

    /// Draw the search input box.
    fn draw_search_box(f: &mut Frame, app: &TuiApp, area: Rect) {
        let input = Paragraph::new(app.query_string.as_str())
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 🔍 Search (type to filter) "),
            );
        f.render_widget(input, area);

        // Show cursor
        f.set_cursor_position(Position::new(
            area.x + app.query_string.len() as u16 + 1,
            area.y + 1,
        ));
    }

    /// Draw the results list.
    fn draw_results(f: &mut Frame, app: &mut TuiApp, area: Rect) {
        let visible_height = area.height.saturating_sub(2) as usize;

        // Update scroll offset based on visible height
        if app.selected >= app.scroll_offset + visible_height {
            app.scroll_offset = app.selected - visible_height + 1;
        }

        let items: Vec<ListItem> = app
            .results
            .iter()
            .skip(app.scroll_offset)
            .take(visible_height)
            .enumerate()
            .map(|(i, result)| {
                let record = &result.record;
                let icon = if record.is_dir { "📁" } else { "📄" };

                let size_str = record.size.map(|s| format_size(s)).unwrap_or_default();

                let line = format!("{} {} {}", icon, record.path, size_str);

                let style = if i + app.scroll_offset == app.selected {
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                ListItem::new(line).style(style)
            })
            .collect();

        let title = format!(
            " Results ({} found in {:.1}ms) ",
            app.results.len(),
            app.last_search_time.as_secs_f64() * 1000.0
        );

        let results = List::new(items).block(Block::default().borders(Borders::ALL).title(title));

        f.render_widget(results, area);
    }

    /// Draw the status bar.
    fn draw_status_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
        let stats = app.app.index.stats();

        let filters = {
            let mut parts = Vec::new();
            if app.files_only {
                parts.push("Files");
            }
            if app.dirs_only {
                parts.push("Dirs");
            }
            if parts.is_empty() {
                "All".to_string()
            } else {
                parts.join(", ")
            }
        };

        let status = if let Some(ref msg) = app.status_message {
            msg.clone()
        } else {
            format!(
                "Index: {} files, {} dirs | Filter: {} | ↑↓:Navigate Enter:Open F2:Copy Esc:Quit Ctrl+F:Files Ctrl+D:Dirs",
                stats.total_files, stats.total_dirs, filters
            )
        };

        let status_bar = Paragraph::new(status).style(Style::default().fg(Color::Gray));

        f.render_widget(status_bar, area);
    }

    /// Format a file size.
    fn format_size(size: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if size >= GB {
            format!("{:.1} GB", size as f64 / GB as f64)
        } else if size >= MB {
            format!("{:.1} MB", size as f64 / MB as f64)
        } else if size >= KB {
            format!("{:.1} KB", size as f64 / KB as f64)
        } else {
            format!("{} B", size)
        }
    }
}

//! UI components for the Glint GUI.

use crate::app::{format_number, format_size, GlintApp};
use crate::service::ServiceStatus;
use eframe::egui::{self, Color32, RichText, Sense};

// Local helper function
fn format_volume_size(bytes: u64) -> String {
    format_size(bytes)
}

/// Menu bar at the top of the window
pub fn menu_bar(ctx: &egui::Context, app: &mut GlintApp) {
    egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            // File menu
            ui.menu_button("File", |ui| {
                if ui.button("Build Index...").clicked() {
                    app.show_index_builder = true;
                    ui.close_menu();
                }
                if ui.button("Reload Index (F5)").clicked() {
                    app.reload_index();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Settings...").clicked() {
                    app.show_settings = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            // Service menu
            ui.menu_button("Service", |ui| {
                let status = app.service_status;
                let status_text = match status {
                    ServiceStatus::NotInstalled => "⚪ Not Installed",
                    ServiceStatus::Stopped => "🔴 Stopped",
                    ServiceStatus::Running => "🟢 Running",
                    ServiceStatus::Unknown => "❓ Unknown",
                };
                ui.label(format!("Status: {}", status_text));
                ui.separator();

                let toggle_text = match status {
                    ServiceStatus::NotInstalled => "Install & Start Service",
                    ServiceStatus::Stopped => "Start Service",
                    ServiceStatus::Running => "Stop Service",
                    ServiceStatus::Unknown => "Refresh Status",
                };

                if ui.button(toggle_text).clicked() {
                    if status == ServiceStatus::Unknown {
                        app.refresh_service_status();
                    } else {
                        app.toggle_service();
                    }
                    ui.close_menu();
                }

                if status != ServiceStatus::NotInstalled {
                    ui.separator();
                    if ui.button("Uninstall Service").clicked() {
                        if let Err(e) = crate::service::request_elevation_for_service("uninstall") {
                            app.status_message = format!("Failed: {}", e);
                        }
                        ui.close_menu();
                    }
                }

                ui.separator();
                if ui.button("Refresh Status").clicked() {
                    app.refresh_service_status();
                    ui.close_menu();
                }
            });

            // Help menu
            ui.menu_button("Help", |ui| {
                if ui.button("About...").clicked() {
                    app.show_about = true;
                    ui.close_menu();
                }
            });
        });
    });
}

/// Top panel with search bar and controls.
pub fn top_panel(ctx: &egui::Context, app: &mut GlintApp) {
    egui::TopBottomPanel::top("search_panel").show(ctx, |ui| {
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            // Search icon
            ui.label(RichText::new("🔍").size(18.0));

            // Search input
            let response = ui.add_sized(
                [ui.available_width() - 150.0, 28.0],
                egui::TextEdit::singleline(&mut app.search.query)
                    .hint_text("Search files... (type at least 2 characters)")
                    .font(egui::TextStyle::Heading),
            );

            if response.changed() {
                app.search.mark_dirty();
            }

            // Focus search box on startup or Ctrl+L
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                app.search.search();
            }

            // Auto-search as you type
            if app.search.should_search(app.index.generation()) {
                app.search.search();
            }

            // Request focus with Ctrl+L
            if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::L)) {
                response.request_focus();
            }

            // Clear button
            if ui.button("✕").on_hover_text("Clear search (Esc)").clicked() {
                app.search.query.clear();
                app.search.clear();
            }

            // Settings button
            if ui.button("⚙").on_hover_text("Settings (Ctrl+,)").clicked() {
                app.show_settings = !app.show_settings;
            }

            // About button
            if ui.button("?").on_hover_text("About").clicked() {
                app.show_about = !app.show_about;
            }
        });

        ui.add_space(4.0);

        // Filter row
        ui.horizontal(|ui| {
            if ui.checkbox(&mut app.search.files_only, "Files only").changed() {
                app.search.dirs_only &= !app.search.files_only;
                app.search.mark_dirty();
            }
            if app.search.files_only {
                app.search.dirs_only = false;
            }

            if ui.checkbox(&mut app.search.dirs_only, "Folders only").changed() {
                app.search.files_only &= !app.search.dirs_only;
                app.search.mark_dirty();
            }
            if app.search.dirs_only {
                app.search.files_only = false;
            }

            ui.separator();

            if ui.checkbox(&mut app.search.case_sensitive, "Case sensitive").changed() {
                app.search.mark_dirty();
            }
            if ui.checkbox(&mut app.search.use_regex, "Regex").changed() {
                app.search.mark_dirty();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !app.search.results.is_empty() {
                    ui.label(format!(
                        "{} results in {:.1}ms",
                        format_number(app.search.results.len()),
                        app.search.search_time.as_secs_f64() * 1000.0
                    ));
                }
            });
        });

        ui.add_space(4.0);
    });
}

/// Bottom status bar.
pub fn bottom_panel(ctx: &egui::Context, app: &mut GlintApp) {
    egui::TopBottomPanel::bottom("bottom_panel")
        .exact_height(24.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Status message
                ui.label(&app.status_message);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(
                            "F5: Reload | Ctrl+L: Focus | Enter: Open | Ctrl+C: Copy Path",
                        )
                        .small()
                        .color(Color32::GRAY),
                    );
                });
            });
        });
}

/// Central panel with search results.
pub fn central_panel(ctx: &egui::Context, app: &mut GlintApp) {
    egui::CentralPanel::default().show(ctx, |ui| {
        // Handle keyboard navigation
        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            app.search.select_previous();
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            app.search.select_next();
        }
        if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
            app.search.page_up(20);
        }
        if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
            app.search.page_down(20);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Home) && i.modifiers.ctrl) {
            app.search.select_first();
        }
        if ui.input(|i| i.key_pressed(egui::Key::End) && i.modifiers.ctrl) {
            app.search.select_last();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            app.search.open_selected();
        }
        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::C)) {
            if let Err(e) = app.search.copy_selected_path() {
                app.status_message = format!("Failed to copy: {}", e);
            } else {
                app.status_message = "Path copied to clipboard".to_string();
            }
        }

        // Show error if any
        if let Some(error) = &app.search.error {
            ui.colored_label(Color32::RED, error);
            return;
        }

        // Show empty state
        if app.search.results.is_empty() {
            ui.centered_and_justified(|ui| {
                if app.search.query.is_empty() {
                    ui.label(
                        RichText::new("Start typing to search files...")
                            .size(18.0)
                            .color(Color32::GRAY),
                    );
                } else if app.search.query.len() < 2 {
                    ui.label(
                        RichText::new("Type at least 2 characters to search")
                            .size(18.0)
                            .color(Color32::GRAY),
                    );
                } else {
                    ui.label(
                        RichText::new("No results found")
                            .size(18.0)
                            .color(Color32::GRAY),
                    );
                }
            });
            return;
        }

        // Results list with virtual scrolling
        let row_height = 24.0;
        let total_rows = app.search.results.len();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_height, total_rows, |ui, row_range| {
                // Scroll to selected if needed
                if app.search.scroll_to_selected && row_range.contains(&app.search.selected) {
                    app.search.scroll_to_selected = false;
                }

                for row in row_range {
                    if let Some(result) = app.search.results.get(row) {
                        let record = &result.record;
                        let is_selected = row == app.search.selected;

                        // Row background
                        let bg_color = if is_selected {
                            Color32::from_rgb(0, 120, 212) // Blue selection
                        } else if row % 2 == 0 {
                            Color32::from_gray(30)
                        } else {
                            Color32::from_gray(35)
                        };

                        let text_color = if is_selected {
                            Color32::WHITE
                        } else {
                            Color32::from_gray(200)
                        };

                        let secondary_color = if is_selected {
                            Color32::from_gray(220)
                        } else {
                            Color32::from_gray(128)
                        };

                        // Draw row
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_height),
                            Sense::click(),
                        );

                        if ui.is_rect_visible(rect) {
                            ui.painter().rect_filled(rect, 0.0, bg_color);

                            // Icon
                            let icon = if record.is_dir { "📁" } else { "📄" };
                            let icon_rect = egui::Rect::from_min_size(
                                rect.min + egui::vec2(8.0, 2.0),
                                egui::vec2(20.0, 20.0),
                            );
                            ui.painter().text(
                                icon_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                icon,
                                egui::FontId::proportional(14.0),
                                text_color,
                            );

                            // Filename
                            let name_rect = egui::Rect::from_min_max(
                                rect.min + egui::vec2(32.0, 0.0),
                                egui::pos2(rect.min.x + 280.0, rect.max.y),
                            );
                            ui.painter().text(
                                name_rect.left_center(),
                                egui::Align2::LEFT_CENTER,
                                &record.name,
                                egui::FontId::proportional(13.0),
                                text_color,
                            );

                            // Path (directory part)
                            let path_dir = std::path::Path::new(&record.path)
                                .parent()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let path_rect = egui::Rect::from_min_max(
                                egui::pos2(rect.min.x + 290.0, rect.min.y),
                                egui::pos2(rect.max.x - 200.0, rect.max.y),
                            );
                            ui.painter().text(
                                path_rect.left_center(),
                                egui::Align2::LEFT_CENTER,
                                &path_dir,
                                egui::FontId::proportional(12.0),
                                secondary_color,
                            );

                            // Size (for files)
                            if !record.is_dir {
                                if let Some(size) = record.size {
                                    let size_rect = egui::Rect::from_min_max(
                                        egui::pos2(rect.max.x - 190.0, rect.min.y),
                                        egui::pos2(rect.max.x - 120.0, rect.max.y),
                                    );
                                    ui.painter().text(
                                        size_rect.right_center(),
                                        egui::Align2::RIGHT_CENTER,
                                        format_size(size),
                                        egui::FontId::proportional(12.0),
                                        secondary_color,
                                    );
                                }
                            }

                            // Modified date
                            if let Some(modified) = record.modified {
                                let date_rect = egui::Rect::from_min_max(
                                    egui::pos2(rect.max.x - 110.0, rect.min.y),
                                    egui::pos2(rect.max.x - 8.0, rect.max.y),
                                );
                                ui.painter().text(
                                    date_rect.right_center(),
                                    egui::Align2::RIGHT_CENTER,
                                    modified.format("%Y-%m-%d %H:%M").to_string(),
                                    egui::FontId::proportional(12.0),
                                    secondary_color,
                                );
                            }
                        }

                        // Handle clicks
                        if response.clicked() {
                            app.search.selected = row;
                        }
                        if response.double_clicked() {
                            app.search.open_selected();
                        }

                        // Copy the name for use in context menu (avoids borrow issues)
                        let record_name = record.name.clone();

                        // Context menu
                        response.context_menu(|ui| {
                            if ui.button("Open in Explorer").clicked() {
                                app.search.selected = row;
                                app.search.open_selected();
                                ui.close_menu();
                            }
                            if ui.button("Copy Path").clicked() {
                                app.search.selected = row;
                                if let Err(e) = app.search.copy_selected_path() {
                                    app.status_message = format!("Failed to copy: {}", e);
                                } else {
                                    app.status_message = "Path copied to clipboard".to_string();
                                }
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Copy Name").clicked() {
                                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                    let _ = clipboard.set_text(&record_name);
                                    app.status_message = "Name copied to clipboard".to_string();
                                }
                                ui.close_menu();
                            }
                        });
                    }
                }
            });
    });
}

/// Settings window.
pub fn settings_window(ctx: &egui::Context, app: &mut GlintApp) {
    let mut show = app.show_settings;
    egui::Window::new("Settings")
        .open(&mut show)
        .resizable(true)
        .default_width(450.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Appearance");
                ui.checkbox(&mut app.dark_mode, "Dark mode");

                ui.add_space(10.0);
                ui.separator();

                ui.heading("Search");
                ui.horizontal(|ui| {
                    ui.label("Max results:");
                    ui.add(
                        egui::DragValue::new(&mut app.search.max_results)
                            .range(100..=100000)
                            .speed(100),
                    );
                });

                ui.add_space(10.0);
                ui.separator();

                ui.heading("Index");
                let stats = app.index.stats();
                ui.label(format!(
                    "Files: {}",
                    format_number(stats.total_files as usize)
                ));
                ui.label(format!(
                    "Directories: {}",
                    format_number(stats.total_dirs as usize)
                ));
                ui.label(format!("Volumes: {}", stats.volume_count));

                ui.add_space(10.0);

                if ui.button("Reload Index (F5)").clicked() {
                    app.reload_index();
                }

                ui.add_space(10.0);
                ui.separator();

                ui.heading("Excluded Folders");
                ui.label("These folders will be skipped during indexing:");

                // Show current exclusions
                let mut to_remove: Option<usize> = None;
                for (i, path) in app.config.exclude.paths.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("📁 {}", path));
                        if ui.small_button("✕").on_hover_text("Remove").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }

                // Remove if requested
                if let Some(idx) = to_remove {
                    app.config.exclude.paths.remove(idx);
                    if let Err(e) = app.config.save() {
                        app.status_message = format!("Failed to save config: {}", e);
                    }
                }

                // Add folder button with native picker
                if ui.button("➕ Add Excluded Folder...").clicked() {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Select folder to exclude from indexing")
                        .pick_folder()
                    {
                        let path_str = folder.to_string_lossy().to_string();
                        if !app.config.exclude.paths.contains(&path_str) {
                            app.config.exclude.paths.push(path_str);
                            if let Err(e) = app.config.save() {
                                app.status_message = format!("Failed to save config: {}", e);
                            } else {
                                app.status_message =
                                    "Folder added to exclusions. Re-index to apply.".to_string();
                            }
                        }
                    }
                }

                ui.add_space(10.0);
                ui.separator();

                ui.heading("Index Location");
                let index_path = app
                    .config
                    .index_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "Unknown".to_string());
                ui.horizontal(|ui| {
                    ui.label(&index_path);
                    if ui
                        .small_button("📂")
                        .on_hover_text("Open in Explorer")
                        .clicked()
                    {
                        let _ = open::that(&index_path);
                    }
                });
            });
        });
    app.show_settings = show;
}

/// About window.
pub fn about_window(ctx: &egui::Context, app: &mut GlintApp) {
    let mut show = app.show_about;
    egui::Window::new("About Glint")
        .open(&mut show)
        .resizable(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Glint");
                ui.label("Fast File Search");
                ui.add_space(10.0);
                ui.label("Version 0.1.0");
                ui.add_space(10.0);
                ui.label("A blazingly fast file search tool");
                ui.label("inspired by Voidtools Everything.");
                ui.add_space(10.0);
                ui.hyperlink_to("GitHub Repository", "https://github.com/padiauj/glint");
                ui.add_space(10.0);
                ui.label("Licensed under MIT or Apache-2.0");
            });
        });
    app.show_about = show;
}

/// Index builder window for first run or rebuilding index.
pub fn index_builder_window(ctx: &egui::Context, app: &mut GlintApp) {
    let mut show = app.show_index_builder;
    egui::Window::new("Build Index")
        .open(&mut show)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.heading("Welcome to Glint!");
            ui.add_space(10.0);

            ui.label("Select volumes to index:");
            ui.add_space(5.0);

            // List available NTFS volumes
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    for volume in &mut app.available_volumes {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut volume.selected, "");
                            ui.label(format!(
                                "{} ({}) - {}",
                                volume.letter,
                                volume.label,
                                format_size(volume.size)
                            ));
                        });
                    }
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Service option
            ui.checkbox(
                &mut app.enable_service_on_index,
                "Enable background service for real-time updates",
            );
            ui.label(
                egui::RichText::new(
                    "The background service monitors file changes and keeps the index up-to-date.",
                )
                .small()
                .weak(),
            );

            ui.add_space(15.0);

            // Build button
            ui.horizontal(|ui| {
                if ui.button("Build Index").clicked() {
                    // Collect selected volumes
                    let selected: Vec<char> = app
                        .available_volumes
                        .iter()
                        .filter(|v| v.selected)
                        .map(|v| v.letter)
                        .collect();

                    if !selected.is_empty() {
                        // Update settings with selected volumes
                        app.settings.indexed_volumes = selected.clone();
                        if let Err(e) = app.settings.save() {
                            app.status_message = format!("Failed to save settings: {}", e);
                        }

                        // Trigger index rebuild
                        app.index_volumes();

                        // Install and start service if requested
                        if app.enable_service_on_index {
                            #[cfg(windows)]
                            {
                                use crate::service;
                                if let Err(e) = service::install_service() {
                                    app.status_message =
                                        format!("Failed to install service: {}", e);
                                } else if let Err(e) = service::start_service() {
                                    app.status_message =
                                        format!("Service installed but failed to start: {}", e);
                                } else {
                                    app.refresh_service_status();
                                }
                            }
                        }

                        app.show_index_builder = false;
                    } else {
                        app.status_message = "Please select at least one volume".to_string();
                    }
                }

                if ui.button("Cancel").clicked() {
                    app.show_index_builder = false;
                }
            });
        });
    app.show_index_builder = show;
}

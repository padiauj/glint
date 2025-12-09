#![cfg_attr(windows, windows_subsystem = "windows")]
//! Glint GUI - Cross-platform graphical interface for Glint.
//!
//! This application provides a fast, responsive search interface using egui.
//! It's designed to work on Windows, macOS, and Linux without external dependencies.
//!
//! ## Self-Installation
//!
//! On Windows, the executable is self-installing:
//! - First run: Installs to LocalAppData/Programs/Glint
//! - Creates Start Menu shortcut
//! - Registers in Add/Remove Programs
//! - Running a newer version automatically updates

mod app;
mod installer;
mod search;
mod service;
mod settings;
mod ui;

use app::GlintApp;
use eframe::egui;
use std::env;

fn main() -> eframe::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_ansi(false)
        .init();

    // Handle command-line arguments
    let args: Vec<String> = env::args().collect();
    
    if args.len() > 1 {
        match args[1].as_str() {
            "--uninstall" => {
                if let Err(e) = installer::uninstall() {
                    eprintln!("Uninstall failed: {}", e);
                    std::process::exit(1);
                }
                println!("Glint has been uninstalled.");
                std::process::exit(0);
            }
            "--service-install" => {
                // Elevated process for service installation
                if let Err(e) = service::install_service() {
                    eprintln!("Service install failed: {}", e);
                    std::process::exit(1);
                }
                if let Err(e) = service::start_service() {
                    eprintln!("Service start failed: {}", e);
                }
                std::process::exit(0);
            }
            "--service-uninstall" => {
                let _ = service::stop_service();
                if let Err(e) = service::uninstall_service() {
                    eprintln!("Service uninstall failed: {}", e);
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            "--service-start" => {
                if let Err(e) = service::start_service() {
                    eprintln!("Service start failed: {}", e);
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            "--service-stop" => {
                if let Err(e) = service::stop_service() {
                    eprintln!("Service stop failed: {}", e);
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("Glint - Fast File Search");
                println!();
                println!("Usage: glint-gui [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --uninstall          Uninstall Glint");
                println!("  --service-install    Install background service (requires admin)");
                println!("  --service-uninstall  Uninstall background service (requires admin)");
                println!("  --service-start      Start background service");
                println!("  --service-stop       Stop background service");
                println!("  --help, -h           Show this help");
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown option: {}", args[1]);
                std::process::exit(1);
            }
        }
    }

    // Perform silent installation/update on Windows
    #[cfg(windows)]
    {
        match installer::install_or_update() {
            Ok(true) => {
                tracing::info!("Installation/update completed");
            }
            Ok(false) => {
                tracing::debug!("No installation needed");
            }
            Err(e) => {
                tracing::warn!("Installation failed (continuing anyway): {}", e);
            }
        }
    }

    tracing::info!("Starting Glint GUI");

    // Configure native options
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([600.0, 400.0])
            .with_title("Glint - Fast File Search")
            .with_icon(load_icon()),
        ..Default::default()
    };

    // Run the application
    eframe::run_native(
        "Glint",
        options,
        Box::new(|cc| Ok(Box::new(GlintApp::new(cc)))),
    )
}

/// Load application icon (returns default if not found)
fn load_icon() -> egui::IconData {
    // Default icon data (a simple magnifying glass shape encoded as RGBA)
    // In production, you'd load from an actual icon file
    egui::IconData::default()
}

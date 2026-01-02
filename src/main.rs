//! TruthDB Installer
//!
//! A minimal installer executable designed to run in initramfs environment.
//! Displays a simple framebuffer UI and handles keyboard input.

mod app;
mod input;
mod platform;
mod ui;

use anyhow::{Context, Result};
use std::process;
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info};

use app::App;
use ui::UiBackend;

/// Main entry point
fn main() {
    // Initialize logging to stdout/stderr
    tracing_subscriber::fmt::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_level(true)
        .init();

    info!("TruthDB Installer starting...");

    // Run the application and handle any errors
    match run() {
        Ok(()) => {
            info!("TruthDB Installer exiting cleanly");
            process::exit(0);
        }
        Err(e) => {
            error!("Fatal error: {:#}", e);
            eprintln!("\nFATAL ERROR: {:#}", e);
            process::exit(1);
        }
    }
}

/// Main application logic
fn run() -> Result<()> {
    // Create application state machine
    let mut app = App::new();

    // Initialize UI backend
    info!("Initializing UI backend...");
    app.log_step("[..] Initializing UI");
    let mut ui = ui::create_backend().context("Failed to create UI backend")?;

    ui.init().context("Failed to initialize UI backend")?;
    app.log_step("[OK] UI initialized");
    render_frame(&app, &mut *ui)?;

    // Initialize input handler
    info!("Initializing input handler...");
    app.log_step("[..] Initializing input");
    let mut input = input::create_handler().context("Failed to create input handler")?;

    input.init().context("Failed to initialize input handler")?;
    app.log_step("[OK] Input initialized (press Q to quit for now)");
    render_frame(&app, &mut *ui)?;

    // Transition from BootSplash to Welcome
    app.initialize().context("Failed to initialize application")?;

    // Render initial screen
    render_frame(&app, &mut *ui)?;

    info!("Entering main loop...");

    // Main event loop
    loop {
        // Poll for input
        match input.poll() {
            Ok(Some(ch)) => {
                debug!("Received input: '{}'", ch);
                app.handle_input(ch)?;

                if ch == 'q' || ch == 'Q' {
                    app.log_step("[..] Exit requested");
                }

                // Re-render after input
                render_frame(&app, &mut *ui)?;
            }
            Ok(None) => {
                // No input, continue
            }
            Err(e) => {
                error!("Input error: {}", e);
                app.handle_error(format!("Input error: {}", e));
                render_frame(&app, &mut *ui)?;
            }
        }

        // Check if we should exit
        if app.should_exit() {
            info!("Exit requested");
            break;
        }

        // Small delay to avoid busy-waiting
        thread::sleep(Duration::from_millis(50));
    }

    // Cleanup
    info!("Cleaning up...");
    input.cleanup().context("Failed to cleanup input handler")?;
    ui.cleanup().context("Failed to cleanup UI backend")?;

    Ok(())
}

/// Render a frame with current application state
fn render_frame(app: &App, ui: &mut dyn UiBackend) -> Result<()> {
    // Clear screen to dark blue
    ui.clear(0, 0, 64)?;

    // Get text to display
    let lines = app.get_display_text();

    // Render text
    ui.render_text(&lines)?;

    // Present the frame
    ui.present()?;

    Ok(())
}

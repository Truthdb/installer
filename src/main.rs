//! TruthDB Installer
//!
//! A minimal installer executable designed to run in initramfs environment.
//! Displays a simple framebuffer UI and handles keyboard input.

mod app;
mod input;
mod platform;
mod ui;

use anyhow::{Context, Result};
use std::path::Path;
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

    // MVP milestone: enumerate disks and enforce "abort if multiple eligible disks".
    app.log_step("[..] Enumerating eligible disks");
    render_frame(&app, &mut *ui)?;
    let target_disk = match platform::disks::DiskScanner::new_default().choose_single_target_disk()
    {
        Ok(disk) => {
            app.log_step(format!(
                "[OK] Target disk: {} ({} bytes)",
                disk.dev_path.display(),
                disk.size_bytes
            ));
            Some(disk)
        }
        Err(e) => {
            app.handle_error(format!("Disk selection failed: {e:#}"));
            None
        }
    };

    if let Some(disk) = target_disk {
        let payload_path = Path::new("/payload/debian-minbase-amd64-bookworm.tar.zst");
        app.log_step("[..] Checking Debian rootfs payload");
        render_frame(&app, &mut *ui)?;
        if !payload_path.exists() {
            app.handle_error(format!("Missing rootfs payload: {}", payload_path.display()));
            // Do not perform destructive disk operations without a payload to install.
            render_frame(&app, &mut *ui)?;
            return Ok(());
        }
        app.log_step("[OK] Rootfs payload present");

        app.log_step("[..] Wiping disk signatures (wipefs)");
        render_frame(&app, &mut *ui)?;
        if let Err(e) = platform::partition::wipefs_all(&disk.dev_path) {
            app.handle_error(format!("wipefs failed: {e:#}"));
        } else {
            app.log_step("[OK] Signatures wiped");
        }

        app.log_step("[..] Partitioning disk (GPT: ESP+root)");
        render_frame(&app, &mut *ui)?;
        if let Err(e) = platform::partition::partition_gpt_esp_root(
            &disk.dev_path,
            platform::partition::PartitionPlan::default(),
        ) {
            app.handle_error(format!("Partitioning failed: {e:#}"));
        } else {
            app.log_step("[OK] Disk partitioned");
            if let Ok((esp, root)) =
                platform::partition::expected_esp_and_root_partitions(&disk.dev_path)
            {
                app.log_step(format!("[OK] ESP partition: {}", esp.display()));
                app.log_step(format!("[OK] Root partition: {}", root.display()));

                app.log_step("[..] Formatting partitions (vfat+ext4)");
                render_frame(&app, &mut *ui)?;
                if let Err(e) = platform::install::format_partitions(&esp, &root) {
                    app.handle_error(format!("Formatting failed: {e:#}"));
                    render_frame(&app, &mut *ui)?;
                    return Ok(());
                } else {
                    app.log_step("[OK] Partitions formatted");

                    app.log_step("[..] Mounting target filesystem");
                    render_frame(&app, &mut *ui)?;
                    let mount_plan = platform::install::MountPlan::default();
                    if let Err(e) = platform::install::mount_partitions(&esp, &root, &mount_plan) {
                        app.handle_error(format!("Mount failed: {e:#}"));
                        render_frame(&app, &mut *ui)?;
                        return Ok(());
                    } else {
                        app.log_step(format!(
                            "[OK] Mounted root at {}",
                            mount_plan.target_root.display()
                        ));

                        app.log_step("[..] Extracting Debian rootfs payload");
                        render_frame(&app, &mut *ui)?;
                        if let Err(e) = platform::install::extract_rootfs_payload(
                            payload_path,
                            &mount_plan.target_root,
                        ) {
                            app.handle_error(format!("Extract failed: {e:#}"));
                            render_frame(&app, &mut *ui)?;
                            let _ = platform::install::unmount_target(&mount_plan);
                            return Ok(());
                        } else {
                            app.log_step("[OK] Rootfs extracted");

                            app.log_step("[..] Setting hostname to truthdb01");
                            render_frame(&app, &mut *ui)?;
                            if let Err(e) =
                                platform::install::configure_hostname(&mount_plan, "truthdb01")
                            {
                                app.handle_error(format!("Hostname setup failed: {e:#}"));
                                render_frame(&app, &mut *ui)?;
                                let _ = platform::install::unmount_target(&mount_plan);
                                return Ok(());
                            } else {
                                app.log_step("[OK] Hostname configured");
                            }

                            app.log_step(
                                "[..] Creating initial user (truthdb) + setting passwords",
                            );
                            render_frame(&app, &mut *ui)?;
                            if let Err(e) = platform::install::configure_initial_users(&mount_plan)
                            {
                                app.handle_error(format!("User setup failed: {e:#}"));
                                render_frame(&app, &mut *ui)?;
                                let _ = platform::install::unmount_target(&mount_plan);
                                return Ok(());
                            } else {
                                app.log_step("[OK] User/password configured");
                            }

                            app.log_step("[..] Enabling DHCP networking (systemd-networkd)");
                            render_frame(&app, &mut *ui)?;
                            if let Err(e) =
                                platform::install::configure_first_boot_dhcp(&mount_plan)
                            {
                                app.handle_error(format!("Networking setup failed: {e:#}"));
                                render_frame(&app, &mut *ui)?;
                                let _ = platform::install::unmount_target(&mount_plan);
                                return Ok(());
                            } else {
                                app.log_step("[OK] Networking configured (DHCP on boot)");
                            }

                            app.log_step("[..] Installing bootloader (systemd-boot)");
                            render_frame(&app, &mut *ui)?;
                            if let Err(e) = platform::install::configure_boot_systemd_boot(
                                &disk.dev_path,
                                &esp,
                                &root,
                                &mount_plan,
                            ) {
                                app.handle_error(format!("Boot config failed: {e:#}"));
                                render_frame(&app, &mut *ui)?;
                                let _ = platform::install::unmount_target(&mount_plan);
                                return Ok(());
                            } else {
                                app.log_step("[OK] Boot configured");

                                app.log_step("[..] Syncing disks");
                                render_frame(&app, &mut *ui)?;
                                if let Err(e) = platform::install::sync_disks() {
                                    app.handle_error(format!("Sync failed: {e:#}"));
                                    render_frame(&app, &mut *ui)?;
                                    let _ = platform::install::unmount_target(&mount_plan);
                                    return Ok(());
                                } else {
                                    app.log_step("[OK] Disks synced");

                                    app.log_step("[..] Unmounting target");
                                    render_frame(&app, &mut *ui)?;
                                    if let Err(e) = platform::install::unmount_target(&mount_plan) {
                                        app.handle_error(format!("Unmount failed: {e:#}"));
                                        render_frame(&app, &mut *ui)?;
                                        return Ok(());
                                    } else {
                                        app.log_step("[OK] Unmounted target");
                                        app.log_step(
                                            "[OK] Install complete (reboot and remove ISO)",
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

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

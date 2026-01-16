//! TruthDB Installer
//!
//! Simplified console-only installer:
//! - Output: stdout only (single channel)
//! - Input: stdin only (blocking prompts)

mod platform;

use anyhow::Result;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::Command;

fn main() {
    if let Err(e) = run() {
        // Keep output on the same channel.
        println!("[ERR] Fatal error: {e:#}");
        let _ = std::io::stdout().flush();
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    println!("TruthDB Installer starting...");
    let _ = std::io::stdout().flush();

    let mut had_error = false;

    println!("[..] Enumerating eligible disks");
    let _ = std::io::stdout().flush();
    let target_disk = match platform::disks::DiskScanner::new_default().choose_single_target_disk()
    {
        Ok(disk) => {
            println!("[OK] Target disk: {} ({} bytes)", disk.dev_path.display(), disk.size_bytes);
            Some(disk)
        }
        Err(e) => {
            println!("[ERR] Disk selection failed: {e:#}");
            had_error = true;
            None
        }
    };
    let _ = std::io::stdout().flush();

    'install: {
        let Some(disk) = target_disk else {
            break 'install;
        };

        prompt_enter(&format!(
            "[!!] About to PARTITION+FORMAT this disk: {}\n[!!] Press ENTER to continue",
            disk.dev_path.display()
        ))?;

        let payload_path = Path::new("/payload/debian-minbase-amd64-bookworm.tar.zst");
        println!("[..] Checking Debian rootfs payload");
        if !payload_path.exists() {
            println!("[ERR] Missing rootfs payload: {}", payload_path.display());
            had_error = true;
            break 'install;
        }
        println!("[OK] Rootfs payload present");
        let _ = std::io::stdout().flush();

        println!("[..] Wiping disk signatures (wipefs)");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::partition::wipefs_all(&disk.dev_path) {
            println!("[ERR] wipefs failed: {e:#}");
            had_error = true;
            break 'install;
        }
        println!("[OK] Signatures wiped");

        println!("[..] Partitioning disk (GPT: ESP+root)");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::partition::partition_gpt_esp_root(
            &disk.dev_path,
            platform::partition::PartitionPlan::default(),
        ) {
            println!("[ERR] Partitioning failed: {e:#}");
            had_error = true;
            break 'install;
        }
        println!("[OK] Disk partitioned");

        let (esp, root) =
            match platform::partition::expected_esp_and_root_partitions(&disk.dev_path) {
                Ok(paths) => paths,
                Err(e) => {
                    println!("[ERR] Could not compute partition paths: {e:#}");
                    had_error = true;
                    break 'install;
                }
            };
        println!("[OK] ESP partition: {}", esp.display());
        println!("[OK] Root partition: {}", root.display());

        println!("[..] Formatting partitions (vfat+ext4)");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::install::format_partitions(&esp, &root) {
            println!("[ERR] Formatting failed: {e:#}");
            had_error = true;
            break 'install;
        }
        println!("[OK] Partitions formatted");

        println!("[..] Mounting target filesystem");
        let _ = std::io::stdout().flush();
        let mount_plan = platform::install::MountPlan::default();
        if let Err(e) = platform::install::mount_partitions(&esp, &root, &mount_plan) {
            println!("[ERR] Mount failed: {e:#}");
            had_error = true;
            break 'install;
        }
        println!("[OK] Mounted root at {}", mount_plan.target_root.display());

        println!("[..] Extracting Debian rootfs payload");
        let _ = std::io::stdout().flush();
        if let Err(e) =
            platform::install::extract_rootfs_payload(payload_path, &mount_plan.target_root)
        {
            println!("[ERR] Extract failed: {e:#}");
            had_error = true;
            let _ = platform::install::unmount_target(&mount_plan);
            break 'install;
        }
        println!("[OK] Rootfs extracted");

        println!("[..] Setting hostname to truthdb01");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::install::configure_hostname(&mount_plan, "truthdb01") {
            println!("[ERR] Hostname setup failed: {e:#}");
            had_error = true;
            let _ = platform::install::unmount_target(&mount_plan);
            break 'install;
        }
        println!("[OK] Hostname configured");

        println!("[..] Creating initial user (truthdb) + setting passwords");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::install::configure_initial_users(&mount_plan) {
            println!("[ERR] User setup failed: {e:#}");
            had_error = true;
            let _ = platform::install::unmount_target(&mount_plan);
            break 'install;
        }
        println!("[OK] User/password configured");

        println!("[..] Enabling DHCP networking (systemd-networkd)");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::install::configure_first_boot_dhcp(&mount_plan) {
            println!("[ERR] Networking setup failed: {e:#}");
            had_error = true;
            let _ = platform::install::unmount_target(&mount_plan);
            break 'install;
        }
        println!("[OK] Networking configured (DHCP on boot)");

        println!("[..] Installing bootloader (systemd-boot)");
        let _ = std::io::stdout().flush();
        if let Err(e) =
            platform::install::configure_boot_systemd_boot(&disk.dev_path, &esp, &root, &mount_plan)
        {
            println!("[ERR] Boot config failed: {e:#}");
            had_error = true;
            let _ = platform::install::unmount_target(&mount_plan);
            break 'install;
        }
        println!("[OK] Boot configured");

        println!("[..] Syncing disks");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::install::sync_disks() {
            println!("[ERR] Sync failed: {e:#}");
            had_error = true;
            let _ = platform::install::unmount_target(&mount_plan);
            break 'install;
        }
        println!("[OK] Disks synced");

        println!("[..] Unmounting target");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::install::unmount_target(&mount_plan) {
            println!("[ERR] Unmount failed: {e:#}");
            had_error = true;
            break 'install;
        }
        println!("[OK] Unmounted target");
        println!("[OK] Install complete (reboot and remove ISO)");
        let _ = std::io::stdout().flush();
    }

    if had_error {
        println!("[ERR] Installer encountered an error");
    } else {
        println!("[OK] Installer finished");
    }
    let _ = std::io::stdout().flush();

    prompt_enter("[!!] Press ENTER to reboot")?;
    reboot_best_effort();

    Ok(())
}

fn prompt_enter(message: &str) -> Result<()> {
    println!("{message}");
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    let mut stdin = std::io::stdin().lock();
    let _ = stdin.read_line(&mut line)?;
    Ok(())
}

fn reboot_best_effort() {
    let _ = Command::new("/bin/busybox").args(["reboot", "-f"]).status();
    let _ = Command::new("reboot").arg("-f").status();
}

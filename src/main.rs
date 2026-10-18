//! TruthDB Installer
//!
//! Simplified console-only installer:
//! - Output: stdout only (single channel)
//! - Input: stdin only (blocking prompts)

mod platform;

use anyhow::{Context, Result, bail};
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

        // Collect every answer before the disk is touched.
        let admin_password = prompt_admin_password()?;

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

        println!("[..] Creating admin user + locking root");
        let _ = std::io::stdout().flush();
        if let Err(e) = platform::install::configure_initial_users(&mount_plan, &admin_password) {
            println!("[ERR] User setup failed: {e:#}");
            had_error = true;
            let _ = platform::install::unmount_target(&mount_plan);
            break 'install;
        }
        println!("[OK] Admin user created, root locked");

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

/// Asks for the admin account's password, twice and without echo, until both entries
/// match.
fn prompt_admin_password() -> Result<String> {
    loop {
        let first = prompt_hidden("[??] Password for the admin account: ")?;
        let second = prompt_hidden("[??] Repeat the password: ")?;
        match validate_new_password(&first, &second) {
            Ok(password) => return Ok(password),
            Err(e) => println!("[ERR] {e}"),
        }
    }
}

fn validate_new_password(first: &str, second: &str) -> Result<String> {
    if first.is_empty() {
        bail!("The password must not be empty");
    }
    if first != second {
        bail!("The passwords do not match");
    }
    Ok(first.to_string())
}

/// Reads one line from stdin with terminal echo turned off.
fn prompt_hidden(message: &str) -> Result<String> {
    print!("{message}");
    let _ = std::io::stdout().flush();

    set_echo(false)?;
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    let restored = set_echo(true);
    println!();

    if read? == 0 {
        bail!("stdin closed while reading the password");
    }
    restored?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn set_echo(on: bool) -> Result<()> {
    let status = Command::new("/bin/busybox")
        .args(["stty", if on { "echo" } else { "-echo" }])
        .status()
        .context("Failed to run stty")?;
    if !status.success() {
        bail!("stty failed: {status}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_must_not_be_empty() {
        assert!(validate_new_password("", "").is_err());
    }

    #[test]
    fn passwords_must_match() {
        assert!(validate_new_password("secret", "secreT").is_err());
    }

    #[test]
    fn matching_password_is_accepted() {
        assert_eq!(validate_new_password("s3 cr:et", "s3 cr:et").unwrap(), "s3 cr:et");
    }
}

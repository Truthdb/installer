//! TruthDB Installer
//!
//! Simplified console-only installer:
//! - Output: stdout only (single channel)
//! - Input: stdin only (best-effort non-blocking)

mod platform;

use anyhow::Result;
use std::io::{Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

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

    // Best-effort: make stdin non-blocking so we can poll for quit without stalling.
    let _stdin_mode = StdinNonBlocking::enable_best_effort();

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
        println!("[ERR] Installer encountered an error; waiting for quit");
    } else {
        println!("[OK] Installer idle; press Q then Enter to exit");
    }
    let _ = std::io::stdout().flush();

    // Idle loop: let the user quit (in initramfs this is useful for debugging logs).
    loop {
        if let Some(ch) = poll_stdin_char_best_effort()
            && (ch == 'q' || ch == 'Q')
        {
            println!("[OK] Exiting");
            let _ = std::io::stdout().flush();
            break;
        }

        thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}

fn poll_stdin_char_best_effort() -> Option<char> {
    let mut buf = [0u8; 16];
    let mut stdin = std::io::stdin().lock();
    match stdin.read(&mut buf) {
        Ok(0) => None,
        Ok(n) => {
            for &b in &buf[..n] {
                let c = b as char;
                if c != '\n' && c != '\r' {
                    return Some(c);
                }
            }
            None
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(_) => None,
    }
}

struct StdinNonBlocking {
    old_flags: i32,
    enabled: bool,
}

impl StdinNonBlocking {
    fn enable_best_effort() -> Self {
        #[cfg(unix)]
        {
            let fd: i32 = 0;
            unsafe {
                let old = libc::fcntl(fd, libc::F_GETFL);
                if old < 0 {
                    return Self { old_flags: 0, enabled: false };
                }
                let new_flags = old | libc::O_NONBLOCK;
                if libc::fcntl(fd, libc::F_SETFL, new_flags) < 0 {
                    return Self { old_flags: old, enabled: false };
                }
                Self { old_flags: old, enabled: true }
            }
        }

        #[cfg(not(unix))]
        {
            Self { old_flags: 0, enabled: false }
        }
    }
}

impl Drop for StdinNonBlocking {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        #[cfg(unix)]
        unsafe {
            let _ = libc::fcntl(0, libc::F_SETFL, self.old_flags);
        }
    }
}

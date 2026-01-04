use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs as unix_fs;

const DEFAULT_PATH: &str = "/bin:/sbin:/usr/bin:/usr/sbin";

#[derive(Debug, Clone)]
pub struct MountPlan {
    pub target_root: PathBuf,
    pub target_efi: PathBuf,
}

impl Default for MountPlan {
    fn default() -> Self {
        Self { target_root: PathBuf::from("/mnt"), target_efi: PathBuf::from("/mnt/boot/efi") }
    }
}

pub fn format_partitions(esp: &Path, root: &Path) -> Result<()> {
    // ESP
    run("mkfs.vfat", &["-F", "32", "-n", "EFI", &esp.display().to_string()])
        .with_context(|| format!("mkfs.vfat failed for {}", esp.display()))?;

    // Root
    run("mkfs.ext4", &["-F", "-L", "root", &root.display().to_string()])
        .with_context(|| format!("mkfs.ext4 failed for {}", root.display()))?;

    Ok(())
}

pub fn mount_partitions(esp: &Path, root: &Path, plan: &MountPlan) -> Result<()> {
    // Ensure /mnt exists in the initramfs, then mount root.
    std::fs::create_dir_all(&plan.target_root)
        .with_context(|| format!("Failed to create {}", plan.target_root.display()))?;

    // Mount root first. Anything created under /mnt before this will be hidden by the mount.
    run(
        "mount",
        &["-t", "ext4", &root.display().to_string(), &plan.target_root.display().to_string()],
    )
    .with_context(|| format!("Failed to mount root {}", root.display()))?;

    // Now create the ESP mountpoint *inside the mounted root*.
    std::fs::create_dir_all(&plan.target_efi)
        .with_context(|| format!("Failed to create {}", plan.target_efi.display()))?;

    // Mount ESP.
    run(
        "mount",
        &["-t", "vfat", &esp.display().to_string(), &plan.target_efi.display().to_string()],
    )
    .with_context(|| format!("Failed to mount ESP {}", esp.display()))?;

    Ok(())
}

pub fn extract_rootfs_payload(payload: &Path, target_root: &Path) -> Result<()> {
    if !payload.exists() {
        return Err(anyhow!("Payload does not exist: {}", payload.display()));
    }

    // Extract payload with permissions/ownership preserved.
    // Note: tar must have zstd support in the initramfs.
    run(
        "tar",
        &[
            "--zstd",
            "-xpf",
            &payload.display().to_string(),
            "-C",
            &target_root.display().to_string(),
        ],
    )
    .with_context(|| {
        format!("Failed to extract payload {} to {}", payload.display(), target_root.display())
    })
}

pub fn configure_initial_users(plan: &MountPlan) -> Result<()> {
    let username = "truthdb";
    let password = "123456";

    // Ensure sudo is present in the payload; otherwise the user won't actually be able to elevate.
    let sudo_path = plan.target_root.join("usr/bin/sudo");
    if !sudo_path.exists() {
        return Err(anyhow!(
            "Target rootfs is missing sudo (expected {}). Ensure payload includes sudo.",
            sudo_path.display()
        ));
    }

    // Ensure required user-management tools exist in the target rootfs.
    for required in [
        plan.target_root.join("usr/sbin/groupadd"),
        plan.target_root.join("usr/sbin/useradd"),
        plan.target_root.join("usr/sbin/chpasswd"),
    ] {
        if !required.exists() {
            return Err(anyhow!("Missing in target rootfs: {}", required.display()));
        }
    }

    // Ensure the sudo group exists (on Debian it's usually created by the sudo package, but keep
    // this resilient).
    chroot_run(&plan.target_root, "/usr/sbin/groupadd", &["-f", "sudo"])
        .context("Failed to ensure sudo group exists")?;

    if !target_user_exists(&plan.target_root, username).unwrap_or(false) {
        // Create a normal user with home dir and bash shell.
        chroot_run(
            &plan.target_root,
            "/usr/sbin/useradd",
            &["-m", "-s", "/bin/bash", "-G", "sudo", username],
        )
        .context("Failed to create truthdb user")?;
    }

    // Set user and root passwords.
    chroot_chpasswd(&plan.target_root, username, password)
        .context("Failed to set truthdb password")?;
    chroot_chpasswd(&plan.target_root, "root", password).context("Failed to set root password")?;

    Ok(())
}

pub fn configure_hostname(plan: &MountPlan, hostname: &str) -> Result<()> {
    let etc_dir = plan.target_root.join("etc");
    std::fs::create_dir_all(&etc_dir)
        .with_context(|| format!("Failed to create {}", etc_dir.display()))?;

    let hostname_path = etc_dir.join("hostname");
    std::fs::write(&hostname_path, format!("{}\n", hostname))
        .with_context(|| format!("Failed to write {}", hostname_path.display()))?;

    let hosts_path = etc_dir.join("hosts");
    let existing = if hosts_path.exists() {
        std::fs::read_to_string(&hosts_path)
            .with_context(|| format!("Failed to read {}", hosts_path.display()))?
    } else {
        String::new()
    };

    let mut out: Vec<String> = Vec::new();
    let mut has_localhost = false;
    let mut wrote_127_0_1_1 = false;

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("127.0.0.1") {
            has_localhost = true;
        }
        if trimmed.starts_with("127.0.1.1") {
            // Replace any existing 127.0.1.1 mapping with ours.
            if !wrote_127_0_1_1 {
                out.push(format!("127.0.1.1\t{}", hostname));
                wrote_127_0_1_1 = true;
            }
            continue;
        }
        out.push(line.to_string());
    }

    if !has_localhost {
        out.push("127.0.0.1\tlocalhost".to_string());
    }
    if !wrote_127_0_1_1 {
        out.push(format!("127.0.1.1\t{}", hostname));
    }

    // Append IPv6 defaults if missing.
    let mut final_hosts = out.join("\n");
    if !final_hosts.ends_with('\n') {
        final_hosts.push('\n');
    }
    if !final_hosts.contains("::1") {
        final_hosts.push_str("::1\tlocalhost ip6-localhost ip6-loopback\n");
        final_hosts.push_str("ff02::1\tip6-allnodes\n");
        final_hosts.push_str("ff02::2\tip6-allrouters\n");
    }

    std::fs::write(&hosts_path, final_hosts)
        .with_context(|| format!("Failed to write {}", hosts_path.display()))?;

    Ok(())
}

pub fn configure_first_boot_dhcp(plan: &MountPlan) -> Result<()> {
    // Configure networking first so DHCP works even if other tweaks fail.
    ensure_machine_id(plan).context("Failed to ensure machine-id")?;
    configure_systemd_networkd_dhcp(plan).context("Failed to configure systemd-networkd DHCP")?;

    // Best-effort: some payloads or usr-merge layouts can make /sbin/init handling surprising.
    // DHCP should not be blocked by this.
    if let Err(e) = ensure_systemd_pid1(plan) {
        eprintln!("WARN: could not ensure systemd is PID 1: {e:#}");
    }
    Ok(())
}

fn ensure_machine_id(plan: &MountPlan) -> Result<()> {
    let machine_id_path = plan.target_root.join("etc/machine-id");
    if let Ok(contents) = std::fs::read_to_string(&machine_id_path)
        && contents.trim().len() >= 32
    {
        return Ok(());
    }

    // Generate a deterministic-length id from the kernel UUID source.
    // /proc should be mounted in the initramfs environment.
    let uuid = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .context("Failed to read /proc/sys/kernel/random/uuid")?;
    let id: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).take(32).collect();
    if id.len() != 32 {
        return Err(anyhow!("Failed to derive a 32-hex machine-id from kernel uuid"));
    }

    if let Some(parent) = machine_id_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::write(&machine_id_path, format!("{}\n", id))
        .with_context(|| format!("Failed to write {}", machine_id_path.display()))?;

    // Debian dbus historically also uses /var/lib/dbus/machine-id.
    // Keep it in sync via symlink when possible.
    let dbus_machine_id = plan.target_root.join("var/lib/dbus/machine-id");
    if let Some(parent) = dbus_machine_id.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    if dbus_machine_id.exists() {
        let _ = std::fs::remove_file(&dbus_machine_id);
    }
    #[cfg(unix)]
    {
        unix_fs::symlink("/etc/machine-id", &dbus_machine_id)
            .with_context(|| format!("Failed to symlink {}", dbus_machine_id.display()))?;
    }

    Ok(())
}

pub fn configure_boot_systemd_boot(
    disk_dev: &Path,
    esp_dev: &Path,
    root_dev: &Path,
    plan: &MountPlan,
) -> Result<()> {
    let root_uuid = blkid_uuid(root_dev).context("Failed to get root UUID")?;
    let esp_uuid = blkid_uuid(esp_dev).context("Failed to get ESP UUID")?;

    write_fstab(&root_uuid, &esp_uuid, plan).context("Failed to write /etc/fstab")?;

    // Install systemd-boot into the mounted ESP.
    install_systemd_boot_efi(&plan.target_efi).context("Failed to install systemd-boot EFI")?;

    // Copy the installed Debian kernel + initrd into ESP so systemd-boot can load them.
    let (kernel_src, initrd_src) = find_installed_kernel_and_initrd(&plan.target_root)
        .context("Failed to locate installed kernel/initrd under /boot")?;

    let kernel_rel = Path::new("EFI/debian/vmlinuz");
    let initrd_rel = Path::new("EFI/debian/initrd.img");
    let kernel_dst = plan.target_efi.join(kernel_rel);
    let initrd_dst = plan.target_efi.join(initrd_rel);

    if let Some(parent) = kernel_dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::copy(&kernel_src, &kernel_dst).with_context(|| {
        format!("Failed to copy kernel {} to {}", kernel_src.display(), kernel_dst.display())
    })?;
    std::fs::copy(&initrd_src, &initrd_dst).with_context(|| {
        format!("Failed to copy initrd {} to {}", initrd_src.display(), initrd_dst.display())
    })?;

    write_systemd_boot_entry(
        &plan.target_efi,
        "/EFI/debian/vmlinuz",
        "/EFI/debian/initrd.img",
        &root_uuid,
    )
    .context("Failed to write systemd-boot entry")?;

    verify_esp_layout(&plan.target_efi).context("ESP does not contain expected boot files")?;

    // Some firmwares/VMs won't auto-scan the fallback path (EFI/BOOT/BOOTX64.EFI) on an internal
    // disk. Create an explicit NVRAM boot entry as well.
    if let Err(e) = register_uefi_boot_entry(disk_dev) {
        eprintln!("WARN: could not register UEFI boot entry (will rely on EFI fallback): {e:#}");
    }

    Ok(())
}

fn configure_systemd_networkd_dhcp(plan: &MountPlan) -> Result<()> {
    // Configure DHCP on first boot using systemd-networkd so we don't depend on interface names
    // being known (enp*, ens*, eth* ...).
    let network_dir = plan.target_root.join("etc/systemd/network");
    std::fs::create_dir_all(&network_dir)
        .with_context(|| format!("Failed to create {}", network_dir.display()))?;

    let dhcp_network = network_dir.join("20-dhcp.network");
    // Match common wired + wifi patterns. Keep it conservative; networking is required for bring-up.
    let contents = "[Match]\nName=en* eth* wl* ww* usb*\n\n[Network]\nDHCP=yes\nIPv6AcceptRA=yes\n";
    std::fs::write(&dhcp_network, contents)
        .with_context(|| format!("Failed to write {}", dhcp_network.display()))?;

    // Enable systemd-networkd (+ optional companions) offline (symlinks under /etc/systemd/system).
    enable_systemd_unit(plan, "systemd-networkd.service")
        .context("Failed to enable systemd-networkd")?;

    // Wait-online improves reliability for services that want network-online.target, but don't
    // hard-fail if it's missing from the payload.
    enable_systemd_unit_optional(plan, "systemd-networkd-wait-online.service")?;

    // systemd-resolved provides the stub resolver; if it's missing, DHCP can still assign an IP.
    let resolved_enabled = enable_systemd_unit_optional(plan, "systemd-resolved.service")?;
    if resolved_enabled {
        // Point /etc/resolv.conf at the systemd-resolved stub.
        let resolv_conf = plan.target_root.join("etc/resolv.conf");
        if resolv_conf.exists() {
            let _ = std::fs::remove_file(&resolv_conf);
        }
        #[cfg(unix)]
        {
            unix_fs::symlink("/run/systemd/resolve/stub-resolv.conf", &resolv_conf)
                .with_context(|| format!("Failed to symlink {}", resolv_conf.display()))?;
        }
    }

    Ok(())
}

fn enable_systemd_unit_optional(plan: &MountPlan, unit_name: &str) -> Result<bool> {
    match find_systemd_unit_file(&plan.target_root, unit_name) {
        Ok(_) => {
            enable_systemd_unit(plan, unit_name)?;
            Ok(true)
        }
        Err(e) => {
            eprintln!("WARN: skipping enable of {unit_name}: {e:#}");
            Ok(false)
        }
    }
}

fn enable_systemd_unit(plan: &MountPlan, unit_name: &str) -> Result<()> {
    let unit_src = find_systemd_unit_file(&plan.target_root, unit_name)?;

    let wants_dir = plan.target_root.join("etc/systemd/system/multi-user.target.wants");
    std::fs::create_dir_all(&wants_dir)
        .with_context(|| format!("Failed to create {}", wants_dir.display()))?;

    let link_path = wants_dir.join(unit_name);
    if link_path.exists() {
        // If it's already enabled, keep it.
        return Ok(());
    }

    let link_target = path_in_target_root(&plan.target_root, &unit_src)?;

    #[cfg(unix)]
    {
        unix_fs::symlink(&link_target, &link_path).with_context(|| {
            format!("Failed to create symlink {} -> {}", link_path.display(), link_target)
        })?;
    }

    Ok(())
}

fn find_systemd_unit_file(target_root: &Path, unit_name: &str) -> Result<PathBuf> {
    // Debian typically uses /lib/systemd/system; some distros use /usr/lib/systemd/system.
    let candidates = [
        target_root.join("lib/systemd/system").join(unit_name),
        target_root.join("usr/lib/systemd/system").join(unit_name),
    ];
    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "Missing systemd unit file '{}' in target root (expected under /lib/systemd/system or /usr/lib/systemd/system). Ensure payload includes systemd.",
        unit_name
    ))
}

fn path_in_target_root(target_root: &Path, absolute_in_target: &Path) -> Result<String> {
    let rel = absolute_in_target.strip_prefix(target_root).with_context(|| {
        format!(
            "Path {} is not under target root {}",
            absolute_in_target.display(),
            target_root.display()
        )
    })?;
    Ok(format!("/{}", rel.display()))
}

fn ensure_systemd_pid1(plan: &MountPlan) -> Result<()> {
    // If the payload is missing systemd-sysv, Debian may boot with sysvinit, and systemctl will fail.
    // Force /sbin/init to systemd when systemd is present.
    let systemd_bin_candidates = [
        plan.target_root.join("lib/systemd/systemd"),
        plan.target_root.join("usr/lib/systemd/systemd"),
    ];
    let systemd_bin = systemd_bin_candidates
        .into_iter()
        .find(|p| p.exists())
        .ok_or_else(|| {
            anyhow!(
                "Target rootfs is missing systemd binary (/lib/systemd/systemd). Ensure payload includes systemd-sysv."
            )
        })?;

    let init_path = plan.target_root.join("sbin/init");
    if init_path.exists() {
        // If it's already a symlink to systemd, keep it.
        if let Ok(link) = std::fs::read_link(&init_path) {
            // Normalize both absolute and relative symlinks.
            let link_str = link.to_string_lossy();
            if link_str.contains("systemd") {
                return Ok(());
            }
        }
        let _ = std::fs::remove_file(&init_path);
    } else if let Some(parent) = init_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let link_target = path_in_target_root(&plan.target_root, &systemd_bin)?;
    #[cfg(unix)]
    {
        unix_fs::symlink(&link_target, &init_path)
            .with_context(|| format!("Failed to set {} -> {}", init_path.display(), link_target))?;
    }
    Ok(())
}

fn verify_esp_layout(esp_mount: &Path) -> Result<()> {
    let must_exist = [
        esp_mount.join("EFI/BOOT/BOOTX64.EFI"),
        esp_mount.join("EFI/systemd/systemd-bootx64.efi"),
        esp_mount.join("loader/loader.conf"),
        esp_mount.join("loader/entries/debian.conf"),
        esp_mount.join("EFI/debian/vmlinuz"),
        esp_mount.join("EFI/debian/initrd.img"),
    ];

    for path in must_exist {
        if !path.exists() {
            return Err(anyhow!("Missing on ESP: {}", path.display()));
        }
    }

    Ok(())
}

pub fn sync_disks() -> Result<()> {
    // We only have busybox in initramfs by default; call its applet directly.
    run("/bin/busybox", &["sync"]).context("busybox sync failed")
}

pub fn unmount_target(plan: &MountPlan) -> Result<()> {
    // Unmount ESP first (it is nested under the root mount), then root.
    run("umount", &[&plan.target_efi.display().to_string()])
        .with_context(|| format!("Failed to umount {}", plan.target_efi.display()))?;

    run("umount", &[&plan.target_root.display().to_string()])
        .with_context(|| format!("Failed to umount {}", plan.target_root.display()))?;

    Ok(())
}

fn register_uefi_boot_entry(disk_dev: &Path) -> Result<()> {
    // Only meaningful when booted in UEFI mode.
    if !Path::new("/sys/firmware/efi").exists() {
        return Ok(());
    }

    // Ensure efivarfs is mounted; efibootmgr needs it.
    let efivars = Path::new("/sys/firmware/efi/efivars");
    std::fs::create_dir_all(efivars)
        .with_context(|| format!("Failed to create {}", efivars.display()))?;

    // Ignore mount errors if it is already mounted; if it's not mounted, efibootmgr will fail and
    // we'll surface that error.
    let _ = run("mount", &["-t", "efivarfs", "efivarfs", &efivars.display().to_string()]);

    // ESP is always partition 1 in our GPT layout.
    // Note: efibootmgr expects the EFI path with backslashes.
    let efi_loader = r"\\EFI\\systemd\\systemd-bootx64.efi";
    let disk = disk_dev.display().to_string();

    let output = command("efibootmgr")
        .args(["-c", "-d", &disk, "-p", "1", "-L", "Debian (TruthDB)", "-l", efi_loader])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to execute efibootmgr")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Common in some VM configs (or when efivarfs isn't available): we can't write NVRAM vars.
    // This should not be fatal as long as the ESP fallback loader exists.
    if stderr.contains("EFI variables are not supported")
        || stderr.contains("Could not prepare boot variable")
        || stderr.contains("Operation not permitted")
        || stderr.contains("Read-only file system")
    {
        return Ok(());
    }

    Err(anyhow!(
        "efibootmgr failed: stdout='{}' stderr='{}'",
        String::from_utf8_lossy(&output.stdout),
        stderr
    ))
}

fn target_user_exists(target_root: &Path, username: &str) -> Result<bool> {
    let passwd_path = target_root.join("etc/passwd");
    let contents = std::fs::read_to_string(&passwd_path)
        .with_context(|| format!("Failed to read {}", passwd_path.display()))?;
    Ok(contents.lines().any(|line| line.starts_with(&format!("{username}:"))))
}

fn chroot_run(target_root: &Path, program_in_chroot: &str, args: &[&str]) -> Result<()> {
    let output = command("chroot")
        .arg(target_root)
        .arg(program_in_chroot)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to execute chroot {}", program_in_chroot))?;

    if output.status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "chroot {} failed: stdout='{}' stderr='{}'",
        program_in_chroot,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn chroot_chpasswd(target_root: &Path, username: &str, password: &str) -> Result<()> {
    let input = format!("{username}:{password}\n");

    let mut cmd = command("chroot");
    cmd.arg(target_root)
        .arg("/usr/sbin/chpasswd")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("Failed to spawn chroot for chpasswd")?;
    {
        use std::io::Write;
        let stdin =
            child.stdin.as_mut().ok_or_else(|| anyhow!("Failed to open stdin for chpasswd"))?;
        stdin.write_all(input.as_bytes()).context("Failed to write chpasswd input")?;
    }

    let output = child.wait_with_output().context("Failed to wait for chpasswd")?;
    if output.status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "chpasswd failed: stdout='{}' stderr='{}'",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn install_systemd_boot_efi(esp_mount: &Path) -> Result<()> {
    // The initramfs build copies /usr/lib/systemd/boot/efi into the initramfs.
    // For x86_64 UEFI, the loader binary is systemd-bootx64.efi.
    let src = Path::new("/usr/lib/systemd/boot/efi/systemd-bootx64.efi");
    if !src.exists() {
        return Err(anyhow!("Missing systemd-boot EFI binary in initramfs: {}", src.display()));
    }

    // UEFI removable media / fallback path.
    let boot_dir = esp_mount.join("EFI/BOOT");
    std::fs::create_dir_all(&boot_dir)
        .with_context(|| format!("Failed to create {}", boot_dir.display()))?;
    let fallback_dst = boot_dir.join("BOOTX64.EFI");
    std::fs::copy(src, &fallback_dst)
        .with_context(|| format!("Failed to copy systemd-boot to {}", fallback_dst.display()))?;

    // Also place it at the conventional systemd location.
    let systemd_dir = esp_mount.join("EFI/systemd");
    std::fs::create_dir_all(&systemd_dir)
        .with_context(|| format!("Failed to create {}", systemd_dir.display()))?;
    let systemd_dst = systemd_dir.join("systemd-bootx64.efi");
    std::fs::copy(src, &systemd_dst)
        .with_context(|| format!("Failed to copy systemd-boot to {}", systemd_dst.display()))?;

    Ok(())
}

fn write_fstab(root_uuid: &str, esp_uuid: &str, plan: &MountPlan) -> Result<()> {
    let etc_dir = plan.target_root.join("etc");
    std::fs::create_dir_all(&etc_dir)
        .with_context(|| format!("Failed to create {}", etc_dir.display()))?;

    let fstab_path = etc_dir.join("fstab");
    let contents = format!(
        "# /etc/fstab: static file system information.\n\
UUID={root_uuid} / ext4 defaults 0 1\n\
UUID={esp_uuid} /boot/efi vfat umask=0077 0 1\n"
    );
    std::fs::write(&fstab_path, contents)
        .with_context(|| format!("Failed to write {}", fstab_path.display()))
}

fn write_systemd_boot_entry(
    esp_mount: &Path,
    linux_path: &str,
    initrd_path: &str,
    root_uuid: &str,
) -> Result<()> {
    let loader_dir = esp_mount.join("loader");
    let entries_dir = loader_dir.join("entries");
    std::fs::create_dir_all(&entries_dir)
        .with_context(|| format!("Failed to create {}", entries_dir.display()))?;

    // Keep it simple: default entry and a single debian.conf.
    let loader_conf = loader_dir.join("loader.conf");
    std::fs::write(&loader_conf, "default debian.conf\ntimeout 0\nconsole-mode keep\n")
        .with_context(|| format!("Failed to write {}", loader_conf.display()))?;

    let entry = format!(
        "title   Debian (TruthDB)\n\
linux   {linux_path}\n\
initrd  {initrd_path}\n\
options root=UUID={root_uuid} rw init=/lib/systemd/systemd\n"
    );
    let entry_path = entries_dir.join("debian.conf");
    std::fs::write(&entry_path, entry)
        .with_context(|| format!("Failed to write {}", entry_path.display()))
}

fn blkid_uuid(dev: &Path) -> Result<String> {
    let output = command("blkid")
        .args(["-s", "UUID", "-o", "value", &dev.display().to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to execute blkid for {}", dev.display()))?;

    if !output.status.success() {
        return Err(anyhow!(
            "blkid failed for {}: stdout='{}' stderr='{}'",
            dev.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let uuid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uuid.is_empty() {
        return Err(anyhow!("blkid returned empty UUID for {}", dev.display()));
    }
    Ok(uuid)
}

fn find_installed_kernel_and_initrd(target_root: &Path) -> Result<(PathBuf, PathBuf)> {
    let boot = target_root.join("boot");
    let mut kernels: Vec<PathBuf> = Vec::new();
    let mut initrds: Vec<PathBuf> = Vec::new();

    for entry in
        std::fs::read_dir(&boot).with_context(|| format!("Failed to read {}", boot.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("vmlinuz-") {
            kernels.push(path);
        } else if name.starts_with("initrd.img-") {
            initrds.push(path);
        }
    }

    kernels.sort();
    initrds.sort();

    let kernel =
        kernels.pop().ok_or_else(|| anyhow!("No vmlinuz-* found under {}", boot.display()))?;
    let initrd =
        initrds.pop().ok_or_else(|| anyhow!("No initrd.img-* found under {}", boot.display()))?;

    Ok((kernel, initrd))
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let output = command(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to execute {program}"))?;

    if output.status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "{program} failed: stdout='{}' stderr='{}'",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("PATH", DEFAULT_PATH);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mount_points() {
        let plan = MountPlan::default();
        assert_eq!(plan.target_root, PathBuf::from("/mnt"));
        assert_eq!(plan.target_efi, PathBuf::from("/mnt/boot/efi"));
    }
}

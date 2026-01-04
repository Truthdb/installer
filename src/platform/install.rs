use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
options root=UUID={root_uuid} rw\n"
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

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
    // Ensure mount points exist.
    std::fs::create_dir_all(&plan.target_root)
        .with_context(|| format!("Failed to create {}", plan.target_root.display()))?;
    std::fs::create_dir_all(&plan.target_efi)
        .with_context(|| format!("Failed to create {}", plan.target_efi.display()))?;

    // Mount root first.
    run(
        "mount",
        &["-t", "ext4", &root.display().to_string(), &plan.target_root.display().to_string()],
    )
    .with_context(|| format!("Failed to mount root {}", root.display()))?;

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

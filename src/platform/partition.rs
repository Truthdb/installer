use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DEFAULT_PATH: &str = "/bin:/sbin:/usr/bin:/usr/sbin";

const EFI_SYSTEM_PARTITION_GUID: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const LINUX_FILESYSTEM_GUID: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

#[derive(Debug, Clone, Copy)]
pub struct PartitionPlan {
    pub esp_size_mib: u64,
}

impl Default for PartitionPlan {
    fn default() -> Self {
        Self { esp_size_mib: 512 }
    }
}

pub fn wipefs_all(disk: &Path) -> Result<()> {
    run("wipefs", &["-a", &disk.display().to_string()])
        .with_context(|| format!("wipefs failed for {}", disk.display()))
}

pub fn partition_gpt_esp_root(disk: &Path, plan: PartitionPlan) -> Result<()> {
    if command_exists("sfdisk") {
        return partition_with_sfdisk(disk, plan);
    }
    if command_exists("parted") {
        return partition_with_parted(disk, plan);
    }

    Err(anyhow!("No partitioning tool available (need 'sfdisk' or 'parted')"))
}

/// Compute the expected partition device paths for a whole-disk device.
///
/// Examples:
/// - `/dev/sda` -> `/dev/sda1`, `/dev/sda2`
/// - `/dev/nvme0n1` -> `/dev/nvme0n1p1`, `/dev/nvme0n1p2`
pub fn expected_esp_and_root_partitions(disk: &Path) -> Result<(PathBuf, PathBuf)> {
    let name = disk
        .file_name()
        .ok_or_else(|| anyhow!("Invalid disk path: {}", disk.display()))?
        .to_string_lossy();

    // Linux partition naming: if the disk name ends with a digit, partitions add a 'p'.
    // Examples: nvme0n1p1, mmcblk0p1.
    let needs_p = name.chars().last().is_some_and(|c| c.is_ascii_digit());
    let sep = if needs_p { "p" } else { "" };

    let esp = PathBuf::from("/dev").join(format!("{name}{sep}1"));
    let root = PathBuf::from("/dev").join(format!("{name}{sep}2"));
    Ok((esp, root))
}

fn partition_with_sfdisk(disk: &Path, plan: PartitionPlan) -> Result<()> {
    let script = sfdisk_gpt_script(plan);

    let mut child = command("sfdisk")
        .arg("--label")
        .arg("gpt")
        .arg(disk)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn sfdisk for {}", disk.display()))?;

    {
        use std::io::Write;
        let stdin =
            child.stdin.as_mut().ok_or_else(|| anyhow!("Failed to open stdin for sfdisk"))?;
        stdin.write_all(script.as_bytes()).context("Failed to write sfdisk script")?;
    }

    let output = child.wait_with_output().context("Failed to wait for sfdisk")?;
    if !output.status.success() {
        return Err(anyhow!(
            "sfdisk failed: stdout='{}' stderr='{}'",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    reread_partition_table(disk)
}

fn partition_with_parted(disk: &Path, plan: PartitionPlan) -> Result<()> {
    // Use MiB-aligned boundaries. Start at 1MiB, ESP spans [1, 1+esp].
    let esp_start = "1MiB".to_string();
    let esp_end = format!("{}MiB", 1 + plan.esp_size_mib);
    let root_start = esp_end.clone();

    run(
        "parted",
        &[
            "-s",
            &disk.display().to_string(),
            "mklabel",
            "gpt",
            "mkpart",
            "ESP",
            "fat32",
            &esp_start,
            &esp_end,
            "set",
            "1",
            "esp",
            "on",
            "mkpart",
            "root",
            "ext4",
            &root_start,
            "100%",
        ],
    )
    .with_context(|| format!("parted failed for {}", disk.display()))?;

    reread_partition_table(disk)
}

fn reread_partition_table(disk: &Path) -> Result<()> {
    if command_exists("partprobe") {
        return run("partprobe", &[&disk.display().to_string()])
            .with_context(|| format!("partprobe failed for {}", disk.display()));
    }

    // Best-effort fallback: let the kernel notice the changes.
    std::thread::sleep(std::time::Duration::from_millis(500));
    Ok(())
}

fn sfdisk_gpt_script(plan: PartitionPlan) -> String {
    // sfdisk script syntax accepts key/value pairs.
    // We keep it minimal: create ESP (fixed size) then root (remainder).
    format!(
        "label: gpt\n\nsize={}MiB, type={}\ntype={}\n",
        plan.esp_size_mib, EFI_SYSTEM_PARTITION_GUID, LINUX_FILESYSTEM_GUID
    )
}

fn command_exists(program: &str) -> bool {
    command(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    fn sfdisk_script_contains_expected_types() {
        let script = sfdisk_gpt_script(PartitionPlan { esp_size_mib: 512 });
        assert!(script.contains("label: gpt"));
        assert!(script.contains(EFI_SYSTEM_PARTITION_GUID));
        assert!(script.contains(LINUX_FILESYSTEM_GUID));
        assert!(script.contains("size=512MiB"));
    }

    #[test]
    fn expected_partition_paths_for_sda() {
        let (esp, root) = expected_esp_and_root_partitions(Path::new("/dev/sda")).unwrap();
        assert_eq!(esp, PathBuf::from("/dev/sda1"));
        assert_eq!(root, PathBuf::from("/dev/sda2"));
    }

    #[test]
    fn expected_partition_paths_for_nvme() {
        let (esp, root) = expected_esp_and_root_partitions(Path::new("/dev/nvme0n1")).unwrap();
        assert_eq!(esp, PathBuf::from("/dev/nvme0n1p1"));
        assert_eq!(root, PathBuf::from("/dev/nvme0n1p2"));
    }
}

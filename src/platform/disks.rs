use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub name: String,
    pub dev_path: PathBuf,
    pub size_bytes: u64,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiskScanner {
    sys_root: PathBuf,
    proc_root: PathBuf,
    min_size_bytes: u64,
}

impl DiskScanner {
    pub fn new(sys_root: impl Into<PathBuf>, proc_root: impl Into<PathBuf>, min_size_bytes: u64) -> Self {
        Self {
            sys_root: sys_root.into(),
            proc_root: proc_root.into(),
            min_size_bytes,
        }
    }

    pub fn new_default() -> Self {
        // MVP safety threshold; can be made configurable later.
        const GIB: u64 = 1024 * 1024 * 1024;
        Self::new("/sys", "/proc", 8 * GIB)
    }

    pub fn eligible_disks(&self) -> Result<Vec<Disk>> {
        let block_dir = self.sys_root.join("block");
        let mut disks = Vec::new();

        for entry in fs::read_dir(&block_dir)
            .with_context(|| format!("Failed to read {}", block_dir.display()))?
        {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy().to_string();
            if !is_candidate_name(&name) {
                continue;
            }

            let disk_sys = entry.path();

            // If there is no backing device directory, it is unlikely to be a real disk.
            if !disk_sys.join("device").exists() {
                continue;
            }

            if read_u64(disk_sys.join("removable")).unwrap_or(1) != 0 {
                continue;
            }
            if read_u64(disk_sys.join("ro")).unwrap_or(1) != 0 {
                continue;
            }

            let size_bytes = disk_size_bytes(&disk_sys).with_context(|| format!("Failed to read size for {name}"))?;
            if size_bytes < self.min_size_bytes {
                continue;
            }

            let dev_path = PathBuf::from("/dev").join(&name);
            if is_device_mounted(&self.proc_root, &name)? {
                continue;
            }

            let model = read_string(disk_sys.join("device").join("model")).ok();

            disks.push(Disk {
                name,
                dev_path,
                size_bytes,
                model,
            });
        }

        disks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(disks)
    }

    pub fn choose_single_target_disk(&self) -> Result<Disk> {
        let eligible = self.eligible_disks()?;
        match eligible.len() {
            0 => Err(anyhow!("No eligible disks found")),
            1 => Ok(eligible.into_iter().next().unwrap()),
            _ => {
                let names = eligible
                    .iter()
                    .map(|d| d.dev_path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(anyhow!(
                    "Multiple eligible disks found (refusing to choose automatically): {names}"
                ))
            }
        }
    }
}

fn is_candidate_name(name: &str) -> bool {
    // Exclude virtual and non-install targets.
    if name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("sr")
        || name.starts_with("fd")
        || name.starts_with("dm-")
        || name.starts_with("md")
    {
        return false;
    }

    // Common install targets: sdX, vdX, nvmeXnY
    name.starts_with("sd") || name.starts_with("vd") || name.starts_with("nvme")
}

fn read_u64(path: impl AsRef<Path>) -> Result<u64> {
    let s = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.as_ref().display()))?;
    let s = s.trim();
    s.parse::<u64>()
        .with_context(|| format!("Failed to parse '{}' from {}", s, path.as_ref().display()))
}

fn read_string(path: impl AsRef<Path>) -> Result<String> {
    let s = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.as_ref().display()))?;
    Ok(s.trim().to_string())
}

fn disk_size_bytes(disk_sys: &Path) -> Result<u64> {
    // /sys/block/<dev>/size is in 512-byte sectors.
    let sectors = read_u64(disk_sys.join("size"))?;
    Ok(sectors.saturating_mul(512))
}

fn is_device_mounted(proc_root: &Path, dev_name: &str) -> Result<bool> {
    let mountinfo = proc_root.join("self").join("mountinfo");
    let contents = fs::read_to_string(&mountinfo)
        .with_context(|| format!("Failed to read {}", mountinfo.display()))?;

    let needle = format!("/dev/{dev_name}");

    for line in contents.lines() {
        // mountinfo format: https://www.kernel.org/doc/Documentation/filesystems/proc.txt
        // ... optional fields ... - fstype source superoptions
        let Some((_, after)) = line.split_once(" - ") else {
            continue;
        };
        let mut parts = after.split_whitespace();
        let _fstype = parts.next();
        let source = parts.next();
        let Some(source) = source else {
            continue;
        };

        // Treat the whole disk or any of its partitions as "mounted".
        // Examples: /dev/sda, /dev/sda1, /dev/nvme0n1, /dev/nvme0n1p1
        if source == needle || source.starts_with(&needle) {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn make_scanner(sys_root: &Path, proc_root: &Path) -> DiskScanner {
        // 1 MiB threshold for tests.
        DiskScanner::new(sys_root, proc_root, 1024 * 1024)
    }

    #[test]
    fn one_disk_is_eligible() {
        let temp = tempfile::tempdir().unwrap();
        let sys = temp.path().join("sys");
        let proc = temp.path().join("proc");

        // /sys/block/vda
        let vda = sys.join("block").join("vda");
        write(&vda.join("removable"), "0\n");
        write(&vda.join("ro"), "0\n");
        write(&vda.join("size"), "4096\n"); // 4096 * 512 = 2MiB
        fs::create_dir_all(vda.join("device")).unwrap();
        write(&vda.join("device").join("model"), "UTM Disk\n");

        // No mounts
        write(&proc.join("self").join("mountinfo"), "");

        let scanner = make_scanner(&sys, &proc);
        let disks = scanner.eligible_disks().unwrap();
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].name, "vda");
        assert_eq!(disks[0].dev_path, PathBuf::from("/dev/vda"));
        assert_eq!(disks[0].model.as_deref(), Some("UTM Disk"));
    }

    #[test]
    fn multiple_disks_is_error() {
        let temp = tempfile::tempdir().unwrap();
        let sys = temp.path().join("sys");
        let proc = temp.path().join("proc");

        for dev in ["vda", "vdb"] {
            let d = sys.join("block").join(dev);
            write(&d.join("removable"), "0\n");
            write(&d.join("ro"), "0\n");
            write(&d.join("size"), "4096\n");
            fs::create_dir_all(d.join("device")).unwrap();
        }
        write(&proc.join("self").join("mountinfo"), "");

        let scanner = make_scanner(&sys, &proc);
        let err = scanner.choose_single_target_disk().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Multiple eligible disks"));
        assert!(msg.contains("/dev/vda"));
        assert!(msg.contains("/dev/vdb"));
    }

    #[test]
    fn removable_disk_is_excluded() {
        let temp = tempfile::tempdir().unwrap();
        let sys = temp.path().join("sys");
        let proc = temp.path().join("proc");

        let vda = sys.join("block").join("vda");
        write(&vda.join("removable"), "1\n");
        write(&vda.join("ro"), "0\n");
        write(&vda.join("size"), "4096\n");
        fs::create_dir_all(vda.join("device")).unwrap();

        write(&proc.join("self").join("mountinfo"), "");

        let scanner = make_scanner(&sys, &proc);
        let disks = scanner.eligible_disks().unwrap();
        assert_eq!(disks.len(), 0);
    }
}

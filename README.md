# TruthDB Installer

The installer executable that runs inside the TruthDB installer ISO's initramfs.

This repo builds a small, statically-linked Rust binary (`truthdb-installer`) intended to be launched by BusyBox `init` on the ISO. The ISO build (see the `installer-iso` repo) provides the surrounding initramfs, embedded Debian payload, and required system tools.

## What It Does (Current Implementation)

The current installer is **console-only** and interacts via blocking stdin prompts.

High-level flow:

1. Enumerate eligible install disks (refuses to choose if more than one candidate is present).
2. Prompt for confirmation.
3. Wipe existing disk signatures (`wipefs -a`).
4. Partition GPT: ESP (512 MiB) + root (remainder) (`sfdisk` preferred, `parted` fallback).
5. Format: ESP as FAT32 (`mkfs.vfat`), root as ext4 (`mkfs.ext4`).
6. Mount root at `/mnt` and ESP at `/mnt/boot/efi`.
7. Extract offline Debian payload from `/payload/debian-minbase-amd64-bookworm.tar.zst` using `tar --zstd`.
8. Configure hostname (`truthdb01`).
9. Create initial user and set passwords (currently hardcoded).
10. Configure DHCP for first boot using `systemd-networkd`.
11. Install `systemd-boot` into the ESP, copy the installed Debian kernel/initrd into the ESP, write a loader entry, and best-effort create an NVRAM entry via `efibootmgr`.
12. Sync, unmount, and reboot.

## Safety / Assumptions

- Destructive by design: it will repartition and format the selected disk.
- Disk selection is deliberately strict:
    - Filters out common non-target devices (loop/ram/sr/fd/dm-/md).
    - Requires a backing `/sys/block/<dev>/device`.
    - Requires non-removable and non-readonly.
    - Requires size >= 8 GiB.
    - Refuses to run if the disk (or its partitions) appear mounted.
    - Refuses to auto-pick if more than one eligible disk exists.

## Runtime Requirements (Initramfs)

Because the installer executes external tools directly (no shell), the initramfs must include these programs (and shared libraries if dynamically linked):

- `wipefs`, `sfdisk` or `parted`, `partprobe`, `blkid`
- `mkfs.vfat`, `mkfs.ext4`, `mount`, `umount`
- `tar` (with zstd support) + `zstd`
- `chroot`
- `efibootmgr` (best-effort; installer remains bootable via ESP fallback path)
- `systemd-boot` EFI binary at `/usr/lib/systemd/boot/efi/systemd-bootx64.efi`

The `installer-iso` release workflow is responsible for assembling a correct initramfs with these tools.

## Credentials (MVP)

The initial username/password are currently hardcoded in code (`truthdb` / `123456`) and should be treated as an MVP default. Plan on changing these immediately after first boot.

## Building

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

Output binary:

`target/x86_64-unknown-linux-musl/release/truthdb-installer`

## CI/CD

- CI runs formatting, clippy, tests, and a musl release build.
- Release (tag `v*`) publishes the musl tarball + sha256 used by `installer-iso`.

## License

MIT. See LICENSE.


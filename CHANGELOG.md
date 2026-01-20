# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial implementation of TruthDB installer (console-only)
- Safe-ish disk selection: refuses to pick if multiple eligible disks exist
- GPT partitioning (ESP + root), formatting, mounting, payload extraction
- Debian post-install configuration: hostname, users/passwords, DHCP via systemd-networkd
- systemd-boot installation on ESP + loader entry + best-effort `efibootmgr`
- Static musl build for initramfs compatibility
- CI workflows for lint, test, and build
- Release workflow packaging musl tarball + sha256

### Changed
- Documentation now reflects the current console-only implementation (older UI/state-machine docs were stale).

### Documentation
- Updated README to match current code paths and ISO workflow expectations

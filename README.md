# TruthDB Installer

The installer used by the ISO boot installation for TruthDB.

## Overview

The TruthDB installer is a minimal, statically-linked executable designed to run in an initramfs environment. It provides a simple framebuffer-based UI and handles the installation workflow for TruthDB systems.

## Features

- **Minimal footprint**: Static musl binary (~600KB)
- **No dependencies**: Runs without systemd, dbus, X11, or Wayland
- **Framebuffer UI**: DRM/KMS or Linux framebuffer (/dev/fb0) support
- **Console fallback**: Works even without graphics
- **Keyboard input**: Via evdev or stdin
- **State machine**: Clean state transitions for installation workflow
- **Structured logging**: All output to stdout/stderr for console/serial capture

## Building

### Prerequisites

- Rust toolchain (stable)
- musl target support

### Build Commands

```bash
# Add musl target
rustup target add x86_64-unknown-linux-musl

# Build release binary
cargo build --release --target x86_64-unknown-linux-musl

# Binary location
ls -lh target/x86_64-unknown-linux-musl/release/truthdb-installer
```

### Build for Development

```bash
# Run tests
cargo test --target x86_64-unknown-linux-musl

# Run clippy
cargo clippy --all-targets --all-features

# Check formatting
cargo fmt -- --check
```

## Usage

The installer is designed to run as PID 2 in an initramfs environment, started by BusyBox init (PID 1).

### Runtime Requirements

- Linux kernel with framebuffer or DRM/KMS support (optional, has console fallback)
- `/dev/input/event*` devices for keyboard input (optional, has stdin fallback)
- `/proc`, `/sys`, `/dev` mounted by init system

### Running

```bash
# In initramfs, BusyBox init will start it automatically
# For manual testing (requires appropriate permissions):
./truthdb-installer
```

### Exit Codes

- `0`: Clean exit (user chose to quit/reboot)
- `non-zero`: Fatal error occurred

### Controls (MVP)

- **Q**: Quit the installer

## Architecture

### Module Structure

```
src/
├── main.rs              # Entry point and main loop
├── app.rs               # State machine and application logic
├── ui/
│   ├── mod.rs          # UI trait definitions
│   ├── fb.rs           # Framebuffer backend
│   └── font_8x8.rs     # Bitmap font data
├── input/
│   ├── mod.rs          # Input trait definitions
│   └── evdev_handler.rs # Keyboard input via evdev
└── platform/
    └── mod.rs          # Platform operations (reboot, poweroff)
```

### State Machine

The installer implements an explicit state machine:

1. **BootSplash**: Initial state on startup
2. **Welcome**: Main screen with instructions (current MVP)
3. **Error**: Error state with message
4. **Exit**: Clean shutdown state

Future states will be added for the installation workflow.

### UI Backends

The installer supports multiple UI backends with automatic fallback:

1. **Framebuffer** (`/dev/fb0`): Direct framebuffer rendering
2. **Console**: ANSI escape codes fallback

Future: DRM/KMS backend for better hardware support.

### Input Handling

The installer supports multiple input methods:

1. **evdev**: Direct keyboard input from `/dev/input/event*`
2. **stdin**: Fallback to standard input

## Development

### Testing

```bash
# Run all tests
cargo test --target x86_64-unknown-linux-musl

# Run specific test
cargo test --target x86_64-unknown-linux-musl test_name
```

### Testing in Docker (recommended)
If you don't have a Linux + musl toolchain locally, you can run the same tests CI runs using Docker:

```bash
./scripts/test_docker.sh
```

This runs:
- `cargo test --target x86_64-unknown-linux-gnu`
- `cargo test --target x86_64-unknown-linux-musl` (after installing `musl-tools` and `rust-std`)

### Code Structure Guidelines

- Keep modules focused and testable
- Use the state machine for all workflow logic
- Log important events for debugging
- Handle errors gracefully with fallbacks
- Prefer static linking for initramfs compatibility

## CI/CD

The project includes GitHub Actions workflows for:

- **CI**: Lint, test, and build on every push/PR
- **Release**: Automated releases with changelog generation

## License

Apache-2.0


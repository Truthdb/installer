# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial implementation of TruthDB installer
- State machine with BootSplash, Welcome, Error, and Exit states
- Framebuffer UI backend with console fallback
- Evdev keyboard input handler with stdin fallback
- 8x8 bitmap font for text rendering
- Structured logging to stdout/stderr
- Static musl build for initramfs compatibility
- CI workflows for lint, test, and build
- Release workflow with automated changelog

### Features
- Minimal UI showing installer status
- Keyboard input handling (Q to quit)
- Clean exit codes (0 for success, non-zero for errors)
- Modular architecture for future expansion

### Documentation
- Comprehensive README with usage instructions
- Architecture documentation
- Build instructions for musl target

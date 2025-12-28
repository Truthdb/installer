//! Evdev-based keyboard input handler

use anyhow::{Context, Result, anyhow};
use evdev::{Device, EventSummary, KeyCode};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info, warn};

use super::InputHandler;

/// Evdev keyboard input handler
pub struct EvdevHandler {
    device: Option<Device>,
    fallback_mode: bool,
}

impl EvdevHandler {
    /// Create a new evdev handler
    pub fn new() -> Result<Self> {
        Ok(Self { device: None, fallback_mode: false })
    }

    /// Find a keyboard device in /dev/input/event*
    fn find_keyboard() -> Result<Device> {
        let input_dir = PathBuf::from("/dev/input");

        if !input_dir.exists() {
            return Err(anyhow!("/dev/input directory not found"));
        }

        let entries = fs::read_dir(&input_dir).context("Failed to read /dev/input directory")?;

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name() {
                if name.to_string_lossy().starts_with("event") {
                    if let Ok(device) = Device::open(&path) {
                        // Check if this device has keyboard capabilities
                        // We check for multiple common keys across different layouts
                        if device.supported_keys().is_some_and(|keys| {
                            // Check for alphanumeric keys that are common across layouts
                            let has_letters = keys.contains(KeyCode::KEY_Q)
                                || keys.contains(KeyCode::KEY_A)
                                || keys.contains(KeyCode::KEY_E);
                            let has_numbers =
                                keys.contains(KeyCode::KEY_1) || keys.contains(KeyCode::KEY_2);
                            let has_enter = keys.contains(KeyCode::KEY_ENTER);

                            // A keyboard typically has letters, numbers, and enter
                            has_letters && (has_numbers || has_enter)
                        }) {
                            info!("Found keyboard device: {:?}", path);
                            return Ok(device);
                        }
                    }
                }
            }
        }

        Err(anyhow!("No keyboard device found in /dev/input"))
    }

    /// Map evdev key to character (simplified)
    fn key_to_char(key: KeyCode) -> Option<char> {
        match key {
            KeyCode::KEY_Q => Some('q'),
            KeyCode::KEY_W => Some('w'),
            KeyCode::KEY_E => Some('e'),
            KeyCode::KEY_R => Some('r'),
            KeyCode::KEY_T => Some('t'),
            KeyCode::KEY_Y => Some('y'),
            KeyCode::KEY_U => Some('u'),
            KeyCode::KEY_I => Some('i'),
            KeyCode::KEY_O => Some('o'),
            KeyCode::KEY_P => Some('p'),
            KeyCode::KEY_A => Some('a'),
            KeyCode::KEY_S => Some('s'),
            KeyCode::KEY_D => Some('d'),
            KeyCode::KEY_F => Some('f'),
            KeyCode::KEY_G => Some('g'),
            KeyCode::KEY_H => Some('h'),
            KeyCode::KEY_J => Some('j'),
            KeyCode::KEY_K => Some('k'),
            KeyCode::KEY_L => Some('l'),
            KeyCode::KEY_Z => Some('z'),
            KeyCode::KEY_X => Some('x'),
            KeyCode::KEY_C => Some('c'),
            KeyCode::KEY_V => Some('v'),
            KeyCode::KEY_B => Some('b'),
            KeyCode::KEY_N => Some('n'),
            KeyCode::KEY_M => Some('m'),
            KeyCode::KEY_SPACE => Some(' '),
            KeyCode::KEY_ENTER => Some('\n'),
            _ => None,
        }
    }

    /// Check stdin for input in fallback mode
    fn poll_stdin() -> Result<Option<char>> {
        use std::io::Read;

        // Set stdin to non-blocking mode
        let stdin = std::io::stdin();
        let mut buffer = [0u8; 1];

        // Try to read one byte without blocking
        match stdin.lock().read(&mut buffer) {
            Ok(1) => {
                let ch = buffer[0] as char;
                Ok(Some(ch.to_ascii_lowercase()))
            }
            Ok(0) => Ok(None), // EOF
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl InputHandler for EvdevHandler {
    fn init(&mut self) -> Result<()> {
        match Self::find_keyboard() {
            Ok(device) => {
                device
                    .set_nonblocking(true)
                    .context("Failed to set input device to non-blocking")?;
                info!("Evdev input handler initialized successfully");
                self.device = Some(device);
                self.fallback_mode = false;
                Ok(())
            }
            Err(e) => {
                warn!("Failed to initialize evdev: {}. Using stdin fallback.", e);
                self.fallback_mode = true;

                // Set stdin to non-blocking mode in fallback
                use nix::fcntl::{FcntlArg, OFlag, fcntl};
                use std::os::fd::BorrowedFd;

                let stdin_fd = unsafe { BorrowedFd::borrow_raw(0) };
                if let Ok(flags) = fcntl(stdin_fd, FcntlArg::F_GETFL) {
                    let mut flags = OFlag::from_bits_truncate(flags);
                    flags.insert(OFlag::O_NONBLOCK);
                    let _ = fcntl(stdin_fd, FcntlArg::F_SETFL(flags));
                }

                Ok(())
            }
        }
    }

    fn poll(&mut self) -> Result<Option<char>> {
        if self.fallback_mode {
            return Self::poll_stdin();
        }

        if let Some(ref mut device) = self.device {
            match device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        if let EventSummary::Key(_, key, value) = event.destructure() {
                            // Only process key press (value == 1), not release (value == 0)
                            if value == 1 {
                                if let Some(ch) = Self::key_to_char(key) {
                                    debug!("Key pressed: {:?} -> '{}'", key, ch);
                                    return Ok(Some(ch));
                                }
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }

        Ok(None)
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.fallback_mode {
            // Restore blocking mode to stdin
            use nix::fcntl::{FcntlArg, OFlag, fcntl};
            use std::os::fd::BorrowedFd;

            let stdin_fd = unsafe { BorrowedFd::borrow_raw(0) };
            if let Ok(flags) = fcntl(stdin_fd, FcntlArg::F_GETFL) {
                let mut flags = OFlag::from_bits_truncate(flags);
                flags.remove(OFlag::O_NONBLOCK);
                let _ = fcntl(stdin_fd, FcntlArg::F_SETFL(flags));
            }
        }

        info!("Evdev input handler cleaned up");
        Ok(())
    }
}

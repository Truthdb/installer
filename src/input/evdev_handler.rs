//! Evdev-based keyboard input handler

use anyhow::{Context, Result, anyhow};
use evdev::{Device, InputEventKind, Key};
use std::fs;
use std::path::PathBuf;
use tracing::{info, debug, warn};

use super::InputHandler;

/// Evdev keyboard input handler
pub struct EvdevHandler {
    device: Option<Device>,
    fallback_mode: bool,
}

impl EvdevHandler {
    /// Create a new evdev handler
    pub fn new() -> Result<Self> {
        Ok(Self {
            device: None,
            fallback_mode: false,
        })
    }

    /// Find a keyboard device in /dev/input/event*
    fn find_keyboard() -> Result<Device> {
        let input_dir = PathBuf::from("/dev/input");
        
        if !input_dir.exists() {
            return Err(anyhow!("/dev/input directory not found"));
        }

        let entries = fs::read_dir(&input_dir)
            .context("Failed to read /dev/input directory")?;

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name() {
                if name.to_string_lossy().starts_with("event") {
                    if let Ok(device) = Device::open(&path) {
                        // Check if this device has keyboard capabilities
                        if device.supported_keys().map_or(false, |keys| {
                            keys.contains(Key::KEY_Q) && keys.contains(Key::KEY_A)
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
    fn key_to_char(key: Key) -> Option<char> {
        match key {
            Key::KEY_Q => Some('q'),
            Key::KEY_W => Some('w'),
            Key::KEY_E => Some('e'),
            Key::KEY_R => Some('r'),
            Key::KEY_T => Some('t'),
            Key::KEY_Y => Some('y'),
            Key::KEY_U => Some('u'),
            Key::KEY_I => Some('i'),
            Key::KEY_O => Some('o'),
            Key::KEY_P => Some('p'),
            Key::KEY_A => Some('a'),
            Key::KEY_S => Some('s'),
            Key::KEY_D => Some('d'),
            Key::KEY_F => Some('f'),
            Key::KEY_G => Some('g'),
            Key::KEY_H => Some('h'),
            Key::KEY_J => Some('j'),
            Key::KEY_K => Some('k'),
            Key::KEY_L => Some('l'),
            Key::KEY_Z => Some('z'),
            Key::KEY_X => Some('x'),
            Key::KEY_C => Some('c'),
            Key::KEY_V => Some('v'),
            Key::KEY_B => Some('b'),
            Key::KEY_N => Some('n'),
            Key::KEY_M => Some('m'),
            Key::KEY_SPACE => Some(' '),
            Key::KEY_ENTER => Some('\n'),
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
                info!("Evdev input handler initialized successfully");
                self.device = Some(device);
                self.fallback_mode = false;
                Ok(())
            }
            Err(e) => {
                warn!("Failed to initialize evdev: {}. Using stdin fallback.", e);
                self.fallback_mode = true;
                
                // Set stdin to non-blocking mode in fallback
                use nix::fcntl::{fcntl, FcntlArg, OFlag};
                let stdin_fd = 0;
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
            // Fetch events (non-blocking)
            while let Ok(events) = device.fetch_events() {
                for event in events {
                    if let InputEventKind::Key(key) = event.kind() {
                        // Only process key press (value == 1), not release (value == 0)
                        if event.value() == 1 {
                            if let Some(ch) = Self::key_to_char(key) {
                                debug!("Key pressed: {:?} -> '{}'", key, ch);
                                return Ok(Some(ch));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.fallback_mode {
            // Restore blocking mode to stdin
            use nix::fcntl::{fcntl, FcntlArg, OFlag};
            let stdin_fd = 0;
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

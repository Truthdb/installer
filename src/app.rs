//! Main application state machine

use anyhow::Result;
use std::collections::VecDeque;
use std::fmt;
use tracing::{info, warn};

/// Application states
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    /// Initial boot splash screen
    BootSplash,
    /// Welcome screen with instructions
    Welcome,
    /// Error state with message
    Error(String),
    /// Exit state
    Exit,
}

impl fmt::Display for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppState::BootSplash => write!(f, "BootSplash"),
            AppState::Welcome => write!(f, "Welcome"),
            AppState::Error(msg) => write!(f, "Error: {}", msg),
            AppState::Exit => write!(f, "Exit"),
        }
    }
}

/// Main application controller
pub struct App {
    state: AppState,
    should_exit: bool,
    log_lines: VecDeque<String>,
    max_log_lines: usize,
}

impl App {
    /// Create a new application instance
    pub fn new() -> Self {
        info!("Creating new application instance");
        let mut app = Self {
            state: AppState::BootSplash,
            should_exit: false,
            log_lines: VecDeque::new(),
            // Conservative default; UI will clip if the screen is smaller.
            max_log_lines: 80,
        };
        app.log_step("TruthDB Installer booting");
        app
    }

    /// Get current state
    #[allow(dead_code)]
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Check if application should exit
    pub fn should_exit(&self) -> bool {
        self.should_exit
    }

    /// Initialize and transition from BootSplash to Welcome
    pub fn initialize(&mut self) -> Result<()> {
        info!("Initializing application");
        if self.state == AppState::BootSplash {
            self.transition_to(AppState::Welcome)?;
        }
        Ok(())
    }

    /// Append a single log line to the UI (one line per step).
    pub fn log_step(&mut self, line: impl Into<String>) {
        let line = line.into();
        self.log_lines.push_back(line);
        while self.log_lines.len() > self.max_log_lines {
            self.log_lines.pop_front();
        }
    }

    /// Handle user input
    pub fn handle_input(&mut self, key: char) -> Result<()> {
        info!("Handling input: '{}'", key);

        match self.state {
            AppState::Welcome => {
                if key == 'q' || key == 'Q' {
                    info!("User requested exit");
                    self.transition_to(AppState::Exit)?;
                }
            }
            AppState::Error(_) => {
                if key == 'q' || key == 'Q' {
                    info!("Exiting from error state");
                    self.transition_to(AppState::Exit)?;
                }
            }
            _ => {
                warn!("Input ignored in state: {}", self.state);
            }
        }

        Ok(())
    }

    /// Transition to a new state
    fn transition_to(&mut self, new_state: AppState) -> Result<()> {
        info!("State transition: {} -> {}", self.state, new_state);

        if new_state == AppState::Exit {
            self.should_exit = true;
        }

        self.state = new_state;
        Ok(())
    }

    /// Handle error condition
    pub fn handle_error(&mut self, error: String) {
        warn!("Application error: {}", error);
        for line in wrap_for_ui(&error, 78, "[ERR] ", "      ") {
            self.log_step(line);
        }
        self.state = AppState::Error(error);
    }

    /// Get display text for current state
    pub fn get_display_text(&self) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();

        // Always render the step log top-left.
        lines.extend(self.log_lines.iter().cloned());

        // Keep minimal state hints without taking over the screen.
        match &self.state {
            AppState::BootSplash => {}
            AppState::Welcome => {}
            AppState::Error(_) => {}
            AppState::Exit => {
                lines.push("[OK] Exiting".to_string());
            }
        }

        lines
    }
}

fn wrap_for_ui(text: &str, width: usize, first_prefix: &str, next_prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut remaining = text.trim();

    let mut is_first = true;
    while !remaining.is_empty() {
        let prefix = if is_first { first_prefix } else { next_prefix };
        let avail = width.saturating_sub(prefix.len()).max(10);

        if remaining.len() <= avail {
            out.push(format!("{}{}", prefix, remaining));
            break;
        }

        // Try to break on whitespace within the available width.
        let mut split_at = avail;
        if let Some(idx) = remaining[..avail].rfind(char::is_whitespace) {
            // Avoid producing empty segments if whitespace is at the start.
            if idx > 0 {
                split_at = idx;
            }
        }

        let (head, tail) = remaining.split_at(split_at);
        out.push(format!("{}{}", prefix, head.trim_end()));
        remaining = tail.trim_start();
        is_first = false;
    }

    out
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_app_starts_in_boot_splash() {
        let app = App::new();
        assert_eq!(app.state(), &AppState::BootSplash);
        assert!(!app.should_exit());
    }

    #[test]
    fn test_initialize_transitions_to_welcome() {
        let mut app = App::new();
        app.initialize().unwrap();
        assert_eq!(app.state(), &AppState::Welcome);
    }

    #[test]
    fn test_quit_on_q_key() {
        let mut app = App::new();
        app.initialize().unwrap();
        app.handle_input('Q').unwrap();
        assert_eq!(app.state(), &AppState::Exit);
        assert!(app.should_exit());
    }

    #[test]
    fn test_display_text() {
        let app = App::new();
        let text = app.get_display_text();
        assert!(!text.is_empty());
        assert!(text[0].contains("TruthDB"));
    }
}

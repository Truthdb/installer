//! Main application state machine

use anyhow::Result;
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
}

impl App {
    /// Create a new application instance
    pub fn new() -> Self {
        info!("Creating new application instance");
        Self { state: AppState::BootSplash, should_exit: false }
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
        self.state = AppState::Error(error);
    }

    /// Get display text for current state
    pub fn get_display_text(&self) -> Vec<String> {
        match &self.state {
            AppState::BootSplash => {
                vec!["TruthDB Installer".to_string(), "Initializing...".to_string()]
            }
            AppState::Welcome => vec![
                "TruthDB Installer".to_string(),
                "Status: booted".to_string(),
                "Press Q to quit (for now)".to_string(),
            ],
            AppState::Error(msg) => vec![
                "TruthDB Installer".to_string(),
                format!("ERROR: {}", msg),
                "Press Q to quit".to_string(),
            ],
            AppState::Exit => vec!["TruthDB Installer".to_string(), "Shutting down...".to_string()],
        }
    }
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
        assert!(text.len() >= 2);
        assert!(text[0].contains("TruthDB"));
    }
}

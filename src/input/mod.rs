//! Input handling module
//!
//! Provides keyboard input handling via evdev

pub mod evdev_handler;

use anyhow::Result;

/// Trait for input handlers
pub trait InputHandler {
    /// Initialize the input handler
    fn init(&mut self) -> Result<()>;
    
    /// Poll for input events (non-blocking)
    /// Returns Some(char) if a key was pressed, None otherwise
    fn poll(&mut self) -> Result<Option<char>>;
    
    /// Cleanup input handler
    fn cleanup(&mut self) -> Result<()>;
}

/// Create an input handler
pub fn create_handler() -> Result<Box<dyn InputHandler>> {
    evdev_handler::EvdevHandler::new()
        .map(|h| Box::new(h) as Box<dyn InputHandler>)
}

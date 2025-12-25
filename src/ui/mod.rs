//! UI rendering module
//!
//! Provides framebuffer-based UI rendering for initramfs environment

pub mod fb;

use anyhow::Result;

/// Trait for UI backends
pub trait UiBackend {
    /// Initialize the UI backend
    fn init(&mut self) -> Result<()>;

    /// Clear screen to a solid color
    fn clear(&mut self, r: u8, g: u8, b: u8) -> Result<()>;

    /// Render text lines at specific positions
    fn render_text(&mut self, lines: &[String]) -> Result<()>;

    /// Flush/present the frame
    fn present(&mut self) -> Result<()>;

    /// Cleanup and restore terminal state
    fn cleanup(&mut self) -> Result<()>;
}

/// Create the appropriate UI backend
pub fn create_backend() -> Result<Box<dyn UiBackend>> {
    // For MVP, we'll use the framebuffer backend
    // In the future, could try DRM first, then fall back to FB
    fb::FramebufferBackend::new().map(|b| Box::new(b) as Box<dyn UiBackend>)
}

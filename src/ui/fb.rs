//! Linux framebuffer UI backend

use anyhow::{anyhow, Context, Result};
use nix::libc;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use tracing::{debug, info, warn};

use super::UiBackend;

const FB_DEVICE: &str = "/dev/fb0";
const FBIOGET_VSCREENINFO: libc::c_int = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_int = 0x4602;

/// Linux framebuffer fixed screen info
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct FbFixScreeninfo {
    id: [u8; 16],
    smem_start: u64,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: u64,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

/// Linux framebuffer variable screen info
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

/// Simple 8x8 bitmap font for basic text rendering
const FONT_8X8: [[u8; 8]; 128] = include!("font_8x8.rs");

/// Framebuffer backend implementation
pub struct FramebufferBackend {
    fb_file: Option<File>,
    width: u32,
    height: u32,
    bits_per_pixel: u32,
    line_length: u32,
    buffer: Vec<u8>,
    fallback_mode: bool,
}

impl FramebufferBackend {
    /// Create a new framebuffer backend
    pub fn new() -> Result<Self> {
        Ok(Self {
            fb_file: None,
            width: 0,
            height: 0,
            bits_per_pixel: 0,
            line_length: 0,
            buffer: Vec::new(),
            fallback_mode: false,
        })
    }

    /// Try to open and initialize framebuffer
    fn try_init_fb(&mut self) -> Result<()> {
        let fb_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(FB_DEVICE)
            .context(format!("Failed to open {}", FB_DEVICE))?;

        let fd = fb_file.as_raw_fd();

        // Get variable screen info
        let mut vinfo: FbVarScreeninfo = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::ioctl(fd, FBIOGET_VSCREENINFO, &mut vinfo as *mut _) };
        if ret != 0 {
            return Err(anyhow!("FBIOGET_VSCREENINFO ioctl failed"));
        }

        // Get fixed screen info
        let mut finfo: FbFixScreeninfo = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::ioctl(fd, FBIOGET_FSCREENINFO, &mut finfo as *mut _) };
        if ret != 0 {
            return Err(anyhow!("FBIOGET_FSCREENINFO ioctl failed"));
        }

        self.width = vinfo.xres;
        self.height = vinfo.yres;
        self.bits_per_pixel = vinfo.bits_per_pixel;
        self.line_length = finfo.line_length;

        info!(
            "Framebuffer initialized: {}x{} @ {} bpp, line_length={}",
            self.width, self.height, self.bits_per_pixel, self.line_length
        );

        // Allocate buffer
        let buffer_size = (self.line_length * self.height) as usize;
        self.buffer = vec![0u8; buffer_size];

        self.fb_file = Some(fb_file);
        Ok(())
    }

    /// Draw a pixel at (x, y) with color (r, g, b)
    fn put_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height {
            return;
        }

        let offset = (y * self.line_length + x * (self.bits_per_pixel / 8)) as usize;

        if offset + 3 < self.buffer.len() {
            match self.bits_per_pixel {
                32 => {
                    // BGRA or RGBA format
                    self.buffer[offset] = b;
                    self.buffer[offset + 1] = g;
                    self.buffer[offset + 2] = r;
                    self.buffer[offset + 3] = 255; // Alpha
                }
                24 => {
                    // BGR or RGB format
                    self.buffer[offset] = b;
                    self.buffer[offset + 1] = g;
                    self.buffer[offset + 2] = r;
                }
                16 => {
                    // RGB565 format
                    let rgb565 = ((r as u16 & 0xF8) << 8)
                        | ((g as u16 & 0xFC) << 3)
                        | ((b as u16 & 0xF8) >> 3);
                    self.buffer[offset] = (rgb565 & 0xFF) as u8;
                    self.buffer[offset + 1] = ((rgb565 >> 8) & 0xFF) as u8;
                }
                _ => {}
            }
        }
    }

    /// Draw a character at position (x, y)
    fn draw_char(&mut self, c: char, x: u32, y: u32, r: u8, g: u8, b: u8) {
        let idx = c as usize;
        if idx >= 128 {
            return;
        }

        let glyph = FONT_8X8[idx];
        for (row, &byte) in glyph.iter().enumerate() {
            for col in 0..8 {
                if byte & (1 << (7 - col)) != 0 {
                    self.put_pixel(x + col, y + row as u32, r, g, b);
                }
            }
        }
    }

    /// Draw a string at position (x, y)
    fn draw_string(&mut self, s: &str, x: u32, y: u32, r: u8, g: u8, b: u8) {
        for (i, c) in s.chars().enumerate() {
            self.draw_char(c, x + (i as u32 * 8), y, r, g, b);
        }
    }

    /// Fallback text output to console
    fn fallback_render(&self, lines: &[String]) -> Result<()> {
        // Clear screen using ANSI escape codes
        print!("\x1B[2J\x1B[H");

        println!("╔════════════════════════════════════════╗");
        for line in lines {
            println!("║ {:<38} ║", line);
        }
        println!("╚════════════════════════════════════════╝");

        std::io::stdout().flush()?;
        Ok(())
    }
}

impl UiBackend for FramebufferBackend {
    fn init(&mut self) -> Result<()> {
        match self.try_init_fb() {
            Ok(_) => {
                info!("Framebuffer backend initialized successfully");
                self.fallback_mode = false;
                Ok(())
            }
            Err(e) => {
                warn!("Failed to initialize framebuffer: {}. Using fallback mode.", e);
                self.fallback_mode = true;
                // Don't fail - we'll use console fallback
                Ok(())
            }
        }
    }

    fn clear(&mut self, r: u8, g: u8, b: u8) -> Result<()> {
        if self.fallback_mode {
            return Ok(());
        }

        debug!("Clearing screen to RGB({}, {}, {})", r, g, b);
        for y in 0..self.height {
            for x in 0..self.width {
                self.put_pixel(x, y, r, g, b);
            }
        }
        Ok(())
    }

    fn render_text(&mut self, lines: &[String]) -> Result<()> {
        if self.fallback_mode {
            return self.fallback_render(lines);
        }

        debug!("Rendering {} lines of text", lines.len());
        let start_y = 100; // Start 100 pixels from top
        let line_height = 20;

        for (i, line) in lines.iter().enumerate() {
            let y = start_y + (i as u32 * line_height);
            self.draw_string(line, 50, y, 255, 255, 255); // White text
        }

        Ok(())
    }

    fn present(&mut self) -> Result<()> {
        if self.fallback_mode {
            return Ok(());
        }

        if let Some(ref mut fb_file) = self.fb_file {
            use std::io::{Seek, SeekFrom};

            fb_file.seek(SeekFrom::Start(0))?;
            fb_file.write_all(&self.buffer)?;
            fb_file.flush()?;
            debug!("Frame presented");
        }

        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.fallback_mode {
            // Reset terminal
            print!("\x1B[2J\x1B[H");
            std::io::stdout().flush()?;
        }

        info!("Framebuffer backend cleaned up");
        Ok(())
    }
}

//! A renderer that draws a camera's image as ASCII art in the terminal, with keyboard and mouse
//! input read from the terminal.

/// Conversion of RGBA pixels to ASCII characters by brightness.
pub mod ascii;
/// The ASCII frame shown in the terminal.
pub mod frame;
/// Keyboard and mouse input read from the terminal.
pub mod input;
/// The plugin that sets up terminal rendering.
pub mod plugin;
/// Reading a camera's rendered image back from the GPU and turning it into ASCII.
pub mod readback;
/// Following the terminal's size with the readback buffer and the camera's render target.
pub mod resize;
mod runner;
/// The ratatui terminal the frame is drawn to.
pub mod terminal;

pub use input::TerminalInput;
pub use plugin::TerminalRendererPlugin;
pub use readback::{readback_terminal_frame, TerminalOutput};
pub use runner::run;

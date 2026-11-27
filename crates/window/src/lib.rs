//! The window and input layer of Concerto: the events a host reports, the [`Window`](plugin::Window)
//! resource, keyboard, mouse and gamepad input, local players, actions and the clipboard.
//!
//! It opens no window itself; a host such as `concerto-winit` feeds it events.

/// Copying to and pasting from the clipboard.
pub mod clipboard;
/// The events a host sends about its window and input devices, and folding them into resources.
pub mod events;
/// Keyboard, mouse and gamepad state, local players and actions.
pub mod input;
/// The plugin that adds the window and input resources, and the window's resizable frame.
pub mod plugin;
mod window;
pub use events::{FoldInput, InputGate};

//! The retained-mode UI of Concerto.
//!
//! # Examples
//!
//! ```no_run
//! use concerto_app::App;
//! use concerto_render::plugin::RenderPlugin;
//! use concerto_ui::plugin::UIPlugin;
//! use concerto_window::plugin::WindowPlugin;
//!
//! fn game(app: &mut App) {
//!     app.register_plugin(WindowPlugin)
//!         .register_plugin(RenderPlugin)
//!         .register_plugin(UIPlugin);
//! }
//! ```

pub mod anchor;
pub mod checkbox;
pub mod elements;
pub mod focus;
pub mod frame_stats_overlay;
pub mod interaction;
pub mod material;
pub mod node;
pub mod plugin;
pub mod render;
pub mod scroll;
pub mod sets;
pub mod slider;
pub mod text;
pub mod text_input;
pub mod theme;
pub mod transform;
pub mod tree;
pub mod widgets;

mod resources;
mod vertex;

pub use node::{UISharedViewport, UIViewport};
pub use resources::UIRenderDiagnostics;

#[cfg(test)]
mod tests {}

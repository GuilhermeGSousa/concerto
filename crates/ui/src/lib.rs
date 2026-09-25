//! The retained-mode UI of Concerto.
//!
//! UI is a hierarchy of entities with [`UINode`](node::UINode) components, laid out
//! with flexbox and drawn by the renderer. Widgets such as checkboxes, sliders,
//! text inputs and scroll areas are components on those entities.
//! [`UIPlugin`](plugin::UIPlugin) registers the systems, which run in
//! `LateUpdate` in the order given by [`UiSet`](sets::UiSet).
//!
//! # Examples
//!
//! ```no_run
//! use concerto_app::App;
//! use concerto_render::plugin::RenderPlugin;
//! use concerto_ui::plugin::UIPlugin;
//! use concerto_window::plugin::WindowPlugin;
//!
//! let mut app = App::new();
//! app.register_plugin(WindowPlugin)
//!     .register_plugin(RenderPlugin)
//!     .register_plugin(UIPlugin);
//! app.run();
//! ```

pub mod anchor;
/// Checkboxes.
pub mod checkbox;
/// Keyboard focus and focus traversal.
pub mod focus;
/// An overlay showing frame timing statistics.
pub mod frame_stats_overlay;
/// Pointer hit testing, hover and click events.
pub mod interaction;
/// The material UI nodes are drawn with.
pub mod material;
/// UI nodes and their layout.
pub mod node;
/// The [`UIPlugin`](plugin::UIPlugin).
pub mod plugin;
/// Drawing UI nodes and text.
pub mod render;
pub mod scroll;
/// System sets for ordering against the UI.
pub mod sets;
/// Sliders.
pub mod slider;
/// Text components and fonts.
pub mod text;
/// Editable text inputs.
pub mod text_input;
/// Colors and metrics shared by widgets.
pub mod theme;
/// Values for sizing and positioning nodes.
pub mod transform;
pub mod widgets;

mod resources;
mod vertex;

pub use node::UIViewport;
pub use resources::UIRenderDiagnostics;

#[cfg(test)]
mod tests {}

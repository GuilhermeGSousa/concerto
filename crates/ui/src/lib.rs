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
/// [`UICheckbox`](checkbox::UICheckbox), a toggleable boolean widget.
pub mod checkbox;
/// [`UIColorPicker`](color_picker::UIColorPicker), a colour picker widget.
pub mod color_picker;
pub mod elements;
/// Keyboard focus: which widget holds it and how it moves between
/// [`UIFocusable`](focus::UIFocusable) widgets.
pub mod focus;
/// [`FrameStatsOverlayPlugin`](frame_stats_overlay::FrameStatsOverlayPlugin), an on-screen
/// frame-time readout.
pub mod frame_stats_overlay;
pub mod grid;
/// Pointer input on UI nodes: hover, press, click, double click and drag, sent as signals to the
/// nodes involved.
pub mod interaction;
/// Items of a [`UIList`](list::UIList) or [`UITree`](tree::UITree): selection, activation and
/// keyboard navigation.
pub mod items;
/// [`UIList`](list::UIList), a vertical list of selectable items.
pub mod list;
/// [`UIMaterial`](material::UIMaterial), the fill and border a UI node is drawn with.
pub mod material;
/// [`UINode`](node::UINode), a box of the flex layout, and the layout that sizes and places it.
pub mod node;
/// [`UIPlugin`](plugin::UIPlugin), which lays out, routes input to and draws the UI.
pub mod plugin;
/// Drawing the UI in the render world.
pub mod render;
pub mod scroll;
/// [`UISearchField`](search::UISearchField), a text input that filters a list or tree.
pub mod search;
/// [`UiSet`](sets::UiSet), the phases of the UI in a frame.
pub mod sets;
/// [`UISlider`](slider::UISlider), a draggable range slider widget.
pub mod slider;
/// [`UISplitter`](splitter::UISplitter), a draggable gap between two siblings.
pub mod splitter;
/// [`UIText`](text::UIText), text drawn in a UI node, and the fonts it is shaped with.
pub mod text;
/// [`UITextInput`](text_input::UITextInput), a single-line text field.
pub mod text_input;
/// [`UITheme`](theme::UITheme), the colours and metrics the themed elements are built from.
pub mod theme;
/// [`UIValue`](transform::UIValue), a length along one axis of a UI node.
pub mod transform;
/// [`UITree`](tree::UITree), a vertical tree of expandable, selectable items.
pub mod tree;
pub mod widgets;

mod resources;
mod vertex;

pub use node::{UISharedViewport, UIViewport};
pub use resources::UIRenderDiagnostics;

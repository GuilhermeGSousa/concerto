//! Immediate-mode debug gizmos.
//!
//! Register [`DebugGizmosPlugin`], then request a
//! [`DebugGizmos`] parameter in any system to draw debug
//! shapes for the current frame:
//!
//! ```ignore
//! use concerto_debug_gizmos::DebugGizmos;
//! use concerto_color::LinearRgba;
//! use glam::Vec3;
//!
//! fn draw(mut gizmos: DebugGizmos) {
//!     gizmos.sphere(Vec3::ZERO, 0.5, LinearRgba::RED);
//! }
//! ```
//!
//! Drawing is immediate mode: shapes must be re-issued every frame to stay
//! visible.

/// Gizmos the editor draws for a selected entity's component, and the built-in ones.
pub mod component;
/// The [`DebugGizmos`] system parameter that systems draw with.
pub mod gizmos;
/// The render pipelines that draw gizmo lines.
pub mod pipeline;
/// The plugin that sets gizmo drawing up.
pub mod plugin;
/// The render-world systems that upload and draw each frame's gizmos.
pub mod render;
/// The buffer gizmos drawn during a frame collect in.
pub mod storage;
/// The vertex formats gizmo lines are drawn with.
pub mod vertex;

pub use component::ComponentGizmo;
pub use gizmos::DebugGizmos;
pub use plugin::DebugGizmosPlugin;
pub use storage::GizmoStorage;
pub use vertex::{GizmoVertex, WideGizmoVertex};

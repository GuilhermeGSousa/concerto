//! Immediate-mode debug gizmos.
//!
//! Register [`DebugGizmosPlugin`](plugin::DebugGizmosPlugin), then request a
//! [`DebugGizmos`](gizmos::DebugGizmos) parameter in any system to draw debug
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

pub mod gizmos;
pub mod pipeline;
pub mod plugin;
pub mod render;
pub mod storage;
pub mod vertex;

pub use gizmos::DebugGizmos;
pub use plugin::DebugGizmosPlugin;
pub use storage::GizmoStorage;
pub use vertex::{GizmoVertex, WideGizmoVertex};

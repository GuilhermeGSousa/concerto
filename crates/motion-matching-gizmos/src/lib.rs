//! The debug drawing of motion matching.
//!
//! [`MotionMatchingGizmos`] draws it in the running game, and the colours,
//! shapes and baked frames here are shared with the editor's preview.

mod frame;
mod gizmos;
mod shapes;

pub use frame::BakedFrame;
pub use gizmos::{
    MotionMatchingGizmos, MotionMatchingGizmosPlugin, draw_motion_matching, matched_shapes,
    query_shapes,
};
pub use shapes::{
    CHANNEL_COLORS, DIMMED, MotionMatchingDraw, SIMULATION_ARROW, SIMULATION_COLOR,
    SIMULATION_RADIUS, SIMULATION_WIDTH, channel_color,
};

//! The debug drawing of motion matching: `MotionMatchingGizmos` for the running game, and the colours, shapes and baked frames it shares with the editor's preview.

mod frame;
mod gizmos;
mod shapes;

pub use frame::{BakedFrame, local_pose, placed_pose};
pub use gizmos::{
    MotionMatchingGizmos, MotionMatchingGizmosPlugin, draw_motion_matching, matched_shapes,
    query_shapes,
};
pub use shapes::{
    CHANNEL_COLORS, DIMMED, SIMULATION_ARROW, SIMULATION_COLOR, SIMULATION_RADIUS,
    SIMULATION_WIDTH, channel_color, draw_placed, draw_shape, draw_simulation, placed_shape,
};

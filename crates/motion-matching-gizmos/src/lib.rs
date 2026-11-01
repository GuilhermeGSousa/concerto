//! The debug drawing of motion matching: the colours, shapes and baked frames that the editor's preview draws.

mod frame;
mod shapes;

pub use frame::{BakedFrame, local_pose, placed_pose};
pub use shapes::{
    CHANNEL_COLORS, DIMMED, SIMULATION_ARROW, SIMULATION_COLOR, SIMULATION_RADIUS,
    SIMULATION_WIDTH, channel_color, draw_shape, draw_simulation, placed_shape,
};

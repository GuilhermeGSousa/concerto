use concerto_color::Color;
use concerto_debug_gizmos::DebugGizmos;
use concerto_foundation::transform::Transform;
use concerto_motion_matching::FeatureShape;
use glam::Vec3;

/// The colour of the simulation bone's ring and arrow.
pub const SIMULATION_COLOR: Color = Color::rgba(1.0, 0.85, 0.1, 1.0);
/// The radius of the simulation bone's ring, in metres.
pub const SIMULATION_RADIUS: f32 = 0.3;
/// The length of the simulation bone's arrow, in metres.
pub const SIMULATION_ARROW: f32 = 0.6;
/// The line width of motion matching's gizmos, in pixels.
pub const SIMULATION_WIDTH: f32 = 2.0;
/// The palette feature channels take their colours from in turn.
pub const CHANNEL_COLORS: [Color; 4] = [
    Color::rgba(0.2, 0.8, 1.0, 1.0),
    Color::rgba(0.3, 1.0, 0.4, 1.0),
    Color::rgba(1.0, 0.35, 0.9, 1.0),
    Color::rgba(1.0, 0.5, 0.15, 1.0),
];
/// What a channel's colour is multiplied by while it isn't the picked one.
pub const DIMMED: f32 = 0.4;

/// A shape in the simulation bone's space, moved into the space `bone` lives in.
pub fn placed_shape(bone: &Transform, shape: FeatureShape) -> FeatureShape {
    match shape {
        FeatureShape::Ring { center, radius } => FeatureShape::Ring {
            center: bone.transform_point(center),
            radius,
        },
        FeatureShape::Line { start, end } => FeatureShape::Line {
            start: bone.transform_point(start),
            end: bone.transform_point(end),
        },
        FeatureShape::Arrow { start, end } => FeatureShape::Arrow {
            start: bone.transform_point(start),
            end: bone.transform_point(end),
        },
    }
}

/// Draws a shape that is already placed; a `Ring` lies flat against `up`.
pub fn draw_placed(gizmos: &mut DebugGizmos, up: Vec3, shape: FeatureShape, color: Color) {
    match shape {
        FeatureShape::Ring { center, radius } => gizmos.circle(center, up, radius, color),
        FeatureShape::Line { start, end } => gizmos.line(start, end, color),
        FeatureShape::Arrow { start, end } => gizmos.arrow(start, end, color),
    }
}

/// Draws a shape in the simulation bone's space, placed at `bone`.
pub fn draw_shape(gizmos: &mut DebugGizmos, bone: &Transform, shape: FeatureShape, color: Color) {
    draw_placed(gizmos, bone.local_y(), placed_shape(bone, shape), color);
}

/// The colour of `channel`: full when it's the `picked` one, dimmed otherwise.
pub fn channel_color(channel: usize, picked: Option<usize>) -> Color {
    let color = CHANNEL_COLORS[channel % CHANNEL_COLORS.len()];
    if picked == Some(channel) {
        return color;
    }
    let [red, green, blue, alpha] = color.to_array();
    Color::rgba(red * DIMMED, green * DIMMED, blue * DIMMED, alpha)
}

/// Draws the simulation bone's ring and the arrow of its facing at `bone`.
pub fn draw_simulation(gizmos: &mut DebugGizmos, bone: &Transform) {
    let tip = bone.translation + bone.local_z() * SIMULATION_ARROW;
    gizmos.circle(
        bone.translation,
        bone.local_y(),
        SIMULATION_RADIUS,
        SIMULATION_COLOR,
    );
    gizmos.arrow(bone.translation, tip, SIMULATION_COLOR);
}

#[cfg(test)]
mod tests;

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

/// The colour of `channel`: full when it's the `picked` one, dimmed otherwise.
pub fn channel_color(channel: usize, picked: Option<usize>) -> Color {
    let color = CHANNEL_COLORS[channel % CHANNEL_COLORS.len()];
    if picked == Some(channel) {
        return color;
    }
    let [red, green, blue, alpha] = color.to_array();
    Color::rgba(red * DIMMED, green * DIMMED, blue * DIMMED, alpha)
}

/// Draws motion matching's shapes and simulation bone into a [`DebugGizmos`].
pub trait MotionMatchingDraw {
    /// Draws a shape that is already placed; a `Ring` lies flat against `up`.
    fn draw_placed(&mut self, up: Vec3, shape: FeatureShape, color: Color);

    /// Draws a shape in the simulation bone's space, placed at `bone`.
    fn draw_shape(&mut self, bone: &Transform, shape: FeatureShape, color: Color);

    /// Draws the simulation bone's ring and the arrow of its facing at `bone`.
    fn draw_simulation(&mut self, bone: &Transform);
}

impl MotionMatchingDraw for DebugGizmos<'_> {
    fn draw_placed(&mut self, up: Vec3, shape: FeatureShape, color: Color) {
        match shape {
            FeatureShape::Ring { center, radius } => self.circle(center, up, radius, color),
            FeatureShape::Line { start, end } => self.line(start, end, color),
            FeatureShape::Arrow { start, end } => self.arrow(start, end, color),
        }
    }

    fn draw_shape(&mut self, bone: &Transform, shape: FeatureShape, color: Color) {
        self.draw_placed(bone.local_y(), shape.placed(bone), color);
    }

    fn draw_simulation(&mut self, bone: &Transform) {
        let tip = bone.translation + bone.local_z() * SIMULATION_ARROW;
        self.circle(
            bone.translation,
            bone.local_y(),
            SIMULATION_RADIUS,
            SIMULATION_COLOR,
        );
        self.arrow(bone.translation, tip, SIMULATION_COLOR);
    }
}

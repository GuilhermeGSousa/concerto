use color::Color;
use ecs::{
    resource::ResMut,
    system::{access::SystemAccess, input::SystemInput},
    world::UnsafeWorldCell,
};
use essential::transform::Transform;
use glam::{Quat, Vec2, Vec3};

use crate::storage::GizmoStorage;

/// Number of straight segments used to approximate a full circle.
const DEFAULT_CIRCLE_SEGMENTS: usize = 32;

/// Immediate-mode debug drawing handle.
///
/// `DebugGizmos` is a system parameter (a [`SystemInput`]): request one in any
/// system and call its drawing methods to render debug shapes for the current
/// frame.  Nothing is retained — a shape is only visible on frames where it is
/// drawn, so calls are typically issued every frame:
///
/// ```ignore
/// use debug_gizmos::DebugGizmos;
/// use color::LinearRgba;
/// use glam::Vec3;
///
/// fn draw_debug(mut gizmos: DebugGizmos) {
///     gizmos.line(Vec3::ZERO, Vec3::X, LinearRgba::RED);
///     gizmos.sphere(Vec3::Y, 0.5, LinearRgba::GREEN);
///     gizmos.cuboid(&Transform::from_translation(Vec3::Z), LinearRgba::BLUE);
/// }
/// ```
///
/// All shapes are rendered as line lists; there is no filled geometry.
pub struct DebugGizmos<'w> {
    storage: ResMut<'w, GizmoStorage>,
}

impl<'w> DebugGizmos<'w> {
    /// Draws a single line between two points.
    #[inline]
    pub fn line(&mut self, start: Vec3, end: Vec3, color: Color) {
        self.storage.push_line(start, end, color, color);
    }

    /// Draws a line whose colour is interpolated from `start_color` to
    /// `end_color` along its length.
    #[inline]
    pub fn line_gradient(&mut self, start: Vec3, end: Vec3, start_color: Color, end_color: Color) {
        self.storage.push_line(start, end, start_color, end_color);
    }

    /// Draws a line from `start` extending along `vector`.
    #[inline]
    pub fn ray(&mut self, start: Vec3, vector: Vec3, color: Color) {
        self.line(start, start + vector, color);
    }

    /// Draws a ray with a colour gradient from origin to tip.
    #[inline]
    pub fn ray_gradient(
        &mut self,
        start: Vec3,
        vector: Vec3,
        start_color: Color,
        end_color: Color,
    ) {
        self.line_gradient(start, start + vector, start_color, end_color);
    }

    /// Connects a sequence of points with line segments (an open polyline).
    pub fn linestrip(&mut self, points: impl IntoIterator<Item = Vec3>, color: Color) {
        let mut iter = points.into_iter();
        let Some(mut prev) = iter.next() else {
            return;
        };
        for point in iter {
            self.line(prev, point, color);
            prev = point;
        }
    }

    /// Draws a small three-axis cross centred on `position`.
    pub fn cross(&mut self, position: Vec3, size: f32, color: Color) {
        let h = size * 0.5;
        self.line(position - Vec3::X * h, position + Vec3::X * h, color);
        self.line(position - Vec3::Y * h, position + Vec3::Y * h, color);
        self.line(position - Vec3::Z * h, position + Vec3::Z * h, color);
    }

    /// Draws a circle of `radius` centred at `center`, lying in the plane whose
    /// normal is `normal`.
    pub fn circle(&mut self, center: Vec3, normal: Vec3, radius: f32, color: Color) {
        let (tangent, bitangent) = orthonormal_basis(normal);
        let seg = DEFAULT_CIRCLE_SEGMENTS;
        let mut prev = center + tangent * radius;
        for i in 1..=seg {
            let angle = (i as f32 / seg as f32) * std::f32::consts::TAU;
            let (s, c) = angle.sin_cos();
            let next = center + (tangent * c + bitangent * s) * radius;
            self.line(prev, next, color);
            prev = next;
        }
    }

    /// Draws a wireframe sphere as three great circles aligned to the world axes.
    pub fn sphere(&mut self, center: Vec3, radius: f32, color: Color) {
        self.circle(center, Vec3::Z, radius, color);
        self.circle(center, Vec3::Y, radius, color);
        self.circle(center, Vec3::X, radius, color);
    }

    /// Draws the outline of a rectangle centred at `center`, lying in the plane
    /// with the given `normal`, sized `size` (width along the plane's first
    /// tangent, height along the second).
    pub fn rect(&mut self, center: Vec3, normal: Vec3, size: Vec2, color: Color) {
        let (tangent, bitangent) = orthonormal_basis(normal);
        let hx = tangent * (size.x * 0.5);
        let hy = bitangent * (size.y * 0.5);
        let a = center - hx - hy;
        let b = center + hx - hy;
        let c = center + hx + hy;
        let d = center - hx + hy;
        self.linestrip([a, b, c, d, a], color);
    }

    /// Draws the twelve edges of a box described by `transform`.
    ///
    /// The box is the unit cube (side length 1, centred at the origin) placed
    /// and sized by the transform, so a `Transform` with translation `t` and
    /// scale `s` yields a box of dimensions `s` centred at `t`.
    pub fn cuboid(&mut self, transform: &Transform, color: Color) {
        let matrix = transform.compute_matrix();
        let corners: [Vec3; 8] = [
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(0.5, -0.5, -0.5),
            Vec3::new(0.5, 0.5, -0.5),
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::new(-0.5, -0.5, 0.5),
            Vec3::new(0.5, -0.5, 0.5),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(-0.5, 0.5, 0.5),
        ]
        .map(|c| matrix.transform_point3(c));

        const EDGES: [(usize, usize); 12] = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];
        for (a, b) in EDGES {
            self.line(corners[a], corners[b], color);
        }
    }

    /// Convenience wrapper around [`cuboid`](Self::cuboid): an axis-aligned box
    /// of the given `size` centred at `center`.
    pub fn cuboid_min_max(&mut self, center: Vec3, size: Vec3, color: Color) {
        let transform = Transform::from_translation_rotation_scale(center, Quat::IDENTITY, size);
        self.cuboid(&transform, color);
    }

    /// Draws the local axes of `transform`: X in red, Y in green, Z in blue,
    /// each extending `length` units from the origin.
    pub fn axes(&mut self, transform: &Transform, length: f32) {
        let origin = transform.translation;
        self.line(origin, origin + transform.local_x() * length, Color::RED);
        self.line(origin, origin + transform.local_y() * length, Color::GREEN);
        self.line(origin, origin + transform.local_z() * length, Color::BLUE);
    }

    /// Draws an arrow from `start` to `end`, including a small arrowhead at the
    /// tip.
    pub fn arrow(&mut self, start: Vec3, end: Vec3, color: Color) {
        self.line(start, end, color);

        let direction = end - start;
        let length = direction.length();
        if length <= f32::EPSILON {
            return;
        }
        let dir = direction / length;
        let (tangent, bitangent) = orthonormal_basis(dir);

        let head = (length * 0.1).min(0.5);
        let back = end - dir * head;
        for offset in [tangent, -tangent, bitangent, -bitangent] {
            self.line(end, back + offset * head, color);
        }
    }
}

/// Builds a right-handed orthonormal basis `(tangent, bitangent)` for the plane
/// perpendicular to `normal`.  `normal` need not be unit length.
fn orthonormal_basis(normal: Vec3) -> (Vec3, Vec3) {
    let n = normal.normalize_or_zero();
    let n = if n == Vec3::ZERO { Vec3::Z } else { n };
    // Pick a reference axis that is not parallel to `n`.
    let reference = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let tangent = reference.cross(n).normalize();
    let bitangent = n.cross(tangent);
    (tangent, bitangent)
}

impl<'w> SystemInput for DebugGizmos<'w> {
    type State = ();
    type Data<'world, 'state> = DebugGizmos<'world>;

    fn init_state(_world: &mut ecs::World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        DebugGizmos {
            storage: ResMut::new(world),
        }
    }

    fn fill_access(_meta: &mut ecs::system::meta::SystemMetadata, access: &mut SystemAccess) {
        access.write_resource::<GizmoStorage>();
    }
}

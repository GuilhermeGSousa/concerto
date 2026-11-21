use concerto_color::Color;
use concerto_debug_gizmos::DebugGizmos;
use concerto_foundation::transform::{GlobalTransform, Transform};
use concerto_physics::shape::{PhysicsShape, ShapeKind};
use glam::{Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LocalShape {
    Sphere { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
    Cuboid { center: Vec3, half_extents: Vec3 },
}

impl LocalShape {
    pub(crate) fn of(shape: &PhysicsShape) -> Self {
        match shape.kind() {
            ShapeKind::Sphere { radius } => Self::Sphere { radius },
            ShapeKind::Capsule {
                half_height,
                radius,
            } => Self::Capsule {
                half_height,
                radius,
            },
            ShapeKind::Cuboid { half_extents } => Self::Cuboid {
                center: Vec3::ZERO,
                half_extents,
            },
            ShapeKind::Mesh => {
                let bounds = shape.local_aabb();
                Self::bounds(*bounds.min(), *bounds.max())
            }
        }
    }

    pub(crate) fn bounds(min: Vec3, max: Vec3) -> Self {
        Self::Cuboid {
            center: (min + max) * 0.5,
            half_extents: (max - min) * 0.5,
        }
    }

    fn center(&self) -> Vec3 {
        match self {
            Self::Cuboid { center, .. } => *center,
            _ => Vec3::ZERO,
        }
    }

    pub(crate) fn draw(
        &self,
        gizmos: &mut DebugGizmos,
        center: Vec3,
        rotation: Quat,
        scale: Vec3,
        color: Color,
    ) {
        let scale = scale.abs();
        match *self {
            Self::Sphere { radius } => gizmos.sphere(center, radius * scale.max_element(), color),
            Self::Capsule {
                half_height,
                radius,
            } => gizmos.capsule(
                center,
                rotation,
                half_height * scale.y,
                radius * scale.x.max(scale.z),
                color,
            ),
            Self::Cuboid { half_extents, .. } => gizmos.cuboid(
                &Transform::from_translation_rotation_scale(
                    center,
                    rotation,
                    half_extents * 2.0 * scale,
                ),
                color,
            ),
        }
    }

    pub(crate) fn draw_at(
        &self,
        gizmos: &mut DebugGizmos,
        transform: &GlobalTransform,
        offset: Vec3,
        color: Color,
    ) {
        let center = transform.matrix().transform_point3(offset + self.center());
        self.draw(
            gizmos,
            center,
            transform.rotation(),
            transform.scale(),
            color,
        );
    }
}

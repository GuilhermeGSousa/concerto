pub use concerto_foundation_macros::Blendable;
use glam::{Quat, Vec3, Vec3A};

pub trait Blendable {
    fn interpolate(from: Self, to: Self, t: f32) -> Self;
}

impl Blendable for Vec3 {
    fn interpolate(from: Self, to: Self, t: f32) -> Self {
        from.lerp(to, t)
    }
}

impl Blendable for Vec3A {
    fn interpolate(from: Self, to: Self, t: f32) -> Self {
        from.lerp(to, t)
    }
}

impl Blendable for Quat {
    fn interpolate(from: Self, to: Self, t: f32) -> Self {
        from.slerp(to, t)
    }
}

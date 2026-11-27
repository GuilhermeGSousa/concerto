pub use concerto_foundation_macros::Blendable;
use glam::{Quat, Vec3, Vec3A};

/// A value that can be interpolated between two others.
///
/// Derive it on a struct whose fields are all `Blendable` to interpolate field by field.
pub trait Blendable {
    /// Returns the value a fraction `t` of the way from `from` to `to`, where `t` is nominally 0.0
    /// to 1.0; a [`Quat`] uses spherical interpolation.
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

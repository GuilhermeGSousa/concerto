use concerto_physics::ragdoll::{BodyPose, PartSettings, PartShape};
use glam::Vec3;

use crate::resolve::ResolvedRagdoll;

/// Two parts, not joined to each other, whose shapes overlap in the rest pose.
///
/// Physics pushes them apart the moment the body is simulated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartOverlap {
    /// The index of one overlapping part.
    pub first: usize,
    /// The index of the other overlapping part.
    pub second: usize,
    /// How far the shapes reach into each other, in metres.
    pub depth: f32,
}

pub(crate) struct Rounded {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
    pub(crate) radius: f32,
}

impl Rounded {
    fn of(part: &PartSettings) -> Self {
        Self::at(part, part.pose)
    }
}

impl Rounded {
    pub(crate) fn at(part: &PartSettings, pose: BodyPose) -> Self {
        let pose: BodyPose = pose.then(part.shape_offset);
        let along = |axis: Vec3, half: f32, radius: f32| Self {
            start: pose.transform_point(-axis * half),
            end: pose.transform_point(axis * half),
            radius,
        };
        match part.shape {
            PartShape::Sphere { radius } => along(Vec3::Y, 0.0, radius),
            PartShape::Capsule {
                half_height,
                radius,
            } => along(Vec3::Y, half_height, radius),
            PartShape::Box { half_extents } => {
                let mut extents = [
                    (half_extents.x, Vec3::X),
                    (half_extents.y, Vec3::Y),
                    (half_extents.z, Vec3::Z),
                ]
                .map(|(extent, axis)| Extent { extent, axis });
                extents.sort_by(|a, b| b.extent.total_cmp(&a.extent));
                let radius = extents[1].extent;
                along(
                    extents[0].axis,
                    (extents[0].extent - radius).max(0.0),
                    radius,
                )
            }
        }
    }
}

struct Extent {
    extent: f32,
    axis: Vec3,
}

impl Rounded {
    fn distance(&self, other: &Self) -> f32 {
        let d1 = self.end - self.start;
        let d2 = other.end - other.start;
        let r = self.start - other.start;
        let length1 = d1.length_squared();
        let length2 = d2.length_squared();
        let f = d2.dot(r);
        let (s, t) = if length1 <= f32::EPSILON && length2 <= f32::EPSILON {
            (0.0, 0.0)
        } else if length1 <= f32::EPSILON {
            (0.0, (f / length2).clamp(0.0, 1.0))
        } else {
            let c = d1.dot(r);
            if length2 <= f32::EPSILON {
                ((-c / length1).clamp(0.0, 1.0), 0.0)
            } else {
                let other = d1.dot(d2);
                let denominator = length1 * length2 - other * other;
                let s = if denominator > f32::EPSILON {
                    ((other * f - c * length2) / denominator).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let t = (other * s + f) / length2;
                if t < 0.0 {
                    ((-c / length1).clamp(0.0, 1.0), 0.0)
                } else if t > 1.0 {
                    (((other - c) / length1).clamp(0.0, 1.0), 1.0)
                } else {
                    (s, t)
                }
            }
        };
        (self.start + d1 * s).distance(other.start + d2 * t)
    }
}

impl ResolvedRagdoll {
    /// The pairs of parts that overlap in the rest pose and are not joined to each other, deepest first.
    ///
    /// A box is judged as a capsule along its longest side.
    pub fn overlapping_parts(&self) -> Vec<PartOverlap> {
        let parts = &self.settings().parts;
        let shapes: Vec<Rounded> = parts.iter().map(Rounded::of).collect();
        let mut overlaps = Vec::new();
        for first in 0..parts.len() {
            for second in first + 1..parts.len() {
                if parts[second].parent == Some(first) || parts[first].parent == Some(second) {
                    continue;
                }
                let depth = shapes[first].radius + shapes[second].radius
                    - shapes[first].distance(&shapes[second]);
                if depth > 0.0 {
                    overlaps.push(PartOverlap {
                        first,
                        second,
                        depth,
                    });
                }
            }
        }
        overlaps.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        overlaps
    }
}

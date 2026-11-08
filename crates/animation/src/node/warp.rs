use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Affine3A, Quat, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
    clip::ClipRange,
    pose::{RootMotion, twist_about},
};

/// Bends a clip's root motion over `window`, each frame closing the share of the window's time it covers, so the root space ends on the `WarpTarget` under `target`; it lands exactly only with the skeleton at the character's origin and the clip reaching the root unblended at the window's end.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct RootMotionWarp {
    /// The blackboard param the game puts the `WarpTarget` under.
    pub target: Name,
    /// The span of the clip the warp works over, or the whole clip when `None`.
    pub window: Option<ClipRange>,
    /// Whether the horizontal translation is corrected; off, a yaw correction still turns the clip's later path, so the end point can move.
    pub translation: bool,
    /// Whether the yaw is warped.
    pub rotation: bool,
}

impl Default for RootMotionWarp {
    fn default() -> Self {
        Self::new(Name::default())
    }
}

impl RootMotionWarp {
    /// Warps translation and rotation over the whole clip toward the `WarpTarget` under `target`.
    pub fn new(target: Name) -> Self {
        Self {
            target,
            window: None,
            translation: true,
            rotation: true,
        }
    }

    /// Warps only over this span of the clip.
    pub fn with_window(mut self, window: ClipRange) -> Self {
        self.window = Some(window);
        self
    }

    /// Whether the horizontal translation is warped.
    pub fn with_translation(mut self, translation: bool) -> Self {
        self.translation = translation;
        self
    }

    /// Whether the yaw is warped.
    pub fn with_rotation(mut self, rotation: bool) -> Self {
        self.rotation = rotation;
        self
    }
}

/// Where a warped clip's root space ends, in world space: its ground position and its yaw about +Y; on a BVH skeleton, whose root rotation is part of the space, the yaw lands exactly only if the root's local twist about +Y holds steady over the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct WarpTarget {
    /// The world position the root space's ground point ends on; its height is ignored.
    pub position: Vec3,
    /// The world rotation whose twist about +Y the root space's yaw ends on; its pitch and roll are ignored.
    pub rotation: Quat,
}

pub(crate) struct WarpAim {
    window: ClipRange,
    target: WarpTarget,
    space: Affine3A,
    turn: Quat,
    translation: bool,
    rotation: bool,
}

impl RootMotionWarp {
    pub(crate) fn aim(
        &self,
        blackboard: &AnimationBlackboard,
        space: Affine3A,
        duration: f32,
    ) -> Option<WarpAim> {
        let target = *blackboard.get::<WarpTarget>(self.target)?;
        if !target.position.is_finite() || !target.rotation.is_finite() {
            return None;
        }
        if !space.is_finite() || space.matrix3.determinant() == 0.0 {
            return None;
        }
        let window = self.window.unwrap_or(ClipRange::new(0.0, duration));
        let window = ClipRange::new(
            window.start.max(0.0).min(duration),
            window.end.max(0.0).min(duration),
        );
        if window.start >= window.end {
            return None;
        }
        Some(WarpAim {
            window,
            target,
            space,
            turn: space.to_scale_rotation_translation().1,
            translation: self.translation,
            rotation: self.rotation,
        })
    }
}

impl WarpAim {
    pub(crate) fn bend(
        &self,
        delta: RootMotion,
        span: ClipRange,
        stretch: impl Fn(f32, f32) -> RootMotion,
    ) -> RootMotion {
        let from = span.start.max(self.window.start);
        let to = span.end.min(self.window.end);
        if from >= to {
            return delta;
        }
        let share = (to - from) / (self.window.end - from);
        let remaining = if span.end < self.window.end {
            stretch(span.end, self.window.end)
        } else {
            RootMotion::IDENTITY
        };

        let mut bent = delta;
        if self.rotation {
            let end = self.world_yaw(remaining.rotation)
                * self.world_yaw(delta.rotation)
                * twist_about(self.turn, Vec3::Y);
            let error = shortest(twist_about(self.target.rotation, Vec3::Y) * end.inverse());
            let correction = Quat::from_scaled_axis(error.to_scaled_axis() * share);
            bent.rotation =
                (self.turn.inverse() * correction * self.turn * delta.rotation).normalize();
        }
        if self.translation {
            let end = ground(Vec3::from(self.space.translation))
                + ground(self.space.transform_vector3(bent.translation))
                + ground(
                    self.world_yaw(bent.rotation)
                        * self.space.transform_vector3(remaining.translation),
                );
            let error = ground(self.target.position - end);
            bent.translation += self.space.inverse().transform_vector3(error * share);
        }
        bent
    }

    fn world_yaw(&self, rotation: Quat) -> Quat {
        twist_about(self.turn * rotation * self.turn.inverse(), Vec3::Y)
    }
}

fn ground(vector: Vec3) -> Vec3 {
    vector.with_y(0.0)
}

fn shortest(rotation: Quat) -> Quat {
    if rotation.w < 0.0 {
        -rotation
    } else {
        rotation
    }
}

use std::f32::consts::FRAC_PI_4;

use concerto_director::{Lens, VirtualCamera};
use concerto_ecs::{
    Component, Query, Res,
    entity::hierarchy::Children,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::time::Time;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;

use crate::camera::CameraBoom;

const SETTLE_EPSILON: f32 = 1e-4;
const MIN_BOOM_DISTANCE: f32 = 1e-3;

/// Which framing a third-person camera eases toward; set by gameplay code.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraStance {
    #[default]
    Follow,
    Crouch,
    Aim,
}

/// The side of the character the camera looks over.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShoulderSide {
    #[default]
    Right,
    Left,
}

/// Where a third-person camera sits, its lens and its look, in one stance.
#[derive(Reflect, Clone, Copy, Debug, PartialEq)]
pub struct CameraFraming {
    /// Up from the pivot, in its level frame.
    pub height: f32,
    /// Toward the side's shoulder, along the view's right for `Right` and its left for `Left`.
    pub shoulder: f32,
    /// The boom length back from the shoulder point.
    pub distance: f32,
    /// The lens's vertical field of view, in radians.
    pub fovy: f32,
    /// Multiplies `CameraSettings::sensitivity`.
    pub sensitivity_scale: f32,
    /// The pivot's pitch clamp, in radians, either way.
    pub pitch_limit: f32,
    /// How fast the camera eases toward this framing; 0 or less snaps.
    pub halflife: f32,
}

impl Default for CameraFraming {
    fn default() -> Self {
        Self {
            height: 0.0,
            shoulder: 0.0,
            distance: 3.0,
            fovy: FRAC_PI_4,
            sensitivity_scale: 1.0,
            pitch_limit: 30f32.to_radians(),
            halflife: 0.2,
        }
    }
}

impl CameraFraming {
    pub fn with_height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn with_shoulder(mut self, shoulder: f32) -> Self {
        self.shoulder = shoulder;
        self
    }

    pub fn with_distance(mut self, distance: f32) -> Self {
        self.distance = distance;
        self
    }

    pub fn with_fovy(mut self, fovy: f32) -> Self {
        self.fovy = fovy;
        self
    }

    pub fn with_sensitivity_scale(mut self, sensitivity_scale: f32) -> Self {
        self.sensitivity_scale = sensitivity_scale;
        self
    }

    pub fn with_pitch_limit(mut self, pitch_limit: f32) -> Self {
        self.pitch_limit = pitch_limit;
        self
    }

    pub fn with_halflife(mut self, halflife: f32) -> Self {
        self.halflife = halflife;
        self
    }

    /// This framing after `dt` seconds of closing half its gap to `target` every `halflife`, settling on fields within 1e-4.
    pub fn eased_toward(&self, target: &CameraFraming, dt: f32, halflife: f32) -> CameraFraming {
        if halflife <= 0.0 {
            return *target;
        }
        let decay = 0.5f32.powf(dt / halflife);
        let ease = |from: f32, to: f32| {
            let eased = to + (from - to) * decay;
            if (eased - to).abs() <= SETTLE_EPSILON {
                to
            } else {
                eased
            }
        };
        CameraFraming {
            height: ease(self.height, target.height),
            shoulder: ease(self.shoulder, target.shoulder),
            distance: ease(self.distance, target.distance),
            fovy: ease(self.fovy, target.fovy),
            sensitivity_scale: ease(self.sensitivity_scale, target.sensitivity_scale),
            pitch_limit: ease(self.pitch_limit, target.pitch_limit),
            halflife: target.halflife,
        }
    }
}

/// Eases a pivot's boomed camera between follow, crouch and aim framings.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct ThirdPersonCamera {
    pub stance: CameraStance,
    pub side: ShoulderSide,
    pub follow: CameraFraming,
    pub crouch: CameraFraming,
    pub aim: CameraFraming,
    #[reflect(skip)]
    current: Option<CameraFraming>,
}

impl Default for ThirdPersonCamera {
    fn default() -> Self {
        Self {
            stance: CameraStance::Follow,
            side: ShoulderSide::Right,
            follow: CameraFraming::default(),
            crouch: CameraFraming::default()
                .with_height(-0.5)
                .with_distance(2.5),
            aim: CameraFraming::default()
                .with_shoulder(0.5)
                .with_distance(1.2)
                .with_fovy(35f32.to_radians())
                .with_sensitivity_scale(0.5)
                .with_pitch_limit(60f32.to_radians())
                .with_halflife(0.1),
            current: None,
        }
    }
}

impl ThirdPersonCamera {
    pub fn with_stance(mut self, stance: CameraStance) -> Self {
        self.stance = stance;
        self
    }

    pub fn with_side(mut self, side: ShoulderSide) -> Self {
        self.side = side;
        self
    }

    pub fn with_follow(mut self, follow: CameraFraming) -> Self {
        self.follow = follow;
        self
    }

    pub fn with_crouch(mut self, crouch: CameraFraming) -> Self {
        self.crouch = crouch;
        self
    }

    pub fn with_aim(mut self, aim: CameraFraming) -> Self {
        self.aim = aim;
        self
    }

    /// The framing set for `stance`.
    pub fn framing(&self, stance: CameraStance) -> &CameraFraming {
        match stance {
            CameraStance::Follow => &self.follow,
            CameraStance::Crouch => &self.crouch,
            CameraStance::Aim => &self.aim,
        }
    }

    /// The stance's framing with `shoulder` signed for the side: positive is the view's right.
    pub fn target(&self) -> CameraFraming {
        let framing = *self.framing(self.stance);
        match self.side {
            ShoulderSide::Right => framing,
            ShoulderSide::Left => framing.with_shoulder(-framing.shoulder),
        }
    }

    /// The eased framing, or `target()` before the first step.
    pub fn current(&self) -> CameraFraming {
        self.current.unwrap_or_else(|| self.target())
    }

    /// What `current` becomes after `dt` seconds: `target()` on the first step, else eased toward it.
    pub fn stepped(&self, dt: f32) -> CameraFraming {
        let target = self.target();
        match self.current {
            None => target,
            Some(current) => current.eased_toward(&target, dt, target.halflife),
        }
    }

    /// Eases `current` by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.current = Some(self.stepped(dt));
    }
}

/// Eases each rig toward its stance and writes the framing onto its children's booms and lenses when it differs.
pub fn update_third_person_cameras(
    rigs: Query<(&mut ThirdPersonCamera, &Children)>,
    booms: Query<&mut CameraBoom>,
    cameras: Query<&mut VirtualCamera>,
    time: Res<Time>,
) {
    let dt = time.delta().as_secs_f32();

    for (mut rig, children) in rigs.iter() {
        let next = rig.stepped(dt);
        if rig.current != Some(next) {
            rig.current = Some(next);
        }
        let framing = rig.current();

        for child in children.iter() {
            if let Some(mut boom) = booms.get_entity(*child) {
                let direction = boom.offset.try_normalize().unwrap_or(Vec3::NEG_Z);
                let right = Vec3::Y.cross(direction).normalize_or_zero();
                let distance = framing.distance.max(MIN_BOOM_DISTANCE);
                let offset = if (boom.offset.length() - distance).abs() <= SETTLE_EPSILON {
                    boom.offset
                } else {
                    direction * distance
                };
                let shoulder = Vec3::Y * framing.height + right * framing.shoulder;
                let shoulder = if boom.shoulder.abs_diff_eq(shoulder, SETTLE_EPSILON) {
                    boom.shoulder
                } else {
                    shoulder
                };
                let framed = (*boom).clone().with_offset(offset).with_shoulder(shoulder);
                boom.set_if_neq(framed);
            }

            if let Some(mut camera) = cameras.get_entity(*child)
                && camera.lens.is_none_or(|lens| lens.fovy != framing.fovy)
            {
                let base = camera.lens.unwrap_or_default();
                camera.lens = Some(Lens {
                    fovy: framing.fovy,
                    ..base
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CameraStance, ThirdPersonCamera};

    #[test]
    fn a_new_stance_closes_half_of_every_fields_gap_per_halflife_however_the_time_is_split() {
        let mut rig = ThirdPersonCamera::default();
        rig.step(0.0);

        rig.stance = CameraStance::Aim;
        rig.step(rig.aim.halflife * 0.25);
        rig.step(rig.aim.halflife * 0.75);

        let from = rig.follow;
        let to = rig.aim;
        let current = rig.current();
        let halfway = |field: &str, actual: f32, from: f32, to: f32| {
            let expected = (from + to) * 0.5;
            assert!(
                (actual - expected).abs() < 1e-5,
                "{field}: expected {expected}, got {actual}"
            );
        };
        halfway("height", current.height, from.height, to.height);
        halfway("shoulder", current.shoulder, from.shoulder, to.shoulder);
        halfway("distance", current.distance, from.distance, to.distance);
        halfway("fovy", current.fovy, from.fovy, to.fovy);
        halfway(
            "sensitivity_scale",
            current.sensitivity_scale,
            from.sensitivity_scale,
            to.sensitivity_scale,
        );
        halfway(
            "pitch_limit",
            current.pitch_limit,
            from.pitch_limit,
            to.pitch_limit,
        );
    }
}

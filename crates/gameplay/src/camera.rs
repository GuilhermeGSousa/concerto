use concerto_ecs::{
    Component, Entity, Query, Res, Resource, Without,
    entity::hierarchy::ChildOf,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::{time::Time, transform::Transform};
use concerto_physics::{body::BodyId, physics_state::PhysicsState, query::QueryFilter};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_window::input::{Input, actions::ActionMap};
use glam::{EulerRot, Quat, Vec3};

use crate::actions::Look;
use crate::third_person::ThirdPersonCamera;

#[derive(Resource)]
pub struct CameraSettings {
    pub sensitivity: f32,
    pub pitch_limit: f32,
    /// Radians per second the camera turns with the `Look` stick at full deflection.
    pub stick_turn_rate: f32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            sensitivity: 0.001,
            pitch_limit: 30f32.to_radians(),
            stick_turn_rate: 3.0,
        }
    }
}

impl CameraSettings {
    pub fn with_stick_turn_rate(mut self, stick_turn_rate: f32) -> Self {
        self.stick_turn_rate = stick_turn_rate;
        self
    }
}

#[derive(Component, Default)]
pub struct CameraPivot {
    yaw: f32,
    pitch: f32,
}

impl CameraPivot {
    /// A level pivot turned `yaw` radians about +Y.
    pub fn facing(yaw: f32) -> Self {
        Self { yaw, pitch: 0.0 }
    }

    pub fn yaw(&self) -> f32 {
        self.yaw
    }
}

/// Turns each pivot by the mouse and the `Look` stick.
///
/// A pivot with a `ThirdPersonCamera` scales the sensitivity and takes its pitch
/// limit from the rig's framing.
pub fn move_camera_pivot(
    mut pivots: Query<(&mut Transform, &mut CameraPivot, Option<&ThirdPersonCamera>)>,
    input: Res<Input>,
    actions: Res<ActionMap>,
    time: Res<Time>,
    settings: Res<CameraSettings>,
) {
    let mouse_delta = input.mouse_delta();
    let stick_turn =
        actions.axis2d(Look, &input) * settings.stick_turn_rate * time.delta().as_secs_f32();

    for (mut transform, mut pivot, rig) in pivots.iter_mut() {
        let framing = rig.map(ThirdPersonCamera::current);
        let scale = framing.map_or(1.0, |framing| framing.sensitivity_scale);
        let sensitivity = settings.sensitivity * scale;
        let pitch_limit = framing.map_or(settings.pitch_limit, |framing| framing.pitch_limit);
        pivot.yaw -= mouse_delta.x * sensitivity + stick_turn.x * scale;
        pivot.pitch = (pivot.pitch - mouse_delta.y * sensitivity + stick_turn.y * scale)
            .clamp(-pitch_limit, pitch_limit);

        transform.rotation = Quat::from_rotation_y(pivot.yaw) * Quat::from_rotation_x(pivot.pitch);
    }
}

#[derive(Component)]
pub struct EntityFollow {
    pub target: Entity,
    pub offset: Vec3,
}

pub(crate) fn update_entity_follow(
    follow: Query<(Entity, &EntityFollow)>,
    mut entities: Query<&mut Transform>,
) {
    for (entity, to_follow) in follow.iter() {
        let Some(target) = entities
            .get_entity_mut(to_follow.target)
            .map(|target| target.translation)
        else {
            continue;
        };
        let Some(mut transform) = entities.get_entity_mut(entity) else {
            continue;
        };

        transform.translation = target + to_follow.offset;
    }
}

/// Holds a pivot's child camera at `offset` from its `shoulder`, each pulled in when a body is in the way.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct CameraBoom {
    pub offset: Vec3,
    /// Where the boom hangs from, as an offset from the pivot in its level frame.
    ///
    /// Being in the level frame, it stays put when the pivot pitches.
    pub shoulder: Vec3,
    pub probe_radius: f32,
    pub min_distance: f32,
    pub return_halflife: f32,
}

impl Default for CameraBoom {
    fn default() -> Self {
        Self {
            offset: Vec3::NEG_Z * 3.0,
            shoulder: Vec3::ZERO,
            probe_radius: 0.2,
            min_distance: 0.3,
            return_halflife: 0.15,
        }
    }
}

impl CameraBoom {
    pub fn with_offset(mut self, offset: Vec3) -> Self {
        self.offset = offset;
        self
    }

    pub fn with_shoulder(mut self, shoulder: Vec3) -> Self {
        self.shoulder = shoulder;
        self
    }

    pub fn with_probe_radius(mut self, probe_radius: f32) -> Self {
        self.probe_radius = probe_radius;
        self
    }

    pub fn with_min_distance(mut self, min_distance: f32) -> Self {
        self.min_distance = min_distance;
        self
    }

    pub fn with_return_halflife(mut self, return_halflife: f32) -> Self {
        self.return_halflife = return_halflife;
        self
    }
}

const BOOM_SKIN: f32 = 0.02;

/// The boom length after `dt` seconds: `allowed` at once when it is shorter than `current`, else eased toward it.
pub fn boom_distance(current: f32, allowed: f32, dt: f32, return_halflife: f32) -> f32 {
    if allowed <= current || return_halflife <= 0.0 {
        return allowed;
    }
    allowed + (current - allowed) * 0.5f32.powf(dt / return_halflife)
}

/// Pulls each boomed camera in toward its shoulder point when a body lies in the way, and eases it back out.
pub fn update_camera_booms(
    mut booms: Query<(&CameraBoom, &ChildOf, &mut Transform)>,
    pivots: Query<(&Transform, Option<&EntityFollow>), Without<CameraBoom>>,
    bodies: Query<&BodyId>,
    physics: Res<PhysicsState>,
    time: Res<Time>,
) {
    let dt = time.delta().as_secs_f32();

    for (boom, child_of, mut transform) in booms.iter_mut() {
        let Some((pivot, follow)) = pivots.get_entity(child_of.parent()) else {
            continue;
        };
        let length = boom.offset.length();
        if length <= 0.0 {
            continue;
        }
        let direction = boom.offset / length;

        let ignored = follow
            .and_then(|follow| bodies.get_entity(follow.target))
            .copied();
        let filter = QueryFilter::ignoring(ignored.as_slice());

        let shoulder_point = if boom.shoulder.length_squared() < 1e-8 {
            pivot.translation
        } else {
            let level = Quat::from_rotation_y(pivot.rotation.to_euler(EulerRot::YXZ).0);
            let shoulder_reach = level * boom.shoulder;
            let reach_length = shoulder_reach.length();
            match physics.cast_sphere(pivot.translation, boom.probe_radius, shoulder_reach, filter)
            {
                Some(hit) => {
                    pivot.translation
                        + shoulder_reach.normalize()
                            * (hit.fraction * reach_length - BOOM_SKIN).max(0.0)
                }
                None => pivot.translation + shoulder_reach,
            }
        };

        let reach = pivot.rotation * boom.offset;
        let allowed = physics
            .cast_sphere(shoulder_point, boom.probe_radius, reach, filter)
            .map_or(length, |hit| hit.fraction * length)
            .clamp(boom.min_distance.min(length), length);

        let shoulder_local = pivot.rotation.inverse() * (shoulder_point - pivot.translation);
        let current = (transform.translation - shoulder_local).dot(direction);
        let distance = boom_distance(current, allowed, dt, boom.return_halflife);
        transform.translation = shoulder_local + direction * distance;
    }
}

#[cfg(test)]
mod tests {
    use super::boom_distance;

    struct Ease {
        current: f32,
        allowed: f32,
        steps: &'static [f32],
        halflife: f32,
        distance: f32,
    }

    #[test]
    fn the_boom_pulls_in_at_once_and_eases_out_half_the_gap_per_halflife_however_the_time_is_split()
    {
        let eases = [
            Ease {
                current: 3.0,
                allowed: 1.0,
                steps: &[0.01],
                halflife: 0.15,
                distance: 1.0,
            },
            Ease {
                current: 1.0,
                allowed: 3.0,
                steps: &[0.15],
                halflife: 0.15,
                distance: 2.0,
            },
            Ease {
                current: 1.0,
                allowed: 3.0,
                steps: &[0.05, 0.1],
                halflife: 0.15,
                distance: 2.0,
            },
            Ease {
                current: 1.0,
                allowed: 3.0,
                steps: &[0.01],
                halflife: 0.0,
                distance: 3.0,
            },
        ];
        for ease in eases {
            let distance = ease.steps.iter().fold(ease.current, |current, dt| {
                boom_distance(current, ease.allowed, *dt, ease.halflife)
            });
            assert!(
                (distance - ease.distance).abs() < 1e-5,
                "from {} toward {} over {:?}: {distance}",
                ease.current,
                ease.allowed,
                ease.steps
            );
        }
    }
}

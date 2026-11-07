use concerto_ecs::{
    Component, Entity, Query, Res, Resource, Without,
    entity::hierarchy::ChildOf,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::{time::Time, transform::Transform};
use concerto_physics::{body::BodyId, physics_state::PhysicsState, query::QueryFilter};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_window::input::Input;
use glam::{EulerRot, Quat, Vec3};

#[derive(Resource)]
pub struct CameraSettings {
    pub sensitivity: f32,
    pub pitch_limit: f32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            sensitivity: 0.001,
            pitch_limit: 30f32.to_radians(),
        }
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

pub fn move_camera_pivot(
    pivots: Query<(&mut Transform, &mut CameraPivot)>,
    input: Res<Input>,
    settings: Res<CameraSettings>,
) {
    let mouse_delta = input.mouse_delta();

    for (mut transform, mut pivot) in pivots.iter() {
        pivot.yaw -= mouse_delta.x * settings.sensitivity;
        pivot.pitch = (pivot.pitch - mouse_delta.y * settings.sensitivity)
            .clamp(-settings.pitch_limit, settings.pitch_limit);

        transform.rotation = Quat::from_rotation_y(pivot.yaw) * Quat::from_rotation_x(pivot.pitch);
    }
}

#[derive(Component)]
pub struct EntityFollow {
    pub target: Entity,
    pub offset: Vec3,
}

pub(crate) fn update_entity_follow(
    follow: Query<(&EntityFollow, &mut Transform)>,
    entities: Query<&mut Transform>,
) {
    for (to_follow, mut transform) in follow.iter() {
        let Some(target) = entities.get_entity(to_follow.target) else {
            continue;
        };

        transform.translation = target.translation + to_follow.offset;
    }
}

/// Holds a pivot's child camera at `offset` from its `shoulder`, each pulled in when a body is in the way.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct CameraBoom {
    pub offset: Vec3,
    /// Where the boom hangs from: an offset from the pivot in its level frame, so pitching the pivot doesn't move it.
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
    booms: Query<(&CameraBoom, &ChildOf, &mut Transform)>,
    pivots: Query<(&Transform, Option<&EntityFollow>), Without<CameraBoom>>,
    bodies: Query<&BodyId>,
    physics: Res<PhysicsState>,
    time: Res<Time>,
) {
    let dt = time.delta().as_secs_f32();

    for (boom, child_of, mut transform) in booms.iter() {
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
        let filter = QueryFilter {
            ignore: ignored.as_slice(),
        };

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

    #[test]
    fn a_shorter_allowed_distance_is_taken_at_once() {
        assert_eq!(boom_distance(3.0, 1.0, 0.01, 0.15), 1.0);
    }

    #[test]
    fn a_longer_allowed_distance_closes_half_the_gap_in_one_halflife() {
        let distance = boom_distance(1.0, 3.0, 0.15, 0.15);
        assert!((distance - 2.0).abs() < 1e-5, "got {distance}");
    }

    #[test]
    fn two_half_steps_land_where_one_full_step_does() {
        let full = boom_distance(1.0, 3.0, 0.1, 0.15);
        let halves = boom_distance(boom_distance(1.0, 3.0, 0.05, 0.15), 3.0, 0.05, 0.15);
        assert!((full - halves).abs() < 1e-5, "{full} vs {halves}");
    }

    #[test]
    fn a_zero_halflife_snaps_out() {
        assert_eq!(boom_distance(1.0, 3.0, 0.01, 0.0), 3.0);
    }
}

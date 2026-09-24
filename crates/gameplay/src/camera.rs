use concerto_ecs::{Component, Entity, Query, Res, Resource};
use concerto_foundation::transform::Transform;
use concerto_window::input::Input;
use glam::{Quat, Vec3};

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

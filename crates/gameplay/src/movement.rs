use concerto_director::VirtualCamera;
use concerto_ecs::{Query, Res, With, entity::hierarchy::Children};
use concerto_foundation::{time::Time, transform::Transform};
use concerto_window::input::{Input, InputState, KeyCode, PhysicalKey};
use glam::{Quat, Vec3};

use crate::player::Player;

pub fn first_person_player_fly(
    players: Query<(&mut Transform, &Children), With<Player>>,
    cameras: Query<&mut Transform, With<VirtualCamera>>,
    input: Res<Input>,
    time: Res<Time>,
) {
    let Some((mut player_transform, children)) = players.iter().next() else {
        return;
    };

    // The world also has a standalone fallback `VirtualCamera` (the director's
    // never-empty stack bottom), so we can't just take the first camera the
    // query finds — it has to be the player's own child camera specifically.
    let Some(mut camera_transform) = children
        .into_iter()
        .find_map(|child| cameras.get_entity(*child))
    else {
        return;
    };

    let displacement = 10.0 * time.delta().as_secs_f32();

    let key_d = input.get_key_state(PhysicalKey::Code(KeyCode::KeyD));
    let key_a = input.get_key_state(PhysicalKey::Code(KeyCode::KeyA));
    let key_w = input.get_key_state(PhysicalKey::Code(KeyCode::KeyW));
    let key_s = input.get_key_state(PhysicalKey::Code(KeyCode::KeyS));

    // Camera transform is local to the player, so combine rotations to get world-space directions.
    let camera_world_rotation = player_transform.rotation * camera_transform.rotation;
    let forward = camera_world_rotation * Vec3::NEG_Z;
    let back = -forward;
    let right = camera_world_rotation * Vec3::X;
    let left = -right;

    if key_d == InputState::Pressed || key_d == InputState::Down {
        player_transform.translation += right * displacement;
    }
    if key_a == InputState::Pressed || key_a == InputState::Down {
        player_transform.translation += left * displacement;
    }
    if key_w == InputState::Pressed || key_w == InputState::Down {
        player_transform.translation += forward * displacement;
    }
    if key_s == InputState::Pressed || key_s == InputState::Down {
        player_transform.translation += back * displacement;
    }

    let sensitivity = -0.003;
    let mouse_delta = input.mouse_delta();
    let yaw_delta = sensitivity * mouse_delta.x;
    let pitch_delta = sensitivity * mouse_delta.y;
    player_transform.rotation *= Quat::from_axis_angle(Vec3::Y, yaw_delta);
    camera_transform.rotation *= Quat::from_axis_angle(Vec3::X, pitch_delta);
}

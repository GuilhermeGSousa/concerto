//! Diagnostics for automated runs in headless browsers.
use concerto::{
    director::MainCamera,
    ecs::{Query, Res, ResMut, Resource, With},
    foundation::transform::{GlobalTransform, Transform},
};

use crate::{game::Game, mannequin::Mannequin, platform, player::Player};

#[derive(Resource, Default)]
pub struct TraceFrame(u64);

pub fn trace(
    cameras: Query<&GlobalTransform, With<MainCamera>>,
    players: Query<(&Transform, &crate::body::Mover), With<Player>>,
    input: Res<concerto::window::input::Input>,
    mannequins: Query<(&Mannequin, &Transform)>,
    game: Res<Game>,
    mut frame: ResMut<TraceFrame>,
) {
    frame.0 += 1;
    if !platform::debug_flag("trace") {
        return;
    }
    let delta = input.mouse_delta();
    if delta != glam::Vec2::ZERO {
        log::info!("trace: mouse delta {delta:?}");
    }
    if !frame.0.is_multiple_of(12) {
        return;
    }
    let camera = cameras
        .iter()
        .next()
        .map(|g| (g.translation(), g.rotation()));
    let player = players
        .iter()
        .next()
        .map(|(t, m)| (t.translation, m.desired, m.velocity()));
    log::info!(
        "trace: phase={:?} paused={} night={} t={:.1} camera={:?} player={:?}",
        game.phase,
        game.paused,
        game.night,
        game.night_time,
        camera,
        player
    );
    for (mannequin, transform) in mannequins.iter() {
        if mannequin.is_hunting() {
            log::info!(
                "trace: hunter at {:?} observed={}",
                transform.translation,
                mannequin.observed
            );
        }
    }
}

/// `?poses`: lays every pose out in rows before the camera, numbered left to
/// right, front row first.
pub fn pose_gallery(
    cmd: &mut concerto::ecs::CommandQueue,
    server: &concerto::foundation::assets::asset_server::AssetServer,
    palette: &crate::palette::Palette,
    library: &crate::poses::PoseLibrary,
) -> bool {
    use concerto::{physics::collider::Collider, render::components::light::Light};
    use glam::Vec3;

    let row_filter = (0..3).find(|r| platform::debug_flag(&format!("poses{r}")));
    if !platform::debug_flag("poses") && row_filter.is_none() {
        return false;
    }
    cmd.spawn((
        crate::house::HouseEntity,
        Collider::cuboid(40.0, 0.5, 40.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    cmd.spawn((
        crate::house::HouseEntity,
        Light::directional_light().with_intensity(3.0),
        Transform::from_rotation(glam::Quat::from_rotation_x(-0.6)),
    ));
    crate::player::spawn_player(cmd, Vec3::new(0.0, 0.05, 0.0), 0.0);
    let per_row = 9;
    for (i, _) in library.poses.iter().enumerate() {
        if row_filter.is_some_and(|r| i / per_row != r) {
            continue;
        }
        let row = if row_filter.is_some() {
            0.0
        } else {
            (i / per_row) as f32
        };
        let col = (i % per_row) as f32 - (per_row as f32 - 1.0) * 0.5;
        let feet = Vec3::new(col * 1.4, 0.0, -5.0 - row * 3.2);
        crate::mannequin::spawn_mannequin(
            cmd,
            server,
            feet,
            0.0,
            crate::mannequin::Kind::Inert,
            if row_filter.is_some() {
                0
            } else {
                (i / per_row) % palette.finishes.len()
            },
            i,
        );
    }
    true
}

/// `?room=N`: starts the night in the corner of room `N`, looking across it.
pub fn room_view(level: &crate::level::Level) -> Option<(glam::Vec3, f32)> {
    let index: usize = platform::debug_value("room")?.parse().ok()?;
    let room = level.rooms.get(index)?;
    let corner = crate::level::Cell::new(room.x, room.y).center();
    let far = crate::level::Cell::new(room.x + room.w - 1, room.y + room.h - 1).center();
    let from = corner - glam::Vec3::new(1.2, 0.0, 1.2);
    let to = far + glam::Vec3::new(1.0, 0.0, 1.0);
    let dir = to - from;
    log::warn!("room {index}: {:?} {}x{}", room.kind, room.w, room.h);
    Some((from + glam::Vec3::Y * 0.05, (-dir.x).atan2(-dir.z)))
}

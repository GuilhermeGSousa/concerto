//! Diagnostics for automated runs in headless browsers. Enabled by URL query
//! flags on the web (`?nolock&noui&trace`) or `AFTER_HOURS_DEBUG=nolock,trace`
//! natively:
//! - `nolock`: treat the pointer as locked, so play never pauses.
//! - `noui`: hide every screen, for clean screenshots.
//! - `trace`: log the camera, player and mannequin state once a second.
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

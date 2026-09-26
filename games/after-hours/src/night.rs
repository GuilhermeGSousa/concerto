//! Nights: building the floor, the objectives, the clock that wakes the
//! decoys, failing lights, and the flow between screens.
use concerto::{
    audio::Audio,
    color::Color,
    ecs::{
        CommandQueue, Entity, Query, Res, ResMut, Resource, With, Without,
        entity::hierarchy::ChildOf,
    },
    foundation::{assets::asset_server::AssetServer, time::Time, transform::Transform},
    render::{MaterialComponent, components::light::Light},
    window::input::{Input, KeyCode, MouseButton, PhysicalKey},
};
use glam::{Quat, Vec3};

use crate::{
    body::{flat, yaw_toward},
    game::{Game, Phase, Rand},
    level::{self, CELL, Dir, LevelSpec},
    mannequin::{self, CurrentLevel, Flow, Kind, Mannequin},
    palette::PaletteSlot,
    platform,
    player::{self, Eye, Flashlight, Player},
    poses::{Menace, PoseLibrary},
    scare::{CameraOverride, Caught},
    sfx::{Sfx, Sounds},
    store::{self, BatteryPickup, CeilingLight, ExitDoor, KeyPickup, StoreEntity},
};

/// Seconds between decoys waking up.
const WAKE_INTERVAL: f32 = 60.0;
/// Minutes past 11 PM the night starts at.
const START_MINUTES: f32 = 52.0;
/// In-game minutes per real second on the floor.
const CLOCK_RATE: f32 = 0.5;

pub struct NightPlan {
    pub spec: LevelSpec,
    pub hunter_speed: f32,
    /// Seconds before the first hunter wakes, and between later ones.
    pub first_wake: f32,
    pub wake_spacing: f32,
}

/// The difficulty curve (see DESIGN.md).
pub fn plan(night: u32, seed: u64) -> NightPlan {
    let n = night.max(1);
    #[rustfmt::skip]
    let table: [(i32, usize, usize, usize, f32, f32, f32, f32); 5] = [
        // size keys hunters decoys lit  speed first_wake spacing
        (7,  3, 2, 6,  0.45, 3.5, 25.0, 30.0),
        (8,  3, 3, 7,  0.35, 4.0, 18.0, 22.0),
        (8,  4, 4, 8,  0.3,  4.5, 12.0, 18.0),
        (9,  4, 5, 9,  0.25, 5.0, 8.0, 14.0),
        (10, 5, 6, 10, 0.2,  5.5, 5.0, 10.0),
    ];
    let (size, keys, hunters, decoys, lit, speed, first_wake, wake_spacing) =
        if n as usize <= table.len() {
            table[n as usize - 1]
        } else {
            let extra = (n as usize - table.len()).min(6);
            (10, 5, 6 + extra, 10, 0.15, 6.0, 3.0, 8.0)
        };
    NightPlan {
        spec: LevelSpec {
            size,
            keys,
            batteries: keys,
            hunters,
            decoys,
            lit_fraction: lit,
            max_lights: 12,
            seed,
        },
        hunter_speed: speed,
        first_wake,
        wake_spacing,
    }
}

#[derive(Resource, Default)]
pub struct NightState {
    pub exit: Option<Entity>,
    pub keys_total: usize,
    pub unlocked: bool,
    wake_timer: f32,
    heartbeat_timer: f32,
    chime_timer: f32,
    /// A line of text for the HUD, and how long it has left.
    pub message: Option<(String, f32)>,
    ambience_on: bool,
    /// Set when the floor must be (re)built before the next frame.
    pub rebuild: Option<Rebuild>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rebuild {
    /// The title backdrop: the night-1 floor with every mannequin asleep.
    Title,
    /// The current night, for real.
    Night,
}

impl NightState {
    pub fn with_rebuild(rebuild: Rebuild) -> Self {
        Self {
            rebuild: Some(rebuild),
            ..Default::default()
        }
    }

    pub fn say(&mut self, text: impl Into<String>, seconds: f32) {
        self.message = Some((text.into(), seconds));
    }
}

/// The clock on the HUD, e.g. "12:07 AM".
pub fn clock_text(night_time: f32) -> String {
    let minutes = START_MINUTES + night_time * CLOCK_RATE;
    let total = (23.0 * 60.0 + minutes) as u32;
    let hour24 = (total / 60) % 24;
    let minute = total % 60;
    let (hour, suffix) = match hour24 {
        0 => (12, "AM"),
        1..=11 => (hour24, "AM"),
        12 => (12, "PM"),
        _ => (hour24 - 12, "PM"),
    };
    format!("{hour}:{minute:02} {suffix}")
}

#[allow(clippy::too_many_arguments)]
pub fn rebuild_night(
    mut state: ResMut<NightState>,
    mut game: ResMut<Game>,
    stale: Query<Entity, (With<StoreEntity>, Without<ChildOf>)>,
    server: Res<AssetServer>,
    palette: Res<PaletteSlot>,
    library: Res<PoseLibrary>,
    mut current: ResMut<CurrentLevel>,
    (mut flow, mut eye, mut caught, mut camera): (
        ResMut<Flow>,
        ResMut<Eye>,
        ResMut<Caught>,
        ResMut<CameraOverride>,
    ),
    mut rand: ResMut<Rand>,
    mut cmd: CommandQueue,
) {
    let Some(kind) = state.rebuild else {
        return;
    };
    let Some(palette) = palette.get() else {
        return;
    };
    if library.poses.is_empty() {
        return;
    }
    state.rebuild = None;
    for entity in stale.iter() {
        cmd.despawn(entity);
    }

    let night = if kind == Rebuild::Title {
        1
    } else {
        game.night
    };
    let seed = if kind == Rebuild::Title {
        0x0071_71e5
    } else {
        game.seed
    };
    let plan = plan(night, seed);
    let level = level::generate(&plan.spec);
    if kind == Rebuild::Title && crate::debug::pose_gallery(&mut cmd, &server, palette, &library) {
        state.exit = None;
        current.0 = Some(level);
        return;
    }
    let built = store::build_store(&mut cmd, &server, palette, &level, seed);

    // Face into the store from the start cell.
    let start = level.start;
    let toward = level
        .neighbors(start)
        .next()
        .map(|n| n.center() - start.center())
        .unwrap_or(Vec3::Z);
    let yaw = (-toward.x).atan2(-toward.z);
    player::spawn_player(&mut cmd, start.center() + Vec3::new(-0.8, 0.05, -0.8), yaw);

    let spawn = |cells: &[level::Cell], kind: Kind, cmd: &mut CommandQueue, rand: &mut Rand| {
        for (i, &cell) in cells.iter().enumerate() {
            let jitter = Vec3::new(rand.range(-1.0, 1.0), 0.0, rand.range(-1.0, 1.0));
            let feet = cell.center() + jitter;
            // On display: facing the aisle, not you. Yet.
            let face = Dir::ALL[rand.index(4)].vector();
            let pose = library.pick(Menace::Display, None, rand).unwrap_or(0);
            mannequin::spawn_mannequin(
                cmd,
                &server,
                feet,
                yaw_toward(face),
                kind,
                plan.hunter_speed,
                rand.index(palette.finishes.len()),
                pose,
                // Hunters wake one by one; the first gives you a head start.
                plan.first_wake + i as f32 * plan.wake_spacing,
            );
        }
    };
    let hunters = if kind == Rebuild::Title {
        Kind::Decoy
    } else {
        Kind::Hunter
    };
    spawn(&level.hunters, hunters, &mut cmd, &mut rand);
    spawn(&level.decoys, Kind::Decoy, &mut cmd, &mut rand);

    state.exit = Some(built.exit);
    state.keys_total = level.keys.len();
    state.unlocked = false;
    state.wake_timer = WAKE_INTERVAL;
    state.heartbeat_timer = 0.0;
    state.chime_timer = 4.0;
    state.message = None;
    current.0 = Some(level);
    *flow = Flow::default();
    eye.valid = false;
    caught.reset();
    camera.clear();
    game.night_time = 0.0;
    game.generation += 1;
}

fn clicked(input: &Input) -> bool {
    input.is_mouse_button_just_pressed(MouseButton::Left)
        || input.is_just_pressed(PhysicalKey::Code(KeyCode::Space))
        || input.is_just_pressed(PhysicalKey::Code(KeyCode::Enter))
}

/// Screen-to-screen flow and pausing.
#[allow(clippy::too_many_arguments)]
pub fn advance_phases(
    mut game: ResMut<Game>,
    mut state: ResMut<NightState>,
    input: Res<Input>,
    time: Res<Time>,
    mut audio: ResMut<Audio>,
    mut sounds: ResMut<Sounds>,
    mut rand: ResMut<Rand>,
) {
    let real_dt = time.real_delta().as_secs_f32();
    game.phase_time += real_dt;
    if let Some((_, left)) = state.message.as_mut() {
        *left -= real_dt;
        if *left <= 0.0 {
            state.message = None;
        }
    }

    match game.phase {
        Phase::Title => {
            if game.phase_time > 0.5 && clicked(&input) {
                sounds.play(&mut audio, Sfx::Click, 0.6);
                game.night = 1;
                game.seed = rand.next_u32() as u64 ^ 0xa11_0005;
                start_night(&mut game, &mut state, &mut audio, &mut sounds);
            }
        }
        Phase::Intro => {
            if game.phase_time > 3.5 || (game.phase_time > 0.8 && clicked(&input)) {
                game.set_phase(Phase::Playing);
                let keys = state.keys_total;
                state.say(format!("Find the {keys} register keys. Then get out."), 5.0);
            }
        }
        Phase::Playing => {
            let locked = platform::pointer_locked();
            if game.paused {
                if locked || (!platform::IS_WEB && clicked(&input)) {
                    game.paused = false;
                }
            } else if (!locked && game.phase_time > 0.6)
                || (!platform::IS_WEB && input.is_just_pressed(PhysicalKey::Code(KeyCode::Escape)))
            {
                // On the web, Escape releases the pointer lock itself.
                game.paused = true;
            }
        }
        Phase::Caught => {}
        Phase::Dead => {
            if game.phase_time > 1.2 && clicked(&input) {
                sounds.play(&mut audio, Sfx::Click, 0.6);
                // Same seed: the same floor, now that you know it.
                start_night(&mut game, &mut state, &mut audio, &mut sounds);
            }
        }
        Phase::Escaped => {
            if game.phase_time > 1.5 && clicked(&input) {
                sounds.play(&mut audio, Sfx::Click, 0.6);
                game.night += 1;
                game.seed = (rand.next_u32() as u64) ^ ((game.night as u64) << 40);
                start_night(&mut game, &mut state, &mut audio, &mut sounds);
            }
        }
    }

    let ambience_wanted = matches!(game.phase, Phase::Title | Phase::Intro | Phase::Playing);
    if ambience_wanted && !state.ambience_on {
        if let Some(ambience) = sounds.ambience {
            audio.play_music(ambience, 1.0);
        }
        state.ambience_on = true;
    } else if !ambience_wanted && state.ambience_on {
        audio.stop_music();
        state.ambience_on = false;
    }
}

fn start_night(game: &mut Game, state: &mut NightState, audio: &mut Audio, sounds: &mut Sounds) {
    game.paused = false;
    game.set_phase(Phase::Intro);
    state.rebuild = Some(Rebuild::Night);
    sounds.play(audio, Sfx::NightStart, 0.7);
}

/// Keys, batteries and the exit.
#[allow(clippy::too_many_arguments)]
pub fn objectives(
    players: Query<(&mut Player, &Transform), Without<KeyPickup>>,
    keys: Query<(Entity, &KeyPickup, &mut Transform), Without<Player>>,
    batteries: Query<
        (Entity, &BatteryPickup, &mut Transform),
        (Without<Player>, Without<KeyPickup>),
    >,
    flashlights: Query<&mut Flashlight>,
    exits: Query<(&ExitDoor, &mut Light)>,
    ceiling: Query<&mut CeilingLight>,
    palette: Res<PaletteSlot>,
    mut state: ResMut<NightState>,
    mut game: ResMut<Game>,
    (time, mut rand): (Res<Time>, ResMut<Rand>),
    (mut audio, mut sounds): (ResMut<Audio>, ResMut<Sounds>),
    mut cmd: CommandQueue,
) {
    let t = game.night_time;
    // Pickups turn and bob so they catch the light.
    for (_, key, mut transform) in keys.iter() {
        transform.translation = key.base + Vec3::Y * (t * 2.0).sin() * 0.06;
        transform.rotation = Quat::from_rotation_y(t * 1.5);
    }
    for (_, battery, mut transform) in batteries.iter() {
        transform.translation = battery.base + Vec3::Y * (t * 2.3).sin() * 0.05;
        transform.rotation = Quat::from_rotation_z(0.5) * Quat::from_rotation_y(t * 1.2);
    }
    if !game.is_live() {
        return;
    }
    game.night_time += time.delta().as_secs_f32();
    let Some(palette) = palette.get() else {
        return;
    };
    let Some((mut player, body)) = players.iter().next() else {
        return;
    };
    let chest = body.translation + Vec3::Y;

    for (entity, key, _) in keys.iter() {
        if key.base.distance(chest) < 1.2 {
            cmd.despawn(entity);
            player.keys += 1;
            sounds.play(&mut audio, Sfx::KeyPickup, 0.8);
            let left = state.keys_total.saturating_sub(player.keys);
            if left == 0 {
                state.unlocked = true;
                sounds.play(&mut audio, Sfx::Unlock, 0.9);
                state.say("That's all of them. The staff exit is unlocked.", 5.0);
                for (door, mut light) in exits.iter() {
                    cmd.insert(
                        MaterialComponent {
                            handle: palette.sign_unlocked.clone(),
                        },
                        door.sign,
                    );
                    light.color = Color::srgba(0.2, 1.0, 0.35, 1.0);
                    light.intensity = 4.0;
                }
            } else {
                state.say(format!("{left} to go."), 3.0);
            }
            // The store notices: a working light somewhere dies for good.
            let working = ceiling.iter().filter(|l| l.on && !l.flickers).count();
            if working > 2 {
                let mut n = rand.index(working);
                for mut light in ceiling.iter() {
                    if light.on && !light.flickers {
                        if n == 0 {
                            light.flickers = true;
                            light.dying = true;
                            break;
                        }
                        n -= 1;
                    }
                }
            }
        }
    }

    for (entity, battery, _) in batteries.iter() {
        if battery.base.distance(chest) < 1.2 {
            cmd.despawn(entity);
            sounds.play(&mut audio, Sfx::BatteryPickup, 0.7);
            for mut flashlight in flashlights.iter() {
                flashlight.battery = (flashlight.battery + 0.5).min(1.0);
                flashlight.switched_on = true;
            }
            state.say("Batteries.", 2.0);
        }
    }

    if state.unlocked {
        for (door, _) in exits.iter() {
            if flat(door.threshold - body.translation).length() < 1.1 {
                sounds.play(&mut audio, Sfx::Escape, 0.9);
                game.set_phase(Phase::Escaped);
                if game.night >= game.best_night {
                    game.best_night = game.night + 1;
                    platform::save_best(game.best_night);
                }
            }
        }
    }
}

/// Wakes decoys on a timer and drives the heartbeat.
#[allow(clippy::too_many_arguments)]
pub fn night_clock(
    mannequins: Query<(&mut Mannequin, &Transform), Without<Player>>,
    players: Query<&Transform, With<Player>>,
    keys: Query<&KeyPickup>,
    exits: Query<&ExitDoor>,
    eye: Res<Eye>,
    mut state: ResMut<NightState>,
    game: Res<Game>,
    time: Res<Time>,
    mut rand: ResMut<Rand>,
    mut audio: ResMut<Audio>,
    mut sounds: ResMut<Sounds>,
) {
    if !game.is_live() {
        return;
    }
    let dt = time.delta().as_secs_f32();
    state.wake_timer -= dt;
    if state.wake_timer <= 0.0 {
        state.wake_timer = WAKE_INTERVAL;
        let sleeping = mannequins.iter().filter(|(m, _)| !m.is_hunting()).count();
        if sleeping > 0 {
            let mut n = rand.index(sleeping);
            for (mut m, _) in mannequins.iter() {
                if !m.is_hunting() {
                    if n == 0 {
                        m.wake();
                        break;
                    }
                    n -= 1;
                }
            }
        }
    }

    let Some(player) = players.iter().next() else {
        return;
    };

    // Guidance by ear: the nearest key (or the open exit) chimes now and then.
    state.chime_timer -= dt;
    if state.chime_timer <= 0.0 {
        state.chime_timer = rand.range(5.0, 8.0);
        let source = if state.unlocked {
            exits
                .iter()
                .next()
                .map(|door| door.threshold + Vec3::Y * 1.5)
        } else {
            keys.iter().map(|key| key.base).min_by(|a, b| {
                a.distance_squared(player.translation)
                    .total_cmp(&b.distance_squared(player.translation))
            })
        };
        if let Some(point) = source {
            let (volume, pan) = mannequin::spatialize(&eye, point);
            // Never quite silent, so a far key can still be found.
            sounds.play_at(&mut audio, Sfx::Chime, (volume * 0.5).max(0.07), pan);
        }
    }

    let nearest = mannequins
        .iter()
        .filter(|(m, _)| m.is_hunting())
        .map(|(_, t)| flat(t.translation - player.translation).length())
        .fold(f32::MAX, f32::min);
    let range = CELL * 3.0;
    if nearest < range {
        let closeness = 1.0 - nearest / range;
        state.heartbeat_timer -= dt;
        if state.heartbeat_timer <= 0.0 {
            state.heartbeat_timer = 1.15 - 0.7 * closeness;
            sounds.play(&mut audio, Sfx::Heartbeat, 0.25 + 0.6 * closeness);
        }
    } else {
        state.heartbeat_timer = 0.0;
    }
}

/// Flickering tubes stutter; dying ones stutter harder and then go out.
pub fn flicker_lights(
    lights: Query<(&mut CeilingLight, &mut Light, &Transform)>,
    palette: Res<PaletteSlot>,
    game: Res<Game>,
    eye: Res<Eye>,
    time: Res<Time>,
    mut rand: ResMut<Rand>,
    mut audio: ResMut<Audio>,
    mut sounds: ResMut<Sounds>,
    mut cmd: CommandQueue,
) {
    let Some(palette) = palette.get() else {
        return;
    };
    let dt = time.delta().as_secs_f32();
    for (mut ceiling, mut light, transform) in lights.iter() {
        if !ceiling.flickers || ceiling.dead {
            continue;
        }
        ceiling.timer -= dt;
        if ceiling.timer > 0.0 {
            continue;
        }
        let was_on = ceiling.on;
        if ceiling.on {
            ceiling.on = false;
            ceiling.timer = rand.range(0.04, if ceiling.dying { 0.5 } else { 0.18 });
        } else {
            ceiling.on = true;
            ceiling.timer = if ceiling.dying {
                rand.range(0.05, 0.6)
            } else {
                rand.range(0.8, 7.0)
            };
            if ceiling.dying {
                ceiling.death_flickers += 1;
                if ceiling.death_flickers > 6 {
                    ceiling.on = false;
                    ceiling.dead = true;
                }
            }
        }
        light.intensity = if ceiling.on {
            ceiling.base_intensity
        } else {
            0.0
        };
        if was_on != ceiling.on {
            cmd.insert(
                MaterialComponent {
                    handle: if ceiling.on {
                        palette.panel_on.clone()
                    } else {
                        palette.panel_off.clone()
                    },
                },
                ceiling.panel,
            );
            if !ceiling.on && game.is_live() {
                let (volume, pan) = mannequin::spatialize(&eye, transform.translation);
                sounds.play_at(&mut audio, Sfx::Flicker, volume * 0.5, pan);
            }
        }
    }
}

/// On the title screen the view idles in the entrance, looking slowly around.
pub fn title_drift(players: Query<&mut Player>, game: Res<Game>) {
    if game.phase != Phase::Title {
        return;
    }
    let t = game.phase_time;
    for mut player in players.iter() {
        player.yaw += (t * 0.11).sin() * 0.0025;
        player.pitch = -0.08 + (t * 0.23).sin() * 0.03;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_rolls_over_midnight() {
        assert_eq!(clock_text(0.0), "11:52 PM");
        assert_eq!(clock_text(16.0), "12:00 AM");
        assert_eq!(clock_text(16.0 + 120.0 * 2.0), "2:00 AM");
    }

    #[test]
    fn nights_get_harder() {
        let mut last = plan(1, 0);
        for night in 2..10 {
            let next = plan(night, 0);
            assert!(next.spec.hunters >= last.spec.hunters);
            assert!(next.hunter_speed >= last.hunter_speed);
            assert!(next.spec.lit_fraction <= last.spec.lit_fraction);
            last = next;
        }
    }
}

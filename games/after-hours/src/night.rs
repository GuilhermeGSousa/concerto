//! Nights: building the house, the pacing, the objectives, the candles, and
//! the flow between screens.
use concerto::{
    audio::Audio,
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
    house::{self, Candle, HouseEntity, Ledger, Lot, OilPickup, PagePickup},
    level::{self, CELL, Dir, LevelSpec, RoomKind},
    mannequin::{self, CurrentLevel, Director, Flow, Kind, Mannequin, Rules},
    palette::{CLARA_FINISH, COMMON_FINISHES, PaletteSlot},
    platform,
    player::{self, Eye, Flashlight, Player},
    poses::{Menace, PoseLibrary},
    scare::{CameraOverride, Caught},
    sfx::{Sfx, Sounds},
    story,
};

const START_MINUTES: f32 = 52.0;
const CLOCK_RATE: f32 = 0.5;
const CATALOGUE_SECONDS: f32 = 1.6;
const SIGN_SECONDS: f32 = 1.2;
const REACH: f32 = 2.3;

pub struct NightPlan {
    pub spec: LevelSpec,
    pub rules: Rules,
    pub possessed: usize,
    /// Clara walks tonight.
    pub clara: bool,
    /// Seconds between candles guttering out on their own (0: never).
    pub draught: f32,
}

/// The difficulty curve (see DESIGN.md).
pub fn plan(night: u32, seed: u64) -> NightPlan {
    let n = night.max(1);
    #[rustfmt::skip]
    let table: [(i32, usize, usize, usize, f32); 5] = [
        (5, 3, 6,  0, 0.36),
        (6, 3, 7,  1, 0.3),
        (6, 4, 8,  2, 0.27),
        (7, 4, 9,  3, 0.24),
        (7, 5, 10, 4, 0.22),
    ];
    let (size, lots, figures, possessed, lit) = if n as usize <= table.len() {
        table[n as usize - 1]
    } else {
        let extra = (n as usize - table.len()).min(4);
        (7, 5, 10, 4 + extra, 0.2)
    };
    let rules = match n {
        1 => Rules {
            doubt_after_lots: Some(2),
            hunt_after_lots: usize::MAX,
            ..Rules::default()
        },
        2 => Rules {
            max_stalkers: 1,
            max_hunters: 1,
            stalk_after: 45.0,
            hunt_after_lots: 2,
            hunt_gap: (50.0, 80.0),
            hunt_speed: 3.0,
            stalk_speed: 2.4,
            hunt_duration: 10.0,
            doubt_after_lots: None,
        },
        3 => Rules {
            max_stalkers: 1,
            max_hunters: 1,
            stalk_after: 30.0,
            hunt_after_lots: 1,
            hunt_gap: (35.0, 60.0),
            hunt_speed: 3.3,
            stalk_speed: 2.6,
            hunt_duration: 12.0,
            doubt_after_lots: None,
        },
        4 => Rules {
            max_stalkers: 2,
            max_hunters: 1,
            stalk_after: 20.0,
            hunt_after_lots: 1,
            hunt_gap: (25.0, 45.0),
            hunt_speed: 3.7,
            stalk_speed: 2.8,
            hunt_duration: 14.0,
            doubt_after_lots: None,
        },
        _ => Rules {
            max_stalkers: 2,
            max_hunters: 2,
            stalk_after: 15.0,
            hunt_after_lots: if n == 5 { 1 } else { 0 },
            hunt_gap: if n == 5 { (20.0, 35.0) } else { (15.0, 30.0) },
            hunt_speed: if n == 5 { 4.0 } else { 4.3 },
            stalk_speed: 3.0,
            hunt_duration: 15.0,
            doubt_after_lots: None,
        },
    };
    NightPlan {
        spec: LevelSpec {
            size,
            lots,
            oil: lots.min(4),
            figures,
            lit_fraction: lit,
            max_lights: 12,
            loop_doors: 0.35,
            bedroom: n >= 3,
            seed,
        },
        rules,
        possessed,
        clara: n >= 5,
        draught: if n >= 3 { 75.0 } else { 0.0 },
    }
}

#[derive(Resource, Default)]
pub struct NightState {
    pub ledger: Option<Entity>,
    pub lots_total: usize,
    pub lots_done: usize,
    pub unlocked: bool,
    heartbeat_timer: f32,
    chime_timer: f32,
    draught_timer: f32,
    draught: f32,
    /// A line of text for the HUD, and how long it has left.
    pub message: Option<(String, f32)>,
    /// A diary page on screen.
    pub reading: Option<&'static str>,
    /// What E would do right now, for the HUD.
    pub prompt: Option<String>,
    /// Progress of a held E, 0..=1.
    pub hold: f32,
    /// The room you are in, shown briefly on entering.
    pub room: Option<(&'static str, f32)>,
    last_room: Option<usize>,
    told: Vec<&'static str>,
    ambience_on: bool,
    /// Set when the house must be (re)built before the next frame.
    pub rebuild: Option<Rebuild>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rebuild {
    /// The title backdrop: the first night's house, everything still.
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

    /// Says `text` once per night, under the key `key`.
    fn tell(&mut self, key: &'static str, text: &str, seconds: f32) {
        if !self.told.contains(&key) {
            self.told.push(key);
            self.say(text, seconds);
        }
    }
}

/// The clock on the HUD, e.g. "12:04 AM".
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
    stale: Query<Entity, (With<HouseEntity>, Without<ChildOf>)>,
    server: Res<AssetServer>,
    palette: Res<PaletteSlot>,
    library: Res<PoseLibrary>,
    mut current: ResMut<CurrentLevel>,
    (mut flow, mut eye, mut caught, mut camera, mut director): (
        ResMut<Flow>,
        ResMut<Eye>,
        ResMut<Caught>,
        ResMut<CameraOverride>,
        ResMut<Director>,
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
    log::info!("building {kind:?} night {} seed {}", game.night, game.seed);
    for entity in stale.iter() {
        cmd.despawn(entity);
    }

    let title = kind == Rebuild::Title;
    let night = if title { 1 } else { game.night };
    let seed = if title { 0x0071_71e5 } else { game.seed };
    let plan = plan(night, seed);
    let level = level::generate(&plan.spec);
    if title && crate::debug::pose_gallery(&mut cmd, &server, palette, &library) {
        state.ledger = None;
        current.0 = Some(level);
        return;
    }
    let built = house::build_house(&mut cmd, &server, palette, &level, night, seed);

    let start = level.start;
    let inward = -level.exit.dir.vector();
    let yaw = (-inward.x).atan2(-inward.z);
    let debug_escape = !title && platform::debug_flag("escape");
    let feet = start.center() + level.exit.dir.vector() * (CELL * 0.5 - 1.1) + Vec3::Y * 0.05;
    let (feet, yaw) = match crate::debug::room_view(&level).or(crate::debug::spawn_view(&level)) {
        Some(view) if !title => view,
        _ => (feet, yaw),
    };
    player::spawn_player(&mut cmd, feet, yaw);

    // Which figures are possessed: never the nearest to the door.
    let dist = level.distances(start);
    let mut order: Vec<usize> = (0..level.figures.len()).collect();
    order.sort_by_key(|&i| -level.distance(&dist, level.figures[i]));
    let possessed: Vec<usize> = if title {
        Vec::new()
    } else {
        let far = &order[..order.len().saturating_sub(1)];
        let mut pool: Vec<usize> = far.to_vec();
        let mut chosen = Vec::new();
        while chosen.len() < plan.possessed && !pool.is_empty() {
            chosen.push(pool.swap_remove(rand.index(pool.len())));
        }
        chosen
    };
    let bedroom_figure = level
        .figures
        .iter()
        .position(|c| level.room(*c).map(|r| r.kind) == Some(RoomKind::Bedroom));
    let clara = if plan.clara {
        bedroom_figure.or(possessed.first().copied())
    } else {
        None
    };

    for (i, &cell) in level.figures.iter().enumerate() {
        let jitter = Vec3::new(rand.range(-0.6, 0.6), 0.0, rand.range(-0.6, 0.6));
        let feet = cell.center() + jitter;
        let face = Dir::ALL[rand.index(4)].vector();
        let is_clara = clara == Some(i) || (!plan.clara && bedroom_figure == Some(i));
        let kind = if possessed.contains(&i) || (plan.clara && clara == Some(i)) {
            Kind::Possessed
        } else {
            Kind::Inert
        };
        let pose = library.pick(Menace::Display, None, &mut rand).unwrap_or(0);
        mannequin::spawn_mannequin(
            &mut cmd,
            &server,
            feet,
            yaw_toward(face),
            kind,
            if is_clara {
                CLARA_FINISH
            } else {
                rand.index(COMMON_FINISHES)
            },
            pose,
        );
    }

    let mut rules = plan.rules;
    if platform::debug_flag("hunt") {
        rules.hunt_after_lots = 0;
        rules.hunt_gap = (4.0, 6.0);
        rules.stalk_after = 0.0;
    }
    *director = Director::new(rules, &mut rand);
    state.ledger = Some(built.ledger);
    state.lots_total = level.lots.len();
    state.lots_done = 0;
    state.unlocked = debug_escape;
    state.heartbeat_timer = 0.0;
    state.chime_timer = 6.0;
    state.draught = plan.draught;
    state.draught_timer = plan.draught;
    state.message = None;
    state.reading = None;
    state.prompt = None;
    state.hold = 0.0;
    state.room = None;
    state.last_room = None;
    state.told.clear();
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
    if let Some((_, left)) = state.room.as_mut() {
        *left -= real_dt;
        if *left <= 0.0 {
            state.room = None;
        }
    }

    match game.phase {
        Phase::Title => {
            if game.phase_time > 0.5 && clicked(&input) {
                sounds.play(&mut audio, Sfx::Click, 0.6);
                game.night = platform::debug_value("night")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                game.seed = platform::debug_value("seed")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(rand.next_u32() as u64 ^ 0xa11_0005);
                start_night(&mut game, &mut state, &mut audio, &mut sounds);
            }
        }
        Phase::Intro => {
            if game.phase_time > 1.5 && clicked(&input) {
                game.set_phase(Phase::Playing);
                if game.night == 1 {
                    state.say(
                        "The marked pictures carry paper lot tags. Find them by lantern light.",
                        6.0,
                    );
                }
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
                game.paused = true;
            }
        }
        Phase::Caught => {}
        Phase::Dead => {
            if game.phase_time > 1.2 && clicked(&input) {
                sounds.play(&mut audio, Sfx::Click, 0.6);
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

fn facing(eye: &Eye, point: Vec3) -> f32 {
    (point - eye.position).normalize_or_zero().dot(eye.forward())
}

/// Lots, the diary page, oil and the ledger.
#[allow(clippy::too_many_arguments)]
pub fn interact(
    players: Query<&Transform, With<Player>>,
    lots: Query<&mut Lot>,
    pages: Query<(Entity, &PagePickup, &mut Transform), Without<Player>>,
    oil: Query<(Entity, &OilPickup, &mut Transform), (Without<Player>, Without<PagePickup>)>,
    (ledgers, candles): (Query<&Ledger>, Query<&mut Candle>),
    (flashlights, mannequins): (Query<&mut Flashlight>, Query<&Mannequin>),
    (input, eye, level): (Res<Input>, Res<Eye>, Res<CurrentLevel>),
    (palette, mut state, mut game, mut director): (
        Res<PaletteSlot>,
        ResMut<NightState>,
        ResMut<Game>,
        ResMut<Director>,
    ),
    (time, mut rand): (Res<Time>, ResMut<Rand>),
    (mut audio, mut sounds): (ResMut<Audio>, ResMut<Sounds>),
    mut cmd: CommandQueue,
) {
    let t = game.night_time;
    for (_, can, mut transform) in oil.iter() {
        transform.translation = can.base;
        transform.rotation = Quat::from_rotation_y(t * 0.3);
    }
    if !game.is_live() {
        state.prompt = None;
        state.hold = 0.0;
        return;
    }
    let dt = time.delta().as_secs_f32();
    game.night_time += dt;
    let Some(palette) = palette.get() else {
        return;
    };
    let Some(body) = players.iter().next() else {
        return;
    };
    let e_down = input.is_held(PhysicalKey::Code(KeyCode::KeyE));
    let e_pressed = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyE));

    if state.reading.is_some() {
        state.prompt = Some("E  put the page down".into());
        if e_pressed || input.is_mouse_button_just_pressed(MouseButton::Left) {
            state.reading = None;
            sounds.play(&mut audio, Sfx::Page, 0.5);
        }
        return;
    }

    if let Some(level) = level.0.as_ref()
        && let Some(i) = level.room_index(level::Cell::from_world(body.translation))
        && state.last_room != Some(i)
    {
        state.last_room = Some(i);
        state.room = Some((house::room_name(level.rooms[i].kind), 2.5));
    }

    // The most-faced thing in reach decides what E does.
    enum Target {
        Lot(usize),
        Page(Entity),
        Ledger,
    }
    let mut best: Option<(Target, f32, String)> = None;
    let mut consider = |target: Target, point: Vec3, reach: f32, min_facing: f32, text: String| {
        let d = (point - eye.position).length();
        let f = facing(&eye, point);
        if d < reach && f > min_facing && best.as_ref().is_none_or(|b| f > b.1) {
            best = Some((target, f, text));
        }
    };
    let lot_list: Vec<(usize, Vec3, bool, u32, &'static str)> = lots
        .iter()
        .enumerate()
        .map(|(i, l)| (i, l.center, l.done, l.number, l.title))
        .collect();
    for &(i, center, done, number, title) in &lot_list {
        if !done {
            consider(
                Target::Lot(i),
                center,
                REACH,
                0.82,
                format!("hold E  catalogue lot {number}, \u{2018}{title}\u{2019}"),
            );
        }
    }
    for (entity, page, _) in pages.iter() {
        consider(
            Target::Page(entity),
            page.base,
            2.0,
            0.7,
            "E  read the page".into(),
        );
    }
    if state.unlocked || state.lots_done >= state.lots_total {
        for ledger in ledgers.iter() {
            consider(
                Target::Ledger,
                ledger.point,
                REACH,
                0.6,
                "hold E  sign the ledger and leave".into(),
            );
        }
    }

    match best {
        None => {
            state.prompt = None;
            state.hold = 0.0;
        }
        Some((Target::Page(entity), _, text)) => {
            state.prompt = Some(text);
            state.hold = 0.0;
            if e_pressed {
                cmd.despawn(entity);
                state.reading = Some(story::page(game.night));
                sounds.play(&mut audio, Sfx::Page, 0.7);
            }
        }
        Some((Target::Lot(i), _, text)) => {
            state.prompt = Some(text);
            state.tell("hold", "Hold E while facing it. Keep your nerve.", 4.0);
            if e_down {
                state.hold += dt / CATALOGUE_SECONDS;
                if state.hold >= 1.0 {
                    state.hold = 0.0;
                    if let Some(mut lot) = lots.iter().nth(i) {
                        lot.done = true;
                        cmd.insert(
                            MaterialComponent {
                                handle: palette.tag_done.clone(),
                            },
                            lot.tag,
                        );
                    }
                    state.lots_done += 1;
                    director.lots_done = state.lots_done;
                    sounds.play(&mut audio, Sfx::Catalogue, 0.8);
                    let left = state.lots_total.saturating_sub(state.lots_done);
                    if left == 0 {
                        state.unlocked = true;
                        if let Some(entity) = state.ledger
                            && let Some(ledger) = ledgers.get_entity(entity)
                        {
                            cmd.insert(
                                MaterialComponent {
                                    handle: palette.ledger_ready.clone(),
                                },
                                ledger.book,
                            );
                        }
                        state.say("That is every lot. Back to the ledger in the hall.", 5.0);
                    } else {
                        state.say(
                            format!("Catalogued. {left} more on Mr Pike's list."),
                            3.0,
                        );
                    }
                    gutter_one(&candles, &mut rand);
                }
            } else {
                state.hold = (state.hold - dt * 2.0).max(0.0);
            }
        }
        Some((Target::Ledger, _, text)) => {
            state.prompt = Some(text);
            if e_down {
                state.hold += dt / SIGN_SECONDS;
                if state.hold >= 1.0 {
                    state.hold = 0.0;
                    sounds.play(&mut audio, Sfx::Escape, 0.9);
                    game.set_phase(Phase::Escaped);
                    if game.night >= game.best_night {
                        game.best_night = game.night + 1;
                        platform::save_best(game.best_night);
                    }
                }
            } else {
                state.hold = (state.hold - dt * 2.0).max(0.0);
            }
        }
    }

    let chest = body.translation + Vec3::Y;
    for (entity, can, _) in oil.iter() {
        if can.base.distance(chest) < 1.3 {
            cmd.despawn(entity);
            sounds.play(&mut audio, Sfx::Oil, 0.7);
            for mut lantern in flashlights.iter() {
                lantern.battery = (lantern.battery + 0.5).min(1.0);
                lantern.switched_on = true;
            }
            state.say("A tin of lamp oil.", 2.0);
        }
    }

    if game.night == 1 {
        if game.night_time > 12.0 {
            state.tell(
                "lantern",
                "F closes the lantern's shutter. The oil will not last all night.",
                5.0,
            );
        }
        let near_figure = mannequins.iter().any(|m| m.observed && m.ever_seen);
        if near_figure && game.night_time > 3.0 {
            state.tell(
                "figure",
                "One of Marrow's lay figures. He painted from them, they say.",
                4.5,
            );
        }
    }
}

/// Starts one steady candle guttering.
fn gutter_one(candles: &Query<&mut Candle>, rand: &mut Rand) {
    let steady = candles
        .iter()
        .filter(|c| !c.out && c.guttering.is_none())
        .count();
    if steady <= 2 {
        return;
    }
    let mut n = rand.index(steady);
    for mut candle in candles.iter() {
        if !candle.out && candle.guttering.is_none() {
            if n == 0 {
                candle.guttering = Some(rand.range(2.5, 4.0));
                return;
            }
            n -= 1;
        }
    }
}

/// The heartbeat, the chime that leads you on, draughts, and hunt cues.
#[allow(clippy::too_many_arguments)]
pub fn night_clock(
    mannequins: Query<(&Mannequin, &Transform), Without<Player>>,
    players: Query<&Transform, With<Player>>,
    lots: Query<&Lot>,
    ledgers: Query<&Ledger>,
    candles: Query<&mut Candle>,
    eye: Res<Eye>,
    mut state: ResMut<NightState>,
    mut director: ResMut<Director>,
    game: Res<Game>,
    (time, mut rand): (Res<Time>, ResMut<Rand>),
    (mut audio, mut sounds): (ResMut<Audio>, ResMut<Sounds>),
) {
    if !game.is_live() {
        return;
    }
    let dt = time.delta().as_secs_f32();
    let Some(player) = players.iter().next() else {
        return;
    };

    if let Some(at) = director.hunt_started.take() {
        let (volume, pan) = mannequin::spatialize(&eye, at + Vec3::Y * 1.5);
        sounds.play_at(&mut audio, Sfx::Knock, (volume * 1.2).max(0.3), pan);
    }

    if state.draught > 0.0 {
        state.draught_timer -= dt;
        if state.draught_timer <= 0.0 {
            state.draught_timer = state.draught * rand.range(0.7, 1.3);
            gutter_one(&candles, &mut rand);
        }
    }

    state.chime_timer -= dt;
    if state.chime_timer <= 0.0 {
        state.chime_timer = rand.range(6.0, 9.0);
        let source = if state.unlocked {
            ledgers.iter().next().map(|l| l.point)
        } else {
            lots.iter()
                .filter(|l| !l.done)
                .map(|l| l.center)
                .min_by(|a, b| {
                    a.distance_squared(player.translation)
                        .total_cmp(&b.distance_squared(player.translation))
                })
        };
        if let Some(point) = source {
            let (volume, pan) = mannequin::spatialize(&eye, point);
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

fn candle_noise(t: f32) -> f32 {
    ((t * 7.3).sin() * 0.5 + (t * 13.1).sin() * 0.3 + (t * 23.7).sin() * 0.2) * 0.5 + 0.5
}

/// Candles breathe; guttering ones stutter and go out.
#[allow(clippy::too_many_arguments)]
pub fn flicker_candles(
    candles: Query<(&mut Candle, &mut Light, &Transform)>,
    flames: Query<&mut Transform, Without<Candle>>,
    palette: Res<PaletteSlot>,
    game: Res<Game>,
    eye: Res<Eye>,
    time: Res<Time>,
    (mut audio, mut sounds): (ResMut<Audio>, ResMut<Sounds>),
    mut cmd: CommandQueue,
) {
    let Some(palette) = palette.get() else {
        return;
    };
    let dt = time.delta().as_secs_f32();
    for (mut candle, mut light, transform) in candles.iter() {
        if candle.out {
            continue;
        }
        candle.phase += dt;
        let mut level = 0.85 + 0.15 * candle_noise(candle.phase);
        if let Some(left) = candle.guttering {
            let left = left - dt;
            level *= if candle_noise(candle.phase * 3.0) > 0.55 { 1.0 } else { 0.15 };
            if left <= 0.0 {
                candle.out = true;
                candle.guttering = None;
                level = 0.0;
                cmd.insert(
                    MaterialComponent {
                        handle: palette.flame_out.clone(),
                    },
                    candle.flame,
                );
                if game.is_live() {
                    let (volume, pan) = mannequin::spatialize(&eye, transform.translation);
                    sounds.play_at(&mut audio, Sfx::Snuff, volume * 0.7, pan);
                }
            } else {
                candle.guttering = Some(left);
            }
        }
        light.intensity = candle.base_intensity * level;
        if let Some(mut flame) = flames.get_entity(candle.flame) {
            flame.scale = Vec3::new(1.0, 0.7 + 0.6 * level, 1.0);
        }
    }
}

/// On the title screen the view idles in the hall, looking slowly around.
pub fn title_drift(players: Query<&mut Player>, game: Res<Game>) {
    if game.phase != Phase::Title {
        return;
    }
    let t = game.phase_time;
    for mut player in players.iter() {
        player.yaw += (t * 0.11).sin() * 0.0025;
        player.pitch = -0.05 + (t * 0.23).sin() * 0.03;
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
    fn nights_build_up_slowly() {
        let first = plan(1, 0);
        assert_eq!(first.possessed, 0);
        assert_eq!(first.rules.max_hunters, 0);
        let mut last = first;
        for night in 2..10 {
            let next = plan(night, 0);
            assert!(next.possessed >= last.possessed);
            assert!(next.rules.hunt_speed >= last.rules.hunt_speed);
            assert!(next.rules.max_hunters <= 2, "never a horde");
            last = next;
        }
    }
}

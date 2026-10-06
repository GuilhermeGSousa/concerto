//! The mannequins: spawning, dressing, posing, the observation rule, and the
//! hunt.
use concerto::{
    animation::{
        clip::AnimationClip,
        player::{AnimationHandleComponent, AnimationPlayer},
    },
    audio::Audio,
    ecs::{
        CommandQueue, Component, Entity, Query, Res, ResMut, Resource, With, Without,
        entity::hierarchy::{ChildOf, Children},
    },
    foundation::{
        assets::{asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle},
        time::Time,
        transform::Transform,
    },
    physics::physics_state::PhysicsState,
    render::{MaterialComponent, assets::material::StandardMaterial, components::light::Light},
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
};
use glam::{Quat, Vec3};

use crate::{
    body::{flat, yaw_toward},
    content,
    game::{Game, Rand},
    house::{HouseEntity, LightSource},
    level::{CELL, Cell, Level},
    lighting::{self, PointLight, SpotLight},
    palette::PaletteSlot,
    player::{
        Eye, FLASHLIGHT_CONE, FLASHLIGHT_OFFSET, FLASHLIGHT_RANGE, FOG, FOV_Y, Flashlight, Player,
    },
    poses::{Menace, PoseLibrary},
    sfx::{Sfx, Sounds},
};

/// Mannequins have no physics body: the physics engine interpolates bodies
/// between fixed steps, which would let a mannequin glide on for a few
/// centimetres after being seen.
pub const RADIUS: f32 = 0.3;
const UNSEEN_GRACE: f32 = 0.12;
/// Closer than this while it moves, and it has you.
pub const CATCH_DISTANCE: f32 = 0.95;
const NEAR_SIGHT: f32 = 2.6;
const VIEW_MARGIN: f32 = 0.12;
const SETTLE_FRAMES: u8 = 3;
const STALK_KEEP_AWAY: f32 = 3.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Wood and linen.
    Inert,
    /// Something is in it.
    Possessed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Still,
    /// Holds its place, but turns to follow you when you look away.
    Watch,
    /// Takes up a new place near you, out of sight.
    Stalk,
    /// Comes for you.
    Hunt,
    /// Withdraws somewhere far away after a hunt.
    Retreat,
}

#[derive(Component)]
pub struct Mannequin {
    pub kind: Kind,
    pub mood: Mood,
    visual: Entity,
    finish: usize,
    anim: Option<Entity>,
    pose: Option<usize>,
    wanted_pose: Option<usize>,
    settle: u8,
    pub observed: bool,
    pub ever_seen: bool,
    unseen_for: f32,
    stare: f32,
    moving: bool,
    creak_timer: f32,
    fidget: f32,
    mood_time: f32,
    pub yaw: f32,
    target: Option<Cell>,
    route: Vec<i32>,
    one_shot: bool,
}

impl Mannequin {
    pub fn is_hunting(&self) -> bool {
        self.mood == Mood::Hunt
    }

    pub fn anim_entity(&self) -> Option<Entity> {
        self.anim
    }

    fn set_target(&mut self, level: &Level, cell: Cell) {
        self.target = Some(cell);
        self.route = level.distances(cell);
        self.mood_time = 0.0;
        self.moving = false;
    }
}

/// Marks an animation player that belongs to a mannequin and has been dressed.
#[derive(Component)]
pub struct Dressed;

pub fn spawn_mannequin(
    cmd: &mut CommandQueue,
    server: &AssetServer,
    feet: Vec3,
    yaw: f32,
    kind: Kind,
    finish: usize,
    pose: usize,
) -> Entity {
    let visual = cmd
        .spawn((
            HouseEntity,
            Transform::from_rotation(Quat::from_rotation_y(yaw)),
            SceneSpawnerComponent(server.load::<Scene>(content::MANNEQUIN_SCENE)),
        ))
        .entity();
    let root = cmd
        .spawn((
            HouseEntity,
            Mannequin {
                kind,
                mood: if kind == Kind::Possessed {
                    Mood::Watch
                } else {
                    Mood::Still
                },
                visual,
                finish,
                anim: None,
                pose: None,
                wanted_pose: Some(pose),
                settle: 0,
                observed: false,
                ever_seen: false,
                unseen_for: 0.0,
                stare: 0.0,
                moving: false,
                creak_timer: 0.0,
                fidget: 10.0,
                mood_time: 0.0,
                yaw,
                target: None,
                route: Vec::new(),
                one_shot: false,
            },
            Transform::from_translation(feet),
        ))
        .entity();
    cmd.add_child(root, visual);
    root
}

fn find_owner(
    mut entity: Entity,
    parents: &Query<&ChildOf>,
    owners: &Query<&mut Mannequin>,
) -> Option<Entity> {
    for _ in 0..32 {
        if owners.contains_entity(entity) {
            return Some(entity);
        }
        entity = parents.get_entity(entity)?.parent();
    }
    None
}

/// Once a mannequin's scene has spawned, repaints it in its finish and hooks up
/// its animation player.
pub fn dress_mannequins(
    players: Query<Entity, (With<AnimationPlayer>, Without<Dressed>)>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    materials: Query<&MaterialComponent<StandardMaterial>>,
    mannequins: Query<&mut Mannequin>,
    palette: Res<PaletteSlot>,
    mut cmd: CommandQueue,
) {
    let Some(palette) = palette.get() else {
        return;
    };
    for player in players.iter() {
        let Some(root) = find_owner(player, &parents, &mannequins) else {
            continue;
        };
        let Some(mut mannequin) = mannequins.get_entity(root) else {
            continue;
        };
        mannequin.anim = Some(player);
        cmd.insert(Dressed, player);
        crate::platform::mark_ready();

        let finish = &palette.finishes[mannequin.finish % palette.finishes.len()];
        let mut stack = vec![mannequin.visual];
        while let Some(entity) = stack.pop() {
            if let Some(material) = materials.get_entity(entity) {
                let handle: AssetHandle<StandardMaterial> =
                    if material.handle.id() == content::MANNEQUIN_MATERIAL_0 {
                        finish.main.clone()
                    } else {
                        finish.joints.clone()
                    };
                cmd.insert(MaterialComponent { handle }, entity);
            }
            if let Some(kids) = children.get_entity(entity) {
                stack.extend(kids.iter().copied());
            }
        }
    }
}

/// Applies wanted poses and pauses settled ones.
pub fn apply_poses(
    mannequins: Query<&mut Mannequin>,
    anim_players: Query<&mut AnimationPlayer>,
    library: Res<PoseLibrary>,
    clips: Res<AssetStore<AnimationClip>>,
    mut cmd: CommandQueue,
) {
    for mut mannequin in mannequins.iter() {
        let Some(anim) = mannequin.anim else {
            continue;
        };
        let Some(mut player) = anim_players.get_entity(anim) else {
            continue;
        };
        let wanted = mannequin.wanted_pose.take();
        if let Some(wanted) = wanted.filter(|w| mannequin.pose != Some(*w))
            && let Some(graph) = library.graph(wanted)
        {
            cmd.insert(AnimationHandleComponent::new(graph), anim);
            player.set_paused(false);
            mannequin.pose = Some(wanted);
            mannequin.settle = SETTLE_FRAMES;
        }
        let clip_ready = mannequin
            .pose
            .and_then(|pose| library.clip(pose))
            .is_none_or(|clip| clips.get(clip).is_some());
        if mannequin.settle > 0 && clip_ready {
            mannequin.settle -= 1;
            if mannequin.settle == 0 {
                player.set_paused(true);
            }
        }
    }
}

/// Lighting and eye state the observation test needs, gathered once a frame.
pub struct Sight<'a> {
    pub eye: &'a Eye,
    pub flashlight: Option<SpotLight>,
    pub lights: &'a [PointLight],
    pub physics: &'a PhysicsState,
}

fn in_view(position: Vec3, rotation: Quat, aspect: f32, point: Vec3) -> bool {
    let local = rotation.inverse() * (point - position);
    let depth = -local.z;
    if depth < 0.02 {
        return local.length() < 0.5;
    }
    let tan_v = (FOV_Y * 0.5 + VIEW_MARGIN).tan();
    let tan_h = ((FOV_Y * 0.5).tan() * aspect).atan() + VIEW_MARGIN;
    local.y.abs() <= depth * tan_v && local.x.abs() <= depth * tan_h.tan()
}

fn is_lit(sight: &Sight, point: Vec3) -> bool {
    if (point - sight.eye.position).length() < NEAR_SIGHT {
        return true;
    }
    let fog = FOG.amount((point - sight.eye.position).length());
    lighting::irradiance(point, sight.lights, sight.flashlight.as_ref()) * (1.0 - fog)
        >= lighting::VISIBLE_IRRADIANCE
}

fn line_of_sight(sight: &Sight, from: Vec3, point: Vec3, owner: Entity) -> bool {
    let to_point = point - from;
    let distance = to_point.length();
    if distance < 0.05 {
        return true;
    }
    let dir = to_point / distance;
    let start = from + dir * 0.4;
    let Some(hit) = sight
        .physics
        .cast_ray(start, dir * (distance - 0.4).max(0.0))
    else {
        return true;
    };
    let _ = owner;
    hit.point.distance(start) >= distance - 0.4 - 0.05
}

/// Whether any part of the mannequin at `feet` can be seen from either eye.
pub fn is_observed(sight: &Sight, owner: Entity, feet: Vec3) -> bool {
    if !sight.eye.valid {
        return true;
    }
    let points = [
        feet + Vec3::Y * 0.35,
        feet + Vec3::Y * 1.1,
        feet + Vec3::Y * 1.65,
    ];
    let eyes = [
        (sight.eye.position, sight.eye.rotation),
        (sight.eye.previous_position, sight.eye.previous_rotation),
    ];
    for point in points {
        if !is_lit(sight, point) {
            continue;
        }
        for (position, rotation) in eyes {
            if in_view(position, rotation, sight.eye.aspect, point)
                && line_of_sight(sight, position, point, owner)
            {
                return true;
            }
        }
    }
    false
}

/// Steps from every cell to the player, rebuilt when the player changes cell.
#[derive(Resource, Default)]
pub struct Flow {
    target: Option<Cell>,
    distances: Vec<i32>,
}

#[derive(Resource, Default)]
pub struct CurrentLevel(pub Option<Level>);

/// How the possessed figures behave on a given night.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rules {
    pub max_stalkers: usize,
    pub max_hunters: usize,
    /// Seconds into the night before anything stalks.
    pub stalk_after: f32,
    /// Lots that must be catalogued before anything hunts.
    pub hunt_after_lots: usize,
    /// Seconds between hunts, as a random range.
    pub hunt_gap: (f32, f32),
    pub hunt_speed: f32,
    pub stalk_speed: f32,
    /// How long a hunt presses before it can be stared down.
    pub hunt_duration: f32,
    /// Night one's one-off: after this many lots, a figure you have seen is
    /// somewhere else.
    pub doubt_after_lots: Option<usize>,
}

/// Paces the night: who may stalk, who may hunt, and when.
#[derive(Resource, Default)]
pub struct Director {
    pub rules: Rules,
    pub lots_done: usize,
    hunt_cooldown: f32,
    stalk_cooldown: f32,
    doubt_done: bool,
    /// A cue for the audio: a figure has just started hunting here.
    pub hunt_started: Option<Vec3>,
}

impl Director {
    pub fn new(rules: Rules, rand: &mut Rand) -> Self {
        Self {
            rules,
            lots_done: 0,
            hunt_cooldown: rand.range(rules.hunt_gap.0, rules.hunt_gap.1) * 0.5,
            stalk_cooldown: rules.stalk_after,
            doubt_done: false,
            hunt_started: None,
        }
    }
}

fn waypoint(level: &Level, distances: &[i32], at: Vec3, goal: Vec3) -> Vec3 {
    let cell = Cell::from_world(at);
    if !level.contains(cell) || distances.is_empty() {
        return goal;
    }
    let here = level.distance(distances, cell);
    if here == 0 {
        return goal;
    }
    let Some(next) = level
        .neighbors(cell)
        .filter(|n| level.distance(distances, *n) < here)
        .min_by_key(|n| level.distance(distances, *n))
    else {
        return goal;
    };
    let axis = (next.center() - cell.center()).normalize_or_zero();
    let offset = flat(at - cell.center());
    let lateral = offset - axis * offset.dot(axis);
    if lateral.length() > 0.3 {
        return cell.center() + axis * offset.dot(axis).max(0.0);
    }
    next.center()
}

fn stalk_cell(
    level: &Level,
    flow: &Flow,
    eye: &Eye,
    player: Vec3,
    taken: &[Cell],
    rand: &mut Rand,
) -> Option<Cell> {
    let forward = flat(eye.forward()).normalize_or_zero();
    level
        .cells()
        .filter(|c| {
            let d = level.distance(&flow.distances, *c);
            (2..=3).contains(&d)
        })
        .filter(|c| taken.iter().all(|t| t.manhattan(*c) >= 2))
        .map(|c| {
            let to = flat(c.center() - player).normalize_or_zero();
            let behind = -to.dot(forward);
            (c, behind * 2.0 + rand.unit())
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c)
}

fn retreat_cell(level: &Level, flow: &Flow, taken: &[Cell], rand: &mut Rand) -> Option<Cell> {
    level
        .cells()
        .filter(|c| level.distance(&flow.distances, *c) >= 4)
        .filter(|c| taken.iter().all(|t| t.manhattan(*c) >= 2))
        .map(|c| {
            (
                c,
                level.distance(&flow.distances, c).min(7) as f32 + rand.unit() * 3.0,
            )
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c)
}

/// Decides moods: who stalks, who hunts, who gives up.
#[allow(clippy::too_many_arguments)]
pub fn direct(
    mannequins: Query<(Entity, &mut Mannequin, &Transform), Without<Player>>,
    players: Query<&Transform, With<Player>>,
    eye: Res<Eye>,
    (level, mut flow): (Res<CurrentLevel>, ResMut<Flow>),
    mut director: ResMut<Director>,
    game: Res<Game>,
    (time, mut rand): (Res<Time>, ResMut<Rand>),
) {
    let Some(level) = level.0.as_ref() else {
        return;
    };
    let Some(player) = players.iter().next() else {
        return;
    };
    let player_cell = Cell::from_world(player.translation);
    if flow.target != Some(player_cell) && level.contains(player_cell) {
        flow.target = Some(player_cell);
        flow.distances = level.distances(player_cell);
    }
    if !game.is_live() || crate::platform::debug_flag("nohunt") {
        return;
    }
    let dt = time.delta().as_secs_f32();
    let rules = director.rules;
    director.stalk_cooldown -= dt;
    let hunting = mannequins
        .iter()
        .filter(|(_, m, _)| m.mood == Mood::Hunt)
        .count();
    if hunting == 0 && director.lots_done >= rules.hunt_after_lots {
        director.hunt_cooldown -= dt;
    }

    let mut taken: Vec<Cell> = mannequins
        .iter()
        .map(|(_, m, t)| m.target.unwrap_or(Cell::from_world(t.translation)))
        .collect();
    taken.push(player_cell);

    if let Some(after) = rules.doubt_after_lots
        && !director.doubt_done
        && director.lots_done >= after
    {
        let candidate = mannequins.iter().find(|(_, m, t)| {
            m.ever_seen
                && !m.observed
                && level.distance(&flow.distances, Cell::from_world(t.translation)) >= 2
        });
        if let Some((_, mut m, _)) = candidate
            && let Some(cell) =
                stalk_cell(level, &flow, &eye, player.translation, &taken, &mut rand)
        {
            m.set_target(level, cell);
            m.mood = Mood::Stalk;
            m.one_shot = true;
            director.doubt_done = true;
        }
    }

    for (_, mut m, t) in mannequins.iter() {
        if m.kind != Kind::Possessed {
            continue;
        }
        m.mood_time += dt;
        let cell = Cell::from_world(t.translation);
        let away = level.distance(&flow.distances, cell);
        match m.mood {
            Mood::Hunt => {
                let stared_down = m.stare >= 1.2 && m.mood_time > rules.hunt_duration;
                let lost_you = away > 6;
                let tired = m.mood_time > rules.hunt_duration * 2.5 && !m.observed;
                if stared_down || lost_you || tired {
                    if let Some(c) = retreat_cell(level, &flow, &taken, &mut rand) {
                        m.set_target(level, c);
                        m.mood = Mood::Retreat;
                    } else {
                        m.mood = Mood::Watch;
                    }
                    director.hunt_cooldown = rand.range(rules.hunt_gap.0, rules.hunt_gap.1);
                }
            }
            Mood::Stalk
                if m.target.is_none() && m.mood_time > rand.range(8.0, 16.0) && away >= 4 =>
            {
                m.mood = Mood::Watch;
            }
            _ => {}
        }
    }

    let stalking = mannequins
        .iter()
        .filter(|(_, m, _)| m.mood == Mood::Stalk && !m.one_shot)
        .count();
    if stalking < rules.max_stalkers && director.stalk_cooldown <= 0.0 {
        director.stalk_cooldown = rand.range(6.0, 14.0);
        let pick = mannequins
            .iter()
            .filter(|(_, m, _)| m.kind == Kind::Possessed && m.mood == Mood::Watch && !m.observed)
            .map(|(e, _, t)| {
                (
                    e,
                    level.distance(&flow.distances, Cell::from_world(t.translation)),
                )
            })
            .filter(|(_, d)| *d >= 3)
            .max_by_key(|(_, d)| *d);
        if let Some((e, _)) = pick
            && let Some(cell) =
                stalk_cell(level, &flow, &eye, player.translation, &taken, &mut rand)
            && let Some((_, mut m, _)) = mannequins.get_entity(e)
        {
            m.set_target(level, cell);
            m.mood = Mood::Stalk;
        }
    }

    if hunting < rules.max_hunters
        && director.hunt_cooldown <= 0.0
        && director.lots_done >= rules.hunt_after_lots
    {
        let pick = mannequins
            .iter()
            .filter(|(_, m, _)| {
                m.kind == Kind::Possessed
                    && matches!(m.mood, Mood::Watch | Mood::Stalk)
                    && !m.observed
            })
            .map(|(e, _, t)| {
                (
                    e,
                    t.translation,
                    level.distance(&flow.distances, Cell::from_world(t.translation)),
                )
            })
            .filter(|(_, _, d)| (2..=5).contains(d))
            .min_by_key(|(_, _, d)| *d);
        if let Some((e, at, _)) = pick
            && let Some((_, mut m, _)) = mannequins.get_entity(e)
        {
            m.mood = Mood::Hunt;
            m.mood_time = 0.0;
            m.moving = false;
            m.target = None;
            m.route.clear();
            m.stare = 0.0;
            director.hunt_started = Some(at);
            director.hunt_cooldown = rand.range(rules.hunt_gap.0, rules.hunt_gap.1);
        }
    }
}

/// Moves whatever is unseen and has somewhere to be.
#[allow(clippy::too_many_arguments)]
pub fn hunt(
    mannequins: Query<(Entity, &mut Mannequin, &mut Transform), Without<Player>>,
    players: Query<&Transform, With<Player>>,
    flashlights: Query<&Flashlight>,
    candles: Query<(&Light, &Transform), (With<LightSource>, Without<Mannequin>)>,
    (eye, physics): (Res<Eye>, Res<PhysicsState>),
    (level, flow, library): (Res<CurrentLevel>, Res<Flow>, Res<PoseLibrary>),
    (mut game, mut caught, director): (ResMut<Game>, ResMut<crate::scare::Caught>, Res<Director>),
    (time, mut rand): (Res<Time>, ResMut<Rand>),
    (mut audio, mut sounds): (ResMut<Audio>, ResMut<Sounds>),
) {
    let dt = time.delta().as_secs_f32();
    let Some(level) = level.0.as_ref() else {
        return;
    };
    let Some(player) = players.iter().next() else {
        return;
    };
    let player_pos = player.translation;
    let live = game.is_live();

    let mut lights: Vec<PointLight> = candles
        .iter()
        .filter(|(light, ..)| light.intensity > 0.0)
        .map(|(light, t)| PointLight {
            position: t.translation,
            intensity: light.intensity,
            range: light.range,
        })
        .collect();
    lights.extend(
        flashlights
            .iter()
            .filter(|f| f.is_emitting())
            .map(|f| PointLight {
                position: eye.position + eye.rotation * FLASHLIGHT_OFFSET,
                intensity: f.glow(),
                range: crate::player::LANTERN_GLOW_RANGE,
            }),
    );
    let flashlight = flashlights
        .iter()
        .find(|f| f.is_emitting())
        .map(|f| SpotLight {
            position: eye.position + eye.rotation * FLASHLIGHT_OFFSET,
            direction: eye.forward(),
            intensity: f.intensity,
            range: FLASHLIGHT_RANGE,
            cone: FLASHLIGHT_CONE,
        });
    let sight = Sight {
        eye: &eye,
        flashlight,
        lights: &lights,
        physics: &physics,
    };

    let positions: Vec<(Entity, Vec3)> = mannequins
        .iter()
        .map(|(e, _, t)| (e, t.translation))
        .collect();

    for (entity, mut mannequin, mut transform) in mannequins.iter() {
        let feet = transform.translation;
        mannequin.observed = is_observed(&sight, entity, feet);
        if mannequin.observed {
            mannequin.ever_seen = true;
            mannequin.stare += dt;
        } else {
            mannequin.stare = 0.0;
        }
        if mannequin.observed || !live || crate::platform::debug_flag("nohunt") {
            mannequin.unseen_for = 0.0;
            mannequin.moving = false;
            continue;
        }
        mannequin.unseen_for += dt;
        if mannequin.unseen_for < UNSEEN_GRACE || mannequin.settle > 0 {
            continue;
        }

        let to_player = flat(player_pos - feet);
        let distance = to_player.length();

        if mannequin.kind == Kind::Possessed && mannequin.mood == Mood::Watch {
            if distance < CELL * 3.5 {
                let facing = yaw_toward(to_player);
                let turn = (facing - mannequin.yaw + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                if turn.abs() > 0.3 {
                    mannequin.yaw = facing;
                    if distance < CELL * 2.5 {
                        let (volume, pan) = spatialize(&eye, feet + Vec3::Y * 1.6);
                        sounds.play_at(&mut audio, Sfx::Joint, volume * 0.35, pan);
                    }
                }
                mannequin.fidget -= dt;
                if mannequin.fidget <= 0.0 {
                    mannequin.fidget = rand.range(15.0, 30.0);
                    mannequin.wanted_pose = library.pick(Menace::Uneasy, mannequin.pose, &mut rand);
                }
            }
            continue;
        }

        let (goal, distances, speed) = match mannequin.mood {
            Mood::Hunt => (player_pos, &flow.distances, director.rules.hunt_speed),
            Mood::Stalk | Mood::Retreat => {
                let Some(target) = mannequin.target else {
                    continue;
                };
                let speed = if mannequin.mood == Mood::Retreat {
                    director.rules.stalk_speed * 1.4
                } else {
                    director.rules.stalk_speed
                };
                (target.center(), &mannequin.route, speed)
            }
            _ => continue,
        };
        let distances = distances.clone();

        if !mannequin.moving {
            mannequin.moving = true;
            let menace = match mannequin.mood {
                Mood::Hunt if distance < CELL * 1.5 => Menace::Hunting,
                Mood::Hunt => {
                    if rand.unit() < 0.5 {
                        Menace::Hunting
                    } else {
                        Menace::Uneasy
                    }
                }
                Mood::Stalk if !mannequin.one_shot && rand.unit() < 0.6 => Menace::Uneasy,
                _ => Menace::Display,
            };
            mannequin.wanted_pose = library.pick(menace, mannequin.pose, &mut rand);
        }

        if mannequin.mood == Mood::Hunt && distance < CATCH_DISTANCE {
            if !caught.active {
                caught.catch(entity);
                game.set_phase(crate::game::Phase::Caught);
            }
            continue;
        }

        let arrived = flat(goal - feet).length() < 0.15;
        let too_close = mannequin.mood == Mood::Stalk && distance < STALK_KEEP_AWAY;
        if mannequin.mood != Mood::Hunt && (arrived || too_close) {
            mannequin.target = None;
            mannequin.route.clear();
            mannequin.moving = false;
            mannequin.yaw = yaw_toward(to_player);
            mannequin.mood_time = 0.0;
            if mannequin.one_shot {
                let (volume, pan) = spatialize(&eye, feet + Vec3::Y * 1.6);
                sounds.play_at(&mut audio, Sfx::Joint, volume * 0.5, pan);
                mannequin.one_shot = false;
                mannequin.kind = Kind::Inert;
                mannequin.mood = Mood::Still;
            } else if mannequin.mood == Mood::Retreat {
                mannequin.mood = Mood::Watch;
            }
            continue;
        }

        let way = waypoint(level, &distances, feet, goal);
        let mut dir = flat(way - feet).normalize_or_zero();
        for &(other, pos) in &positions {
            if other != entity {
                let away = flat(feet - pos);
                let d = away.length();
                if d < 1.0 && d > 1e-3 {
                    dir += away / d * (1.0 - d) * 2.0;
                }
            }
        }
        let limit = if mannequin.mood == Mood::Hunt {
            (distance - CATCH_DISTANCE * 0.8).max(0.0)
        } else {
            flat(goal - feet).length()
        };
        let step = (speed * dt).min(limit);
        transform.translation = feet + dir.normalize_or_zero() * step;
        mannequin.yaw = if mannequin.mood == Mood::Hunt {
            yaw_toward(to_player)
        } else {
            yaw_toward(dir)
        };

        mannequin.creak_timer -= dt;
        if mannequin.creak_timer <= 0.0 {
            mannequin.creak_timer = rand.range(0.7, 1.4);
            let (volume, pan) = spatialize(&eye, feet + Vec3::Y);
            let loud = if mannequin.mood == Mood::Hunt {
                0.9
            } else {
                0.45
            };
            sounds.play_at(&mut audio, Sfx::Creak, volume * loud, pan);
        }
    }
}

/// Volume falloff and stereo pan for a sound at `point`, heard from the eye.
pub fn spatialize(eye: &Eye, point: Vec3) -> (f32, f32) {
    let to = point - eye.position;
    let distance = to.length();
    let right = eye.rotation * Vec3::X;
    let pan = to.normalize_or_zero().dot(right) * 0.85;
    let volume = 1.0 / (1.0 + distance * 0.18);
    (volume, pan)
}

/// Turns each mannequin's visual to face its yaw.
pub fn face_mannequins(
    mannequins: Query<&Mannequin>,
    visuals: Query<&mut Transform, Without<Mannequin>>,
    game: Res<Game>,
) {
    for mannequin in mannequins.iter() {
        if mannequin.observed && game.phase != crate::game::Phase::Caught {
            continue;
        }
        if let Some(mut transform) = visuals.get_entity(mannequin.visual) {
            transform.rotation = Quat::from_rotation_y(mannequin.yaw);
        }
    }
}

/// Keeps the player from walking through mannequins: velocity into one is
/// cancelled, and any overlap is pushed back out.
pub fn block_player(
    players: Query<(&Transform, &mut crate::body::Mover), With<Player>>,
    mannequins: Query<&Transform, With<Mannequin>>,
) {
    let reach = RADIUS + crate::player::RADIUS;
    for (transform, mut mover) in players.iter() {
        let mut desired = mover.desired;
        for mannequin in mannequins.iter() {
            let away = flat(transform.translation - mannequin.translation);
            let d = away.length();
            if d >= reach + 0.15 || d < 1e-4 {
                continue;
            }
            let normal = away / d;
            let into = desired.dot(-normal);
            if into > 0.0 {
                desired += normal * into;
            }
            if d < reach {
                desired += normal * (reach - d) * 12.0;
            }
        }
        mover.desired = desired;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn house(seed: u64) -> Level {
        crate::level::generate(&crate::level::LevelSpec {
            size: 7,
            lots: 4,
            oil: 2,
            figures: 8,
            lit_fraction: 0.3,
            max_lights: 12,
            loop_doors: 0.35,
            bedroom: true,
            seed,
        })
    }

    #[test]
    fn stalkers_wait_near_but_not_on_top_of_you_and_behind_you() {
        for seed in 0..30 {
            let level = house(seed);
            let player_cell = level.start;
            let flow = Flow {
                target: Some(player_cell),
                distances: level.distances(player_cell),
            };
            let eye = Eye {
                rotation: Quat::IDENTITY,
                valid: true,
                ..Default::default()
            };
            let mut rand = Rand::new(seed);
            let taken = vec![player_cell];
            let Some(cell) =
                stalk_cell(&level, &flow, &eye, player_cell.center(), &taken, &mut rand)
            else {
                continue;
            };
            let steps = level.distance(&flow.distances, cell);
            assert!((2..=3).contains(&steps), "stalk cell {steps} steps away");
            let blocked = vec![player_cell, cell];
            if let Some(other) = stalk_cell(
                &level,
                &flow,
                &eye,
                player_cell.center(),
                &blocked,
                &mut rand,
            ) {
                assert!(other.manhattan(cell) >= 2, "two stalkers crowd one spot");
            }
        }
    }

    #[test]
    fn retreats_go_far_away() {
        for seed in 0..30 {
            let level = house(seed);
            let flow = Flow {
                target: Some(level.start),
                distances: level.distances(level.start),
            };
            let mut rand = Rand::new(seed);
            if let Some(cell) = retreat_cell(&level, &flow, &[level.start], &mut rand) {
                assert!(level.distance(&flow.distances, cell) >= 4);
            }
        }
    }

    #[test]
    fn view_test_matches_the_camera() {
        let rotation = Quat::IDENTITY;
        let pos = Vec3::ZERO;
        assert!(in_view(
            pos,
            rotation,
            16.0 / 9.0,
            Vec3::new(0.0, 0.0, -5.0)
        ));
        assert!(!in_view(
            pos,
            rotation,
            16.0 / 9.0,
            Vec3::new(0.0, 0.0, 5.0)
        ));
        let half_h = ((FOV_Y * 0.5).tan() * 16.0 / 9.0).atan();
        let edge = Vec3::new((half_h + 0.05).tan() * 5.0, 0.0, -5.0);
        assert!(in_view(pos, rotation, 16.0 / 9.0, edge));
        let outside = Vec3::new((half_h + 0.3).tan() * 5.0, 0.0, -5.0);
        assert!(!in_view(pos, rotation, 16.0 / 9.0, outside));
        assert!(in_view(pos, rotation, 1.0, Vec3::new(0.0, 0.0, 0.3)));
    }

    #[test]
    fn spatialize_pans_toward_the_source() {
        let eye = Eye {
            rotation: Quat::IDENTITY,
            valid: true,
            ..Default::default()
        };
        let (_, right) = spatialize(&eye, Vec3::new(5.0, 0.0, 0.0));
        let (_, left) = spatialize(&eye, Vec3::new(-5.0, 0.0, 0.0));
        assert!(right > 0.5 && left < -0.5);
        let (near, _) = spatialize(&eye, Vec3::new(0.0, 0.0, -1.0));
        let (far, _) = spatialize(&eye, Vec3::new(0.0, 0.0, -20.0));
        assert!(near > far);
    }
}

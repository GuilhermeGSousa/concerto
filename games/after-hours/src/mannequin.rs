//! The mannequins: spawning, dressing, posing, the observation rule, and the
//! hunt.
//!
//! The one rule the player must be able to trust: **a mannequin that can be
//! seen does not move.** "Seen" is computed conservatively in
//! [`is_observed`], against both this frame's and last frame's eye.
use concerto::{
    animation::player::{AnimationHandleComponent, AnimationPlayer},
    audio::Audio,
    ecs::{
        CommandQueue, Component, Entity, Query, Res, ResMut, Resource, With, Without,
        entity::hierarchy::{ChildOf, Children},
    },
    foundation::{
        assets::{asset_server::AssetServer, handle::AssetHandle},
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
    level::{CELL, Cell, Level},
    lighting::{self, PointLight, SpotLight},
    palette::PaletteSlot,
    player::{
        Eye, FLASHLIGHT_CONE, FLASHLIGHT_OFFSET, FLASHLIGHT_RANGE, FOG, FOV_Y, Flashlight, Player,
    },
    poses::{Menace, PoseLibrary},
    sfx::{Sfx, Sounds},
    store::{CeilingLight, StoreEntity},
};

/// Mannequins have no physics body: the physics engine interpolates bodies
/// between fixed steps, which would let a mannequin glide on for a few
/// centimetres after being seen. They move by transform alone, and the
/// player is kept out of them by `block_player`.
pub const RADIUS: f32 = 0.3;
/// Seconds a mannequin must go unseen before it may move: a flicker of a
/// glance still protects you.
const UNSEEN_GRACE: f32 = 0.12;
/// Closer than this while it moves, and it has you.
pub const CATCH_DISTANCE: f32 = 0.95;
/// You can make out shapes this close even in the dark.
const NEAR_SIGHT: f32 = 2.6;
/// Widening of the view frustum used for observation, in radians, covering
/// rounding and anything peeking in at the screen edge.
const VIEW_MARGIN: f32 = 0.12;
/// Frames a new pose is left unpaused so it gets evaluated onto the bones.
const SETTLE_FRAMES: u8 = 3;
/// Beyond this many cells of walking distance, hunters only stalk.
const STALK_CELLS: i32 = 3;
const STALK_SPEED: f32 = 0.55;
/// A dormant hunter wakes at once if the player comes this close (cells).
const WAKE_CELLS: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Stalks the player whenever unseen.
    Hunter,
    /// Stands still. For now.
    Decoy,
}

#[derive(Component)]
pub struct Mannequin {
    pub kind: Kind,
    visual: Entity,
    finish: usize,
    anim: Option<Entity>,
    /// Pose index into the library, and the one waiting to be applied.
    pose: Option<usize>,
    wanted_pose: Option<usize>,
    settle: u8,
    pub observed: bool,
    unseen_for: f32,
    /// True while a burst of unseen movement is under way; a fresh pose is
    /// chosen at the start of each burst.
    moving: bool,
    creak_timer: f32,
    pub speed: f32,
    pub yaw: f32,
    /// Seconds before a hunter starts hunting. Walking right up to it wakes
    /// it early.
    dormant: f32,
}

impl Mannequin {
    pub fn is_hunting(&self) -> bool {
        self.kind == Kind::Hunter && self.dormant <= 0.0
    }

    pub fn wake(&mut self) {
        self.kind = Kind::Hunter;
        self.dormant = 0.0;
    }

    pub fn anim_entity(&self) -> Option<Entity> {
        self.anim
    }
}

/// Marks an animation player that belongs to a mannequin and has been dressed.
#[derive(Component)]
pub struct Dressed;

#[allow(clippy::too_many_arguments)]
pub fn spawn_mannequin(
    cmd: &mut CommandQueue,
    server: &AssetServer,
    feet: Vec3,
    yaw: f32,
    kind: Kind,
    speed: f32,
    finish: usize,
    pose: usize,
    dormant: f32,
) -> Entity {
    let visual = cmd
        .spawn((
            StoreEntity,
            Transform::from_rotation(Quat::from_rotation_y(yaw)),
            SceneSpawnerComponent(server.load::<Scene>(content::MANNEQUIN_SCENE)),
        ))
        .entity();
    let root = cmd
        .spawn((
            StoreEntity,
            Mannequin {
                kind,
                visual,
                finish,
                anim: None,
                pose: None,
                wanted_pose: Some(pose),
                settle: 0,
                observed: false,
                unseen_for: 0.0,
                moving: false,
                creak_timer: 0.0,
                speed,
                yaw,
                dormant,
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

/// Once a mannequin's scene has spawned, repaints it in its finish and hooks
/// up its animation player.
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
        if mannequin.settle > 0 {
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
        // Behind the eye, or so close it straddles the near plane: a body
        // this close is on screen whichever way it leans.
        return local.length() < 0.5;
    }
    let tan_v = (FOV_Y * 0.5 + VIEW_MARGIN).tan();
    let tan_h = ((FOV_Y * 0.5).tan() * aspect).atan() + VIEW_MARGIN;
    local.y.abs() <= depth * tan_v && local.x.abs() <= depth * tan_h.tan()
}

/// Whether `point` is bright enough on screen to make out, using the same
/// falloff the renderer uses. Up close you can see shapes even in the dark.
fn is_lit(sight: &Sight, point: Vec3) -> bool {
    if (point - sight.eye.position).length() < NEAR_SIGHT {
        return true;
    }
    // What the fog swallows is as good as dark.
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
    // Start just outside the player's own capsule.
    let start = from + dir * 0.4;
    let Some(hit) = sight
        .physics
        .cast_ray(start, dir * (distance - 0.4).max(0.0))
    else {
        return true;
    };
    let _ = owner;
    // Anything solid short of the point hides it.
    hit.point.distance(start) >= distance - 0.4 - 0.05
}

/// Per sample point: (in view of the current eye, fogged irradiance, line of
/// sight). For the debug trace.
pub fn explain_observation(sight: &Sight, owner: Entity, feet: Vec3) -> Vec<(bool, f32, bool)> {
    [0.35, 1.1, 1.65]
        .map(|h| feet + Vec3::Y * h)
        .into_iter()
        .map(|point| {
            let fog = FOG.amount((point - sight.eye.position).length());
            (
                in_view(
                    sight.eye.position,
                    sight.eye.rotation,
                    sight.eye.aspect,
                    point,
                ),
                lighting::irradiance(point, sight.lights, sight.flashlight.as_ref()) * (1.0 - fog),
                line_of_sight(sight, sight.eye.position, point, owner),
            )
        })
        .collect()
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

fn next_waypoint(level: &Level, flow: &Flow, at: Vec3, goal: Vec3) -> Vec3 {
    let cell = Cell::from_world(at);
    if !level.contains(cell) || Some(cell) == flow.target || flow.distances.is_empty() {
        return goal;
    }
    let here = level.distance(&flow.distances, cell);
    let Some(next) = level
        .neighbors(cell)
        .filter(|n| level.distance(&flow.distances, *n) < here)
        .min_by_key(|n| level.distance(&flow.distances, *n))
    else {
        return goal;
    };
    // Line up with the doorway before crossing it so nobody ends a burst
    // half inside a shelf.
    let axis = (next.center() - cell.center()).normalize_or_zero();
    let offset = flat(at - cell.center());
    let lateral = offset - axis * offset.dot(axis);
    if lateral.length() > 0.45 {
        return cell.center() + axis * offset.dot(axis).max(0.0);
    }
    next.center()
}

#[allow(clippy::too_many_arguments)]
pub fn hunt(
    mannequins: Query<(Entity, &mut Mannequin, &mut Transform), Without<Player>>,
    players: Query<&Transform, With<Player>>,
    flashlights: Query<&Flashlight>,
    ceiling_lights: Query<(&Light, &Transform), (With<CeilingLight>, Without<Mannequin>)>,
    (eye, physics): (Res<Eye>, Res<PhysicsState>),
    (level, mut flow, library): (Res<CurrentLevel>, ResMut<Flow>, Res<PoseLibrary>),
    (mut game, mut caught): (ResMut<Game>, ResMut<crate::scare::Caught>),
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

    let player_cell = Cell::from_world(player_pos);
    if flow.target != Some(player_cell) && level.contains(player_cell) {
        flow.target = Some(player_cell);
        flow.distances = level.distances(player_cell);
    }

    let lights: Vec<PointLight> = ceiling_lights
        .iter()
        .filter(|(light, ..)| light.intensity > 0.0)
        .map(|(light, t)| PointLight {
            position: t.translation,
            intensity: light.intensity,
            range: light.range,
        })
        .collect();
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
        let cell = Cell::from_world(feet);
        let cells_away = if level.contains(cell) && !flow.distances.is_empty() {
            level.distance(&flow.distances, cell)
        } else {
            i32::MAX
        };
        if live && mannequin.kind == Kind::Hunter && mannequin.dormant > 0.0 {
            mannequin.dormant -= dt;
            if cells_away <= WAKE_CELLS {
                mannequin.dormant = 0.0;
            }
        }
        mannequin.observed = is_observed(&sight, entity, feet);
        if mannequin.observed
            && crate::platform::debug_flag("trace")
            && (feet - eye.position).length() > 12.0
            && rand.unit() < 0.02
        {
            log::info!(
                "trace: far observation at {feet:?} from {:?}: {:?}",
                eye.position,
                explain_observation(&sight, entity, feet)
            );
        }
        if mannequin.observed
            || !live
            || !mannequin.is_hunting()
            || crate::platform::debug_flag("nohunt")
        {
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
        if !mannequin.moving {
            // A new burst: take a new pose, scarier the closer it gets.
            mannequin.moving = true;
            let cells_away = distance / CELL;
            let menace = if cells_away < 1.2 {
                Menace::Hunting
            } else if cells_away < 3.0 {
                if rand.unit() < 0.6 {
                    Menace::Hunting
                } else {
                    Menace::Uneasy
                }
            } else if rand.unit() < 0.6 {
                Menace::Uneasy
            } else {
                Menace::Display
            };
            mannequin.wanted_pose = library.pick(menace, mannequin.pose, &mut rand);
        }

        if distance < CATCH_DISTANCE {
            if !caught.active {
                caught.catch(entity);
                game.set_phase(crate::game::Phase::Caught);
            }
            continue;
        }

        let waypoint = next_waypoint(level, &flow, feet, player_pos);
        let mut dir = flat(waypoint - feet).normalize_or_zero();
        // Keep a little apart from each other.
        for &(other, pos) in &positions {
            if other != entity {
                let away = flat(feet - pos);
                let d = away.length();
                if d < 0.8 && d > 1e-3 {
                    dir += away / d * (0.8 - d) * 2.0;
                }
            }
        }
        let speed = if cells_away > STALK_CELLS {
            mannequin.speed * STALK_SPEED
        } else {
            mannequin.speed
        };
        let step = (speed * dt).min((distance - CATCH_DISTANCE * 0.8).max(0.0));
        transform.translation = feet + dir.normalize_or_zero() * step;
        mannequin.yaw = yaw_toward(to_player);

        mannequin.creak_timer -= dt;
        if mannequin.creak_timer <= 0.0 {
            mannequin.creak_timer = rand.range(0.7, 1.4);
            let (volume, pan) = spatialize(&eye, feet + Vec3::Y);
            sounds.play_at(&mut audio, Sfx::Creak, volume * 0.9, pan);
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
        // Seen means frozen, except for the one lunging at you.
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

    #[test]
    fn view_test_matches_the_camera() {
        let rotation = Quat::IDENTITY; // looking down -Z
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
        // Just past the horizontal edge of a 16:9 view, inside the margin.
        let half_h = ((FOV_Y * 0.5).tan() * 16.0 / 9.0).atan();
        let edge = Vec3::new((half_h + 0.05).tan() * 5.0, 0.0, -5.0);
        assert!(in_view(pos, rotation, 16.0 / 9.0, edge));
        let outside = Vec3::new((half_h + 0.3).tan() * 5.0, 0.0, -5.0);
        assert!(!in_view(pos, rotation, 16.0 / 9.0, outside));
        // Right behind the shoulder, touching distance: counts as seen.
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

use concerto::{asset_id, foundation::assets::AssetId};
use std::f32::consts::PI;

use concerto::animation::graph::{AnimationGraph, AnimationNodeIndex};
use concerto::animation::player::{AnimationHandleComponent, AnimationPlayer};
use concerto::director::VirtualCamera;
use concerto::ecs::component::scene::SceneEntityRef;
use concerto::ecs::entity::hierarchy::Children;
use concerto::ecs::reflect::{ReflectComponent, ReflectSceneComponent};
use concerto::ecs::{Entity, Query, With, Without};
use concerto::foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto::foundation::transform::Transform;
use concerto::gameplay::actions::Move;
use concerto::gameplay::camera::{CameraBoom, CameraPivot, EntityFollow};
use concerto::gameplay::third_person::{
    CameraFraming, CameraStance, ShoulderSide, ThirdPersonCamera,
};
use concerto::mesh::MeshComponent;
use concerto::motion_matching::{
    Adjustment, CharacterDisplacement, CharacterLink, CharacterSimulation, Clamp, DesiredMotion,
    MotionDatabase, MotionMatchingNode, Synchronization,
};
use concerto::name::{Name, name};
use concerto::navigation::NavAgent;
use concerto::physics::body::BodyId;
use concerto::physics::collider::{Collider, ColliderOffset};
use concerto::physics::ground::GroundProbe;
use concerto::physics::rigid_body::MotionType::Dynamic;
use concerto::physics::rigid_body::{AllowedDofs, RigidBody};
use concerto::physics::shape::MeshCollider;
use concerto::procedural_animation::{
    FootLockLeg, FootLockNode, FootPlacementLeg, FootPlacementNode, IgnoredBody,
};
use concerto::scene::{scene::Scene, spawner::SceneSpawnerComponent};
use concerto::window::define_action;
use concerto::window::input::actions::{ActionMap, Shortcut};
use concerto::window::input::{GamepadButton, Input, KeyCode, MouseButton, PhysicalKey};
use concerto::{
    ecs::{CommandQueue, Component, Res, Resource},
    foundation::assets::asset_server::AssetServer,
    reflect::{Reflect, ReflectDefault},
};
use concerto_motion_matching_gizmos::MotionMatchingGizmos;
use concerto_procedural_animation_gizmos::FootLockGizmos;
use glam::{EulerRot, Quat, Vec3};

use crate::body::{AimCamera, Body, Recoil, bind_body_controls};
use crate::footsteps::FootContacts;
use crate::grid::Patrol;
use crate::retarget::{RetargetedPlayer, retargeted_character};

const CHARACTER_SCENE: AssetId = asset_id!("content/lafan1/model_skeleton/scene.gasset");
pub(crate) const DATABASE: AssetId = asset_id!("content/lafan_database.gasset");

const CAMERA_HEIGHT: f32 = 1.4;
const SHOULDER: f32 = 0.45;
const AIM_SHOULDER: f32 = 0.5;
const FOLLOW_FOVY: f32 = 0.96;
const AIM_FOVY: f32 = 0.7;
const PITCH_LIMIT: f32 = 1.05;
const CROUCH_HEIGHT: f32 = -0.45;
const CROUCH_DISTANCE: f32 = 0.85;
const AIM_DISTANCE: f32 = 0.55;

define_action!(
    /// Runs while held: either Shift, or the left stick button.
    Run
);
define_action!(
    /// Aims while held: the right mouse button, or the left trigger.
    Aim
);
define_action!(
    /// Toggles crouching: C, or the east face button.
    Crouch
);
define_action!(
    /// Swaps the camera's shoulder: Q, or the right bumper.
    SwapShoulder
);
define_action!(
    /// Toggles facing the camera: F.
    FaceCamera
);

/// Binds the demo's character actions globally; movement and look are gameplay's `Move` and `Look`.
pub(crate) fn bind_controls(actions: &mut ActionMap) {
    actions.bind_global(Run, Shortcut::key(KeyCode::ShiftLeft));
    actions.bind_global(Run, Shortcut::key(KeyCode::ShiftRight));
    actions.bind_global(Run, GamepadButton::LeftThumb);
    actions.bind_global(Aim, MouseButton::Right);
    actions.bind_global(Aim, GamepadButton::LeftTrigger);
    actions.bind_global(Crouch, Shortcut::key(KeyCode::KeyC));
    actions.bind_global(Crouch, GamepadButton::East);
    actions.bind_global(SwapShoulder, Shortcut::key(KeyCode::KeyQ));
    actions.bind_global(SwapShoulder, GamepadButton::RightBumper);
    actions.bind_global(FaceCamera, Shortcut::key(KeyCode::KeyF));
    bind_body_controls(actions);
}

/// The player capsule's size, how far behind its shoulder the camera follows, and how the visible character follows it.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct CharacterConfig {
    pub(crate) radius: f32,
    pub(crate) height: f32,
    pub(crate) camera_distance: f32,
    pub(crate) displacement: CharacterDisplacement,
}

impl CharacterConfig {
    /// The grid's character, placed on its capsule each frame.
    pub(crate) fn grid() -> Self {
        Self {
            radius: 0.3,
            height: 1.6,
            camera_distance: 2.2,
            displacement: on_the_capsule(),
        }
    }

    /// The sanatorium's character, placed on its capsule each frame.
    pub(crate) fn sanatorium() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            camera_distance: 1.9,
            displacement: on_the_capsule(),
        }
    }
}

fn on_the_capsule() -> CharacterDisplacement {
    CharacterDisplacement::Synchronization(Synchronization { data_factor: 0.0 })
}

#[derive(Component)]
pub(crate) struct Player {
    pub(crate) faces_camera: bool,
    pub(crate) crouched: bool,
    pub(crate) foot_locking: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            faces_camera: false,
            crouched: false,
            foot_locking: true,
        }
    }
}

/// The capsule a visible character links to once its skeleton spawns, and the graph that skeleton plays.
#[derive(Component)]
pub(crate) struct PendingLink {
    pub(crate) capsule: Entity,
    graph: AssetHandle<AnimationGraph>,
    recoil: Option<AnimationNodeIndex>,
}

#[derive(Component, Reflect, Default, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub(crate) struct PlayerSpawner {
    should_spawn: bool,
}

impl PlayerSpawner {
    /// A spawner that places the character on the next run.
    pub(crate) fn ready() -> Self {
        Self { should_spawn: true }
    }
}

#[derive(Component, Default, Debug, PartialEq)]
pub(crate) struct NpcSpawner {
    should_spawn: bool,
}

impl NpcSpawner {
    /// A spawner that places a patrolling NPC on the next run.
    pub(crate) fn ready() -> Self {
        Self { should_spawn: true }
    }
}

/// The scene a visible character spawns, the graph its skeleton plays, and the graph's node that plays a shot's kick, when it has one.
pub(crate) struct CharacterAssets {
    pub(crate) scene: AssetHandle<Scene>,
    pub(crate) graph: AssetHandle<AnimationGraph>,
    pub(crate) recoil: Option<AnimationNodeIndex>,
}

impl CharacterAssets {
    fn lafan1(asset_server: &AssetServer) -> Self {
        Self {
            scene: asset_server.load(CHARACTER_SCENE),
            graph: asset_server.add(character_graph(asset_server.load(DATABASE))),
            recoil: None,
        }
    }
}

/// A camera close over the right shoulder at `distance`: lower and nearer while crouched, and tight on the shoulder with a longer lens while aiming.
pub(crate) fn grounded_camera(distance: f32) -> ThirdPersonCamera {
    let follow = CameraFraming::default()
        .with_shoulder(SHOULDER)
        .with_distance(distance)
        .with_fovy(FOLLOW_FOVY)
        .with_pitch_limit(PITCH_LIMIT);
    ThirdPersonCamera::default()
        .with_follow(follow)
        .with_crouch(
            follow
                .with_height(CROUCH_HEIGHT)
                .with_distance(distance * CROUCH_DISTANCE),
        )
        .with_aim(
            follow
                .with_shoulder(AIM_SHOULDER)
                .with_distance(distance * AIM_DISTANCE)
                .with_fovy(AIM_FOVY)
                .with_sensitivity_scale(0.5)
                .with_halflife(0.1),
        )
}

fn spawn_capsule(
    placement: Transform,
    config: &CharacterConfig,
    character: CharacterAssets,
    cmd: &mut CommandQueue,
) -> Entity {
    let collider = Collider::capsule(config.height * 0.5 - config.radius, config.radius);
    let offset = ColliderOffset::bottom_origin(&collider);
    let capsule = cmd
        .spawn((
            RigidBody {
                density: 1000.0,
                allowed_dofs: AllowedDofs::TRANSLATION | AllowedDofs::ROTATION_Y,
                motion_type: Dynamic,
            },
            CharacterSimulation::default(),
            collider,
            offset,
            placement.clone(),
            GroundProbe::default(),
        ))
        .entity();

    cmd.spawn((
        placement,
        config.displacement.clone(),
        MotionMatchingGizmos::default(),
        SceneSpawnerComponent(character.scene),
        PendingLink {
            capsule,
            graph: character.graph,
            recoil: character.recoil,
        },
    ));
    capsule
}

/// Spawns the capsule and the visible character at the spawner, once the ground has its collider; the character is UAL1, retargeted, when the level has `RetargetedPlayer`, and LAFAN1's otherwise.
pub(crate) fn spawn_character(
    spawners: Query<(&Transform, &mut PlayerSpawner)>,
    pending_ground: Query<Entity, (With<MeshCollider>, With<MeshComponent>, Without<Collider>)>,
    asset_server: Res<AssetServer>,
    config: Res<CharacterConfig>,
    retargeted: Option<Res<RetargetedPlayer>>,
    mut cmd: CommandQueue,
) {
    if pending_ground.iter().next().is_some() {
        return;
    }

    for (spawn_point, mut spawner) in spawners.iter() {
        if !spawner.should_spawn {
            return;
        }

        let placement =
            Transform::from_translation_rotation(spawn_point.translation, spawn_point.rotation);
        let character = if retargeted.is_some() {
            retargeted_character(&asset_server)
        } else {
            CharacterAssets::lafan1(&asset_server)
        };
        let capsule = spawn_capsule(placement, &config, character, &mut cmd);

        let (yaw, _, _) = spawn_point.rotation.to_euler(EulerRot::YXZ);
        let mut pivot = cmd.spawn((
            CameraPivot::facing(yaw),
            spawn_point.clone(),
            EntityFollow {
                target: capsule,
                offset: Vec3::Y * CAMERA_HEIGHT,
            },
        ));
        pivot.insert(grounded_camera(config.camera_distance));
        let pivot = pivot.entity();
        let camera = cmd
            .spawn((
                VirtualCamera::new(0),
                Transform::from_translation_rotation(
                    Vec3::NEG_Z * config.camera_distance,
                    Quat::from_rotation_y(PI),
                ),
            ))
            .entity();
        cmd.insert(
            CameraBoom {
                offset: Vec3::NEG_Z * config.camera_distance,
                ..Default::default()
            },
            camera,
        );
        cmd.add_child(pivot, camera);
        cmd.insert(
            (Player::default(), Body::default(), AimCamera(camera)),
            capsule,
        );

        spawner.should_spawn = false;
    }
}

/// Spawns a patrolling NPC's capsule and visible character at each ready NPC spawner, once the ground has its collider.
pub(crate) fn spawn_npc(
    spawners: Query<(&Transform, &mut NpcSpawner)>,
    pending_ground: Query<Entity, (With<MeshCollider>, With<MeshComponent>, Without<Collider>)>,
    asset_server: Res<AssetServer>,
    config: Res<CharacterConfig>,
    mut cmd: CommandQueue,
) {
    if pending_ground.iter().next().is_some() {
        return;
    }

    for (spawn_point, mut spawner) in spawners.iter() {
        if !spawner.should_spawn {
            continue;
        }
        let placement =
            Transform::from_translation_rotation(spawn_point.translation, spawn_point.rotation);
        let capsule = spawn_capsule(
            placement,
            &config,
            CharacterAssets::lafan1(&asset_server),
            &mut cmd,
        );
        cmd.insert((NavAgent::new(), Patrol::default()), capsule);
        spawner.should_spawn = false;
    }
}

pub(crate) fn foot_placement_weight() -> Name {
    name!("foot_placement_weight")
}

pub(crate) fn foot_placement_ignore() -> Name {
    name!("foot_placement_ignore")
}

pub(crate) fn foot_lock_weight() -> Name {
    name!("foot_lock_weight")
}

fn character_graph(database: AssetHandle<MotionDatabase>) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(
            FootPlacementNode::new("Hips")
                .with_leg(FootPlacementLeg::new("LeftFoot").with_toe("LeftToe"))
                .with_leg(FootPlacementLeg::new("RightFoot").with_toe("RightToe"))
                .with_weight_param(foot_placement_weight())
                .with_ignore_param(foot_placement_ignore()),
        )
        .input(
            FootLockNode::new()
                .with_leg(FootLockLeg::new("LeftFoot").with_toe("LeftToe"))
                .with_leg(FootLockLeg::new("RightFoot").with_toe("RightToe"))
                .with_weight_param(foot_lock_weight()),
        )
        .add_input(MotionMatchingNode::new(database));
    graph
}

/// Gives each pending character's own `AnimationPlayer`, found among its descendants, the character's graph once that graph is in the store, starts its foot placement and foot locking at weight 0, adds foot lock gizmos and footstep tracking when the capsule is the `Player`'s and the `Recoil` node when the graph has one, and links the character to its capsule and skeleton.
pub(crate) fn setup_character_animations(
    players: Query<&mut AnimationPlayer>,
    characters: Query<(Entity, &PendingLink)>,
    children: Query<&Children>,
    player_capsules: Query<Entity, With<Player>>,
    graphs: Res<AssetStore<AnimationGraph>>,
    mut cmd: CommandQueue,
) {
    for (character, pending) in characters.iter() {
        if graphs.get(&pending.graph).is_none() {
            continue;
        }
        let Some(skeleton) = descendant(character, &children, |entity| {
            players.contains_entity(entity)
        }) else {
            continue;
        };
        let Some(mut player) = players.get_entity(skeleton) else {
            continue;
        };
        player.set_float_param(foot_placement_weight(), 0.0);
        player.set_float_param(foot_lock_weight(), 0.0);
        cmd.insert(
            AnimationHandleComponent::new(pending.graph.clone()),
            skeleton,
        );
        if player_capsules.contains_entity(pending.capsule) {
            cmd.insert(
                (FootLockGizmos::default(), FootContacts::default()),
                skeleton,
            );
        }
        if let Some(node) = pending.recoil {
            cmd.insert(Recoil { node }, skeleton);
        }
        cmd.insert(
            CharacterLink {
                capsule: SceneEntityRef::Entity(pending.capsule),
                skeleton: SceneEntityRef::Entity(skeleton),
            },
            character,
        );
        cmd.remove::<PendingLink>(character);
    }
}

fn descendant(
    root: Entity,
    children: &Query<&Children>,
    matches: impl Fn(Entity) -> bool,
) -> Option<Entity> {
    let mut pending: Vec<Entity> = children
        .get_entity(root)
        .map(|direct| direct.iter().copied().collect())
        .unwrap_or_default();
    while let Some(entity) = pending.pop() {
        if matches(entity) {
            return Some(entity);
        }
        if let Some(below) = children.get_entity(entity) {
            pending.extend(below.iter().copied());
        }
    }
    None
}

/// Tells each linked skeleton's foot placement to skip its capsule, to place the feet only while the capsule stands on the ground, and to lock them only then and while its player has foot locking on; writes a param only when its value changes.
pub(crate) fn feed_foot_placement(
    links: Query<&CharacterLink>,
    capsules: Query<(Option<&BodyId>, &GroundProbe, &Player)>,
    players: Query<&mut AnimationPlayer>,
) {
    for link in links.iter() {
        let (Some(capsule), Some(skeleton)) = (link.capsule.entity(), link.skeleton.entity())
        else {
            continue;
        };
        let Some(mut player) = players.get_entity(skeleton) else {
            continue;
        };
        let Some((body, probe, player_state)) = capsules.get_entity(capsule) else {
            continue;
        };
        let weight = if body.is_some() && probe.is_grounded() {
            1.0
        } else {
            0.0
        };
        let body = body.copied();
        if player
            .get_param::<IgnoredBody>(foot_placement_ignore())
            .map(|ignored| ignored.body)
            != Some(body)
        {
            player.set_param(foot_placement_ignore(), IgnoredBody { body });
        }
        if player.get_float_param(foot_placement_weight()) != Some(weight) {
            player.set_float_param(foot_placement_weight(), weight);
        }
        let lock_weight = if player_state.foot_locking {
            weight
        } else {
            0.0
        };
        if player.get_float_param(foot_lock_weight()) != Some(lock_weight) {
            player.set_float_param(foot_lock_weight(), lock_weight);
        }
    }
}

struct Speeds {
    forward: f32,
    side: f32,
    back: f32,
}

const WALK: Speeds = Speeds {
    forward: 1.75,
    side: 1.5,
    back: 1.25,
};
const RUN: Speeds = Speeds {
    forward: 4.0,
    side: 3.0,
    back: 2.5,
};

fn movement(
    actions: &ActionMap,
    input: &Input,
    yaw: f32,
    faces_camera: bool,
    current: Quat,
) -> (Vec3, Quat) {
    let push = actions.axis2d(Move, input);
    let direction =
        Quat::from_rotation_y(yaw) * Vec3::new(-push.x, 0.0, push.y).normalize_or_zero();
    let rotation = if faces_camera {
        Quat::from_rotation_y(yaw)
    } else {
        facing(direction, current)
    };
    let speeds = if actions.is_held(Run, input) {
        &RUN
    } else {
        &WALK
    };
    let local = rotation.inverse() * direction;
    let along = if local.z > 0.0 {
        speeds.forward
    } else {
        speeds.back
    };
    let velocity =
        rotation * Vec3::new(local.x * speeds.side, 0.0, local.z * along) * push.length();
    (velocity, rotation)
}

pub(crate) fn facing(velocity: Vec3, current: Quat) -> Quat {
    if velocity == Vec3::ZERO {
        current
    } else {
        Quat::from_rotation_y(velocity.x.atan2(velocity.z))
    }
}

pub(crate) fn update_movement(
    players: Query<(&mut Player, &mut DesiredMotion, &GroundProbe, Option<&Body>)>,
    pivots: Query<(&CameraPivot, Option<&ThirdPersonCamera>)>,
    input: Res<Input>,
    actions: Res<ActionMap>,
) {
    let yaw = pivots.iter().next().map_or(0.0, |(pivot, _)| pivot.yaw());
    let aiming = pivots
        .iter()
        .find_map(|(_, rig)| rig)
        .is_some_and(|rig| rig.stance == CameraStance::Aim);
    let toggled = actions.is_just_pressed(FaceCamera, &input);
    let locking_toggled = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyL));

    for (mut player, mut desired, ground, body) in players.iter() {
        if toggled {
            player.faces_camera = !player.faces_camera;
        }
        if locking_toggled {
            player.foot_locking = !player.foot_locking;
        }
        let faces_camera = player.faces_camera || aiming || body.is_some_and(|body| body.aiming);
        let (velocity, rotation) = movement(&actions, &input, yaw, faces_camera, desired.rotation);
        if ground.is_grounded() {
            desired.velocity = velocity;
        }
        desired.rotation = rotation;
    }
}

/// Toggles crouching on `Crouch` and swaps shoulders on `SwapShoulder`, and aims every rig while `Aim` is held, else crouches it while the player is crouched.
pub(crate) fn update_camera_stance(
    players: Query<&mut Player>,
    rigs: Query<&mut ThirdPersonCamera>,
    input: Res<Input>,
    actions: Res<ActionMap>,
) {
    if actions.is_just_pressed(Crouch, &input) {
        for mut player in players.iter() {
            player.crouched = !player.crouched;
        }
    }
    let crouched = players.iter().next().is_some_and(|player| player.crouched);
    let stance = if actions.is_held(Aim, &input) {
        CameraStance::Aim
    } else if crouched {
        CameraStance::Crouch
    } else {
        CameraStance::Follow
    };
    let swapped = actions.is_just_pressed(SwapShoulder, &input);

    for mut rig in rigs.iter() {
        if rig.stance != stance {
            rig.stance = stance;
        }
        if swapped {
            rig.side = match rig.side {
                ShoulderSide::Right => ShoulderSide::Left,
                ShoulderSide::Left => ShoulderSide::Right,
            };
        }
    }
}

fn next_displacement(current: &CharacterDisplacement) -> CharacterDisplacement {
    match current {
        CharacterDisplacement::Adjustment(_) => CharacterDisplacement::Clamp(Clamp::default()),
        CharacterDisplacement::Clamp(_) => {
            CharacterDisplacement::Synchronization(Synchronization { data_factor: 0.0 })
        }
        CharacterDisplacement::Synchronization(sync) if sync.data_factor == 0.0 => {
            CharacterDisplacement::Synchronization(Synchronization { data_factor: 1.0 })
        }
        CharacterDisplacement::Synchronization(_) => {
            CharacterDisplacement::Adjustment(Adjustment::default())
        }
    }
}

pub(crate) fn debug_keys(
    characters: Query<&mut MotionMatchingGizmos>,
    skeletons: Query<&mut FootLockGizmos>,
    input: Res<Input>,
) {
    let pressed = |key| input.is_just_pressed(PhysicalKey::Code(key));
    if pressed(KeyCode::Digit5) {
        for mut gizmos in skeletons.iter() {
            gizmos.locks = !gizmos.locks;
        }
    }
    for mut gizmos in characters.iter() {
        if pressed(KeyCode::Digit1) {
            gizmos.simulation = !gizmos.simulation;
        }
        if pressed(KeyCode::Digit2) {
            gizmos.query = !gizmos.query;
        }
        if pressed(KeyCode::Digit3) {
            gizmos.matched = !gizmos.matched;
        }
        if pressed(KeyCode::Digit4) {
            gizmos.history = !gizmos.history;
        }
    }
}

pub(crate) fn cycle_displacement(characters: Query<&mut CharacterDisplacement>, input: Res<Input>) {
    if !input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyM)) {
        return;
    }
    for mut displacement in characters.iter() {
        **displacement = next_displacement(&displacement);
    }
}

#[cfg(test)]
mod tests {
    use concerto::animation::clip::AnimationClip;
    use concerto::animation::graph::AnimationGraph;
    use concerto::animation::mask::BoneMask;
    use concerto::animation::player::AnimationPlayer;
    use concerto::animation::retarget::{PoseRetarget, RetargetRig};
    use concerto::app::{
        App,
        schedule_groups::{LateFixedUpdate, LateUpdate},
    };
    use concerto::asset_id;
    use concerto::ecs::component::scene::SceneEntityRef;
    use concerto::ecs::entity::hierarchy::{ChildOf, Children};
    use concerto::ecs::system::{
        executor::single_thread::SingleThreadedExecutor, schedule::Schedules,
    };
    use concerto::ecs::{Entity, IntoSystem, System, With, World};
    use concerto::foundation::{
        assets::{
            Asset, AssetId,
            asset_server::AssetServer,
            asset_store::AssetStore,
            content::{asset_record, decode_content_asset},
        },
        transform::Transform,
    };
    use concerto::gameplay::actions::bind_gameplay_actions;
    use concerto::gameplay::camera::CameraPivot;
    use concerto::gameplay::third_person::{CameraStance, ShoulderSide, ThirdPersonCamera};
    use concerto::mesh::{Mesh, MeshComponent, Skeleton};
    use concerto::motion_matching::{
        Adjustment, CharacterDisplacement, CharacterLink, CharacterSimulation, Clamp,
        DesiredMotion, MotionDatabase,
    };
    use concerto::physics::physics_state::PhysicsState;
    use concerto::physics::{
        body::BodyId,
        collider::Collider,
        ground::GroundProbe,
        plugin::PhysicsPlugin,
        rigid_body::{AllowedDofs, RigidBody},
        shape::MeshCollider,
    };
    use concerto::procedural_animation::IgnoredBody;
    use concerto::render::assets::material::StandardMaterial;
    use concerto::scene::scene::Scene;
    use concerto::window::input::actions::ActionMap;
    use concerto::window::input::{
        GamepadAxis, GamepadId, GamepadSettings, Input, KeyCode, MouseButton, PhysicalKey,
    };
    use concerto_motion_matching_gizmos::MotionMatchingGizmos;
    use concerto_procedural_animation_gizmos::FootLockGizmos;
    use glam::{Quat, Vec3};

    use super::{
        CharacterConfig, Player, PlayerSpawner, RUN, WALK, bind_controls, cycle_displacement,
        debug_keys, feed_foot_placement, foot_lock_weight, foot_placement_ignore,
        foot_placement_weight, movement, spawn_character, update_camera_stance, update_movement,
    };

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn stored<A: Asset + 'static>(world: &mut World, server: &mut AssetServer) {
        let store = AssetStore::<A>::new();
        server.register_asset(&store);
        world.insert_resource(store);
    }

    struct SpawnerWorld {
        world: World,
        spawn_point: Vec3,
    }

    fn world_with_spawner(rotation: Quat) -> SpawnerWorld {
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(PhysicsState::new());
        let mut server = AssetServer::new();
        stored::<Scene>(&mut world, &mut server);
        stored::<Mesh>(&mut world, &mut server);
        stored::<StandardMaterial>(&mut world, &mut server);
        stored::<AnimationGraph>(&mut world, &mut server);
        stored::<MotionDatabase>(&mut world, &mut server);
        stored::<RetargetRig>(&mut world, &mut server);
        stored::<Skeleton>(&mut world, &mut server);
        stored::<AnimationClip>(&mut world, &mut server);
        stored::<BoneMask>(&mut world, &mut server);
        world.insert_resource(server);
        world.insert_resource(CharacterConfig::grid());
        let spawn_point = Vec3::new(3.0, 1.5, -7.0);
        world.spawn((
            Transform::from_translation_rotation(spawn_point, rotation),
            PlayerSpawner { should_spawn: true },
        ));
        SpawnerWorld { world, spawn_point }
    }

    fn characters(world: &mut World) -> Vec<Vec3> {
        world
            .query::<&Transform, With<Player>>()
            .iter(world)
            .map(|transform| transform.translation)
            .collect()
    }

    #[test]
    fn the_character_spawns_at_its_spawner_once_the_ground_collides_with_the_camera_behind_it() {
        let SpawnerWorld {
            mut world,
            spawn_point,
        } = world_with_spawner(Quat::from_rotation_y(std::f32::consts::PI));
        let ground = world.spawn((MeshCollider, MeshComponent::default()));

        let mut system = spawn_character.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);
        assert!(characters(&mut world).is_empty());

        world.insert(Collider::capsule(2.0, 1.0), ground);
        system.run_and_apply((), &mut world);
        assert_eq!(characters(&mut world), vec![spawn_point]);
        let yaws: Vec<f32> = world
            .query::<&CameraPivot, ()>()
            .iter(&mut world)
            .map(|pivot| pivot.yaw().abs())
            .collect();
        assert_eq!(yaws.len(), 1);
        assert!((yaws[0] - std::f32::consts::PI).abs() < 1e-4);
    }

    struct FedPlayer {
        app: App,
        capsule: Entity,
        player: Entity,
    }

    impl FedPlayer {
        fn animation(&self) -> &AnimationPlayer {
            self.app
                .main()
                .world()
                .get_component_for_entity::<AnimationPlayer>(self.player)
                .unwrap()
        }

        fn weight(&self) -> Option<f32> {
            self.animation().get_float_param(foot_placement_weight())
        }

        fn lock_weight(&self) -> Option<f32> {
            self.animation().get_float_param(foot_lock_weight())
        }

        fn ignored(&self) -> Option<BodyId> {
            self.animation()
                .get_param::<IgnoredBody>(foot_placement_ignore())
                .and_then(|ignored| ignored.body)
        }

        fn body(&self) -> Option<BodyId> {
            self.app
                .main()
                .world()
                .get_component_for_entity::<BodyId>(self.capsule)
                .copied()
        }
    }

    fn fed_after(fixed_steps: usize, foot_locking: bool) -> FedPlayer {
        let mut app = App::new();
        app.register_plugin(PhysicsPlugin);
        let world = app.main_mut().world_mut();
        world.register_component::<Collider>();
        world.register_component::<Transform>();
        world.spawn((
            Collider::cuboid(50.0, 0.5, 50.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        ));
        let capsule = world.spawn((
            RigidBody {
                allowed_dofs: AllowedDofs::TRANSLATION,
                ..Default::default()
            },
            Collider::capsule(0.5, 0.3),
            Transform::from_translation(Vec3::new(0.0, 0.85, 0.0)),
            GroundProbe::default(),
            Player {
                foot_locking,
                ..Player::default()
            },
        ));
        let player = world.spawn(AnimationPlayer::new(1));
        world.spawn(CharacterLink {
            capsule: SceneEntityRef::Entity(capsule),
            skeleton: SceneEntityRef::Entity(player),
        });
        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);
        for _ in 0..fixed_steps {
            world.run_schedule(LateFixedUpdate);
        }

        run(world, feed_foot_placement);
        FedPlayer {
            app,
            capsule,
            player,
        }
    }

    #[test]
    fn grounded_feet_are_placed_and_locked_ignoring_the_capsule_and_in_the_air_they_are_not() {
        let grounded = fed_after(60, true);
        assert!(grounded.body().is_some());
        assert_eq!(grounded.ignored(), grounded.body());
        assert_eq!(grounded.weight(), Some(1.0));
        assert_eq!(grounded.lock_weight(), Some(1.0));

        assert_eq!(fed_after(60, false).lock_weight(), Some(0.0));

        let in_air = fed_after(0, true);
        assert_eq!(in_air.weight(), Some(0.0));
        assert_eq!(in_air.lock_weight(), Some(0.0));
    }

    fn committed_skeleton(path: &str) -> Skeleton {
        let file =
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
        let record = asset_record(&file, Some("skeleton/0")).unwrap();
        decode_content_asset::<Skeleton>(&record).unwrap().1
    }

    const UAL1_SKELETON: AssetId = asset_id!("content/UAL1/scene.gasset#skeleton/0");
    const LAFAN1_SKELETON: AssetId =
        asset_id!("content/lafan1/model_skeleton/scene.gasset#skeleton/0");

    fn committed_rig(path: &str) -> RetargetRig {
        let bytes =
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
        decode_content_asset::<RetargetRig>(&bytes).unwrap().1
    }

    #[test]
    fn the_committed_lafan1_and_ual1_rigs_resolve_against_their_committed_skeletons_and_retarget() {
        let lafan1 = committed_skeleton("content/lafan1/model_skeleton/scene.gasset");
        let ual1 = committed_skeleton("content/UAL1/scene.gasset");
        let lafan1_rig = committed_rig("content/lafan1/model_skeleton/rig.gasset");
        let ual1_rig = committed_rig("content/UAL1/rig.gasset");
        assert_eq!(lafan1_rig.skeleton.id(), LAFAN1_SKELETON);
        assert_eq!(ual1_rig.skeleton.id(), UAL1_SKELETON);

        let source = lafan1_rig.resolve(&lafan1, None).unwrap();
        let target = ual1_rig.resolve(&ual1, None).unwrap();
        assert_eq!(source.goals.len(), 2);
        assert_eq!(target.goals.len(), 2);
        assert!(PoseRetarget::new(&source, &lafan1, &target, &ual1).is_ok());

        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/lafan_database.gasset"),
        )
        .unwrap();
        let database = decode_content_asset::<MotionDatabase>(&bytes).unwrap().1;
        assert_eq!(database.skeleton.id(), LAFAN1_SKELETON);
    }

    fn controls() -> ActionMap {
        let mut actions = ActionMap::default();
        bind_gameplay_actions(&mut actions);
        bind_controls(&mut actions);
        actions
    }

    fn holding(keys: &[KeyCode]) -> Input {
        let mut input = Input::new();
        for key in keys {
            input.update_key_input(PhysicalKey::Code(*key), true);
        }
        input
    }

    fn pushing_the_left_stick_forward(length: f32) -> Input {
        let mut input = Input::new();
        input.set_gamepad_settings(
            GamepadSettings::default()
                .with_dead_zone(0.0)
                .with_live_zone(1.0),
        );
        let pad = GamepadId::new(0);
        input.connect_gamepad(pad, "pad");
        input.update_gamepad_axis(pad, GamepadAxis::LeftStickY, length);
        input
    }

    struct Walk {
        input: Input,
        yaw: f32,
        faces_camera: bool,
        local: Vec3,
    }

    impl Walk {
        fn keys(keys: &[KeyCode], yaw: f32, local: Vec3) -> Self {
            Self {
                input: holding(keys),
                yaw,
                faces_camera: false,
                local,
            }
        }

        fn facing_the_camera(mut self) -> Self {
            self.faces_camera = true;
            self
        }
    }

    #[test]
    fn input_walks_and_runs_relative_to_the_camera_and_turns_the_character_onto_its_path() {
        let held = Quat::from_rotation_y(-1.2);
        let rows = [
            Walk::keys(&[KeyCode::KeyW], 0.7, Vec3::Z * WALK.forward),
            Walk::keys(&[KeyCode::ArrowLeft], 0.7, Vec3::X * WALK.forward),
            Walk::keys(
                &[KeyCode::KeyW, KeyCode::KeyD],
                0.0,
                Vec3::new(-1.0, 0.0, 1.0).normalize() * WALK.forward,
            ),
            Walk::keys(
                &[KeyCode::KeyW, KeyCode::ShiftRight],
                0.0,
                Vec3::Z * RUN.forward,
            ),
            Walk {
                input: pushing_the_left_stick_forward(0.5),
                yaw: 0.0,
                faces_camera: false,
                local: Vec3::Z * WALK.forward * 0.5,
            },
            Walk::keys(&[], 0.7, Vec3::ZERO),
            Walk::keys(&[KeyCode::KeyS], 0.7, Vec3::NEG_Z * WALK.back).facing_the_camera(),
            Walk::keys(
                &[KeyCode::KeyD, KeyCode::ShiftLeft],
                0.7,
                Vec3::NEG_X * RUN.side,
            )
            .facing_the_camera(),
        ];
        for row in rows {
            let camera = Quat::from_rotation_y(row.yaw);
            let (velocity, rotation) =
                movement(&controls(), &row.input, row.yaw, row.faces_camera, held);

            assert!(
                velocity.abs_diff_eq(camera * row.local, 1e-5),
                "{velocity} vs {}",
                camera * row.local
            );
            let forward = rotation * Vec3::Z;
            if row.faces_camera {
                assert!(rotation.abs_diff_eq(camera, 1e-5));
            } else if velocity == Vec3::ZERO {
                assert_eq!(rotation, held);
            } else {
                assert!(forward.cross(velocity).length() < 1e-5);
                assert!(forward.dot(velocity) > 0.0);
            }
        }
    }

    fn rig_after(world: &mut World, input: Input) -> ThirdPersonCamera {
        world.insert_resource(input);
        run(world, update_camera_stance);
        world
            .query::<&ThirdPersonCamera, ()>()
            .iter(world)
            .next()
            .unwrap()
            .clone()
    }

    fn aiming() -> Input {
        let mut input = Input::new();
        input.update_mouse_button(MouseButton::Right, true);
        input
    }

    #[test]
    fn the_camera_crouches_aims_and_swaps_shoulders_and_aiming_turns_the_character_to_it() {
        let yaw = 0.7;
        let mut world = World::new();
        world.register_component::<Collider>();
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        world.spawn((
            Player::default(),
            RigidBody::default(),
            Collider::capsule(2.0, 1.0),
            Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Quat::IDENTITY),
            CharacterSimulation::default(),
            DesiredMotion::default(),
            GroundProbe::default(),
        ));
        world.spawn((CameraPivot::facing(yaw), ThirdPersonCamera::default()));
        world.insert_resource(controls());

        assert_eq!(
            rig_after(&mut world, holding(&[KeyCode::KeyC])).stance,
            CameraStance::Crouch
        );
        assert_eq!(rig_after(&mut world, aiming()).stance, CameraStance::Aim);
        run(&mut world, update_movement);
        let rotations: Vec<Quat> = world
            .query::<&DesiredMotion, With<Player>>()
            .iter(&mut world)
            .map(|desired| desired.rotation)
            .collect();
        assert!(rotations[0].abs_diff_eq(Quat::from_rotation_y(yaw), 1e-5));
        assert_eq!(
            rig_after(&mut world, Input::new()).stance,
            CameraStance::Crouch
        );
        assert_eq!(
            rig_after(&mut world, holding(&[KeyCode::KeyC])).stance,
            CameraStance::Follow
        );
        assert_eq!(
            rig_after(&mut world, holding(&[KeyCode::KeyQ])).side,
            ShoulderSide::Left
        );
    }

    #[test]
    fn the_number_keys_toggle_their_gizmo_layer_and_m_cycles_the_displacement_method() {
        let mut world = World::new();
        let character = world.spawn((
            MotionMatchingGizmos::default(),
            CharacterDisplacement::Adjustment(Adjustment::default()),
        ));
        let skeleton = world.spawn(FootLockGizmos::default());

        world.insert_resource(holding(&[KeyCode::Digit2]));
        run(&mut world, debug_keys);
        world.insert_resource(holding(&[KeyCode::Digit5]));
        run(&mut world, debug_keys);
        world.insert_resource(holding(&[KeyCode::KeyM]));
        run(&mut world, cycle_displacement);

        let gizmos = world
            .get_component_for_entity::<MotionMatchingGizmos>(character)
            .unwrap();
        assert!(gizmos.simulation && !gizmos.query && gizmos.matched && gizmos.history);
        assert!(
            !world
                .get_component_for_entity::<FootLockGizmos>(skeleton)
                .unwrap()
                .locks
        );
        assert_eq!(
            world.get_component_for_entity::<CharacterDisplacement>(character),
            Some(&CharacterDisplacement::Clamp(Clamp::default()))
        );
    }
}

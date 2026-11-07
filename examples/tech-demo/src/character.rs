use concerto::{asset_id, foundation::assets::AssetId};
use std::f32::consts::PI;

use concerto::animation::graph::AnimationGraph;
use concerto::animation::player::{AnimationHandleComponent, AnimationPlayer};
use concerto::color::Color;
use concerto::director::VirtualCamera;
use concerto::ecs::component::scene::SceneEntityRef;
use concerto::ecs::reflect::{ReflectComponent, ReflectSceneComponent};
use concerto::ecs::{Entity, Query, With, Without};
use concerto::foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto::foundation::transform::Transform;
use concerto::gameplay::camera::{CameraBoom, CameraPivot, EntityFollow};
use concerto::gameplay::third_person::{
    CameraFraming, CameraStance, ShoulderSide, ThirdPersonCamera,
};
use concerto::mesh::{MeshComponent, primitives::cuboid};
use concerto::motion_matching::{
    Adjustment, CharacterDisplacement, CharacterLink, CharacterSimulation, Clamp, DesiredMotion,
    MotionDatabase, MotionMatchingNode, Synchronization,
};
use concerto::name::{Name, name};
use concerto::physics::body::BodyId;
use concerto::physics::collider::{Collider, ColliderOffset};
use concerto::physics::ground::GroundProbe;
use concerto::physics::rigid_body::MotionType::Dynamic;
use concerto::physics::rigid_body::{AllowedDofs, RigidBody};
use concerto::physics::shape::MeshCollider;
use concerto::procedural_animation::{FootPlacementLeg, FootPlacementNode, IgnoredBody};
use concerto::render::{MaterialComponent, assets::material::StandardMaterial};
use concerto::scene::{scene::Scene, spawner::SceneSpawnerComponent};
use concerto::window::input::{Input, KeyCode, MouseButton, PhysicalKey};
use concerto::{
    ecs::{CommandQueue, Component, Res, Resource},
    foundation::assets::asset_server::AssetServer,
    reflect::{Reflect, ReflectDefault},
};
use concerto_motion_matching_gizmos::MotionMatchingGizmos;
use glam::{EulerRot, Quat, Vec2, Vec3};

const CHARACTER_SCENE: AssetId = asset_id!("content/lafan1/model_skeleton/scene.gasset");
const DATABASE: AssetId = asset_id!("content/lafan_database.gasset");

const WALLS: [(Vec3, Vec3); 2] = [
    (Vec3::new(0.0, 1.0, -18.0), Vec3::new(6.0, 2.0, 0.5)),
    (Vec3::new(5.0, 1.0, -28.0), Vec3::new(0.5, 2.0, 8.0)),
];
const WALL_COLOR: Color = Color::srgba(0.45, 0.45, 0.5, 1.0);

const CAMERA_HEIGHT: f32 = 1.4;

/// The player capsule's size, how far behind it the camera sits and whether walls pull it in and it has stances, and how the visible character follows it.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct CharacterConfig {
    pub(crate) radius: f32,
    pub(crate) height: f32,
    pub(crate) camera_distance: f32,
    pub(crate) camera_collision: bool,
    pub(crate) displacement: CharacterDisplacement,
}

impl CharacterConfig {
    /// The forest's character, which follows its capsule by the default method.
    pub(crate) fn forest() -> Self {
        Self {
            radius: 0.3,
            height: 1.6,
            camera_distance: 5.0,
            camera_collision: false,
            displacement: CharacterDisplacement::default(),
        }
    }

    /// The sanatorium's character, moved by its root motion alone.
    pub(crate) fn sanatorium() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            camera_distance: 3.0,
            camera_collision: true,
            displacement: CharacterDisplacement::Synchronization(Synchronization {
                data_factor: 1.0,
            }),
        }
    }
}

#[derive(Component, Default)]
pub(crate) struct Player {
    pub(crate) faces_camera: bool,
    pub(crate) crouched: bool,
}

/// The capsule a visible character links to once its skeleton spawns, and the graph that skeleton plays.
#[derive(Component)]
pub(crate) struct PendingLink {
    capsule: Entity,
    graph: AssetHandle<AnimationGraph>,
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

/// Spawns the capsule and the visible character at the spawner, once the ground has its collider.
pub(crate) fn spawn_character(
    spawners: Query<(&Transform, &mut PlayerSpawner)>,
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
            return;
        }

        let placement =
            Transform::from_translation_rotation(spawn_point.translation, spawn_point.rotation);
        let collider = Collider::capsule(config.height * 0.5 - config.radius, config.radius);
        let offset = ColliderOffset::bottom_origin(&collider);
        let capsule = cmd
            .spawn((
                Player::default(),
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

        let graph = asset_server.add(character_graph(asset_server.load(DATABASE)));
        cmd.spawn((
            placement,
            config.displacement.clone(),
            MotionMatchingGizmos::default(),
            SceneSpawnerComponent(asset_server.load::<Scene>(CHARACTER_SCENE)),
            PendingLink { capsule, graph },
        ));

        let (yaw, _, _) = spawn_point.rotation.to_euler(EulerRot::YXZ);
        let mut pivot = cmd.spawn((
            CameraPivot::facing(yaw),
            spawn_point.clone(),
            EntityFollow {
                target: capsule,
                offset: Vec3::Y * CAMERA_HEIGHT,
            },
        ));
        if config.camera_collision {
            pivot.insert(
                ThirdPersonCamera::default()
                    .with_follow(CameraFraming::default().with_distance(config.camera_distance)),
            );
        }
        pivot.add_child_with(
            (
                VirtualCamera::new(0),
                Transform::from_translation_rotation(
                    Vec3::NEG_Z * config.camera_distance,
                    Quat::from_rotation_y(PI),
                ),
            ),
            |mut camera| {
                if config.camera_collision {
                    camera.insert(CameraBoom {
                        offset: Vec3::NEG_Z * config.camera_distance,
                        ..Default::default()
                    });
                }
            },
        );

        spawner.should_spawn = false;
    }
}

fn foot_placement_weight() -> Name {
    name!("foot_placement_weight")
}

fn foot_placement_ignore() -> Name {
    name!("foot_placement_ignore")
}

fn character_graph(database: AssetHandle<MotionDatabase>) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph.result_node().with_input(
        FootPlacementNode::new("Hips")
            .with_leg(FootPlacementLeg::new("LeftFoot").with_toe("LeftToe"))
            .with_leg(FootPlacementLeg::new("RightFoot").with_toe("RightToe"))
            .with_weight_param(foot_placement_weight())
            .with_ignore_param(foot_placement_ignore()),
        |mut placement| {
            placement.with_input(
                MotionMatchingNode {
                    database,
                    ..Default::default()
                },
                |_| {},
            );
        },
    );
    graph
}

/// Spawns the forest's two test walls as static colliders.
pub(crate) fn spawn_forest_walls(asset_server: Res<AssetServer>, mut cmd: CommandQueue) {
    for (centre, size) in WALLS {
        cmd.spawn((
            Transform::from_translation(centre),
            Collider::cuboid(size.x * 0.5, size.y * 0.5, size.z * 0.5),
            MeshComponent {
                handle: asset_server.add(cuboid(size)),
            },
            MaterialComponent {
                handle: asset_server
                    .add(StandardMaterial::default().with_base_color_factor(WALL_COLOR)),
            },
        ));
    }
}

/// Gives the sole `AnimationPlayer` its character's graph once that graph is in the store, starts its foot placement at weight 0, and links the character to its capsule and skeleton.
pub(crate) fn setup_character_animations(
    players: Query<(Entity, &mut AnimationPlayer)>,
    characters: Query<(Entity, &PendingLink)>,
    graphs: Res<AssetStore<AnimationGraph>>,
    mut cmd: CommandQueue,
) {
    let Some((skeleton, mut player)) = players.iter().next() else {
        return;
    };

    for (character, pending) in characters.iter() {
        if graphs.get(&pending.graph).is_none() {
            continue;
        }
        player.set_float_param(foot_placement_weight(), 0.0);
        cmd.insert(
            AnimationHandleComponent::new(pending.graph.clone()),
            skeleton,
        );
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

/// Tells each linked skeleton's foot placement to skip its capsule, and to place the feet only while the capsule stands on the ground; writes a param only when its value changes.
pub(crate) fn feed_foot_placement(
    links: Query<&CharacterLink>,
    capsules: Query<(Option<&BodyId>, &GroundProbe)>,
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
        let Some((body, probe)) = capsules.get_entity(capsule) else {
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

const MOVEMENT_KEYS: [(KeyCode, Vec2); 8] = [
    (KeyCode::KeyW, Vec2::Y),
    (KeyCode::ArrowUp, Vec2::Y),
    (KeyCode::KeyS, Vec2::NEG_Y),
    (KeyCode::ArrowDown, Vec2::NEG_Y),
    (KeyCode::KeyA, Vec2::X),
    (KeyCode::ArrowLeft, Vec2::X),
    (KeyCode::KeyD, Vec2::NEG_X),
    (KeyCode::ArrowRight, Vec2::NEG_X),
];

fn held(input: &Input, key: KeyCode) -> bool {
    input.is_held(PhysicalKey::Code(key))
}

fn movement(input: &Input, yaw: f32, faces_camera: bool, current: Quat) -> (Vec3, Quat) {
    let keys: Vec2 = MOVEMENT_KEYS
        .iter()
        .filter(|(key, _)| held(input, *key))
        .map(|(_, direction)| *direction)
        .sum();
    let direction = Quat::from_rotation_y(yaw) * Vec3::new(keys.x, 0.0, keys.y).normalize_or_zero();
    let rotation = if faces_camera {
        Quat::from_rotation_y(yaw)
    } else {
        facing(direction, current)
    };
    let speeds = if held(input, KeyCode::ShiftLeft) || held(input, KeyCode::ShiftRight) {
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
    let velocity = rotation * Vec3::new(local.x * speeds.side, 0.0, local.z * along);
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
    players: Query<(&mut Player, &mut DesiredMotion, &GroundProbe)>,
    pivots: Query<(&CameraPivot, Option<&ThirdPersonCamera>)>,
    input: Res<Input>,
) {
    let yaw = pivots.iter().next().map_or(0.0, |(pivot, _)| pivot.yaw());
    let aiming = pivots
        .iter()
        .find_map(|(_, rig)| rig)
        .is_some_and(|rig| rig.stance == CameraStance::Aim);
    let toggled = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyF));

    for (mut player, mut desired, ground) in players.iter() {
        if toggled {
            player.faces_camera = !player.faces_camera;
        }
        let faces_camera = player.faces_camera || aiming;
        let (velocity, rotation) = movement(&input, yaw, faces_camera, desired.rotation);
        if ground.is_grounded() {
            desired.velocity = velocity;
        }
        desired.rotation = rotation;
    }
}

/// Toggles crouching on C and swaps shoulders on Q, and aims every rig while the right mouse button is held, else crouches it while the player is crouched.
pub(crate) fn update_camera_stance(
    players: Query<&mut Player>,
    rigs: Query<&mut ThirdPersonCamera>,
    input: Res<Input>,
) {
    let pressed = |key| input.is_just_pressed(PhysicalKey::Code(key));
    if pressed(KeyCode::KeyC) {
        for mut player in players.iter() {
            player.crouched = !player.crouched;
        }
    }
    let crouched = players.iter().next().is_some_and(|player| player.crouched);
    let stance = if input.is_mouse_button_held(MouseButton::Right) {
        CameraStance::Aim
    } else if crouched {
        CameraStance::Crouch
    } else {
        CameraStance::Follow
    };
    let swapped = pressed(KeyCode::KeyQ);

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

pub(crate) fn debug_keys(characters: Query<&mut MotionMatchingGizmos>, input: Res<Input>) {
    let pressed = |key| input.is_just_pressed(PhysicalKey::Code(key));
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
    use concerto::animation::graph::{AnimationGraph, AnimationNodeIndex};
    use concerto::animation::player::{AnimationHandleComponent, AnimationPlayer};
    use concerto::app::{
        App,
        schedule_groups::{LateFixedUpdate, LateUpdate},
    };
    use concerto::ecs::component::scene::SceneEntityRef;
    use concerto::ecs::system::{
        executor::single_thread::SingleThreadedExecutor, schedule::Schedules,
    };
    use concerto::ecs::{Changed, Entity, IntoSystem, System, With, Without, World};
    use concerto::foundation::{
        assets::{
            Asset, AssetId,
            asset_server::{AssetServer, handle_asset_load_events},
            asset_store::AssetStore,
            content::decode_content_asset,
            handle::AssetHandle,
        },
        transform::Transform,
    };
    use concerto::mesh::{Mesh, MeshComponent};
    use concerto::motion_matching::{
        Adjustment, CharacterDisplacement, CharacterLink, CharacterSimulation, Clamp,
        DesiredMotion, MotionDatabase, MotionMatchingNode, Synchronization,
    };
    use concerto::name::name;
    use concerto::physics::physics_state::PhysicsState;
    use concerto::physics::{
        body::BodyId,
        ground::GroundProbe,
        plugin::PhysicsPlugin,
        rigid_body::{AllowedDofs, RigidBody},
    };
    use concerto::physics::{
        collider::{Collider, ColliderOffset},
        shape::MeshCollider,
    };
    use concerto::procedural_animation::{FootPlacementLeg, FootPlacementNode, IgnoredBody};
    use concerto::reflect::{apply_value, to_value};
    use concerto::render::{MaterialComponent, assets::material::StandardMaterial};
    use concerto::scene::{
        scene::{Scene, SceneNode},
        spawner::SceneSpawnerComponent,
    };
    use concerto::window::input::{Input, KeyCode, MouseButton, PhysicalKey};
    use concerto_motion_matching_gizmos::MotionMatchingGizmos;
    use glam::{Quat, Vec3};

    use concerto::director::VirtualCamera;
    use concerto::gameplay::camera::{CameraBoom, CameraPivot};
    use concerto::gameplay::third_person::{CameraStance, ShoulderSide, ThirdPersonCamera};

    use super::{
        CHARACTER_SCENE, CharacterConfig, DATABASE, PendingLink, Player, PlayerSpawner, RUN, WALK,
        cycle_displacement, debug_keys, facing, feed_foot_placement, foot_placement_ignore,
        foot_placement_weight, movement, setup_character_animations, spawn_character,
        spawn_forest_walls, update_camera_stance, update_movement,
    };

    fn spawned(world: &mut World) {
        let mut system = spawn_character.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn capsules(world: &mut World) -> Vec<(Entity, Vec3)> {
        world
            .query::<(
                Entity,
                &Transform,
                &RigidBody,
                &CharacterSimulation,
                &Collider,
                &ColliderOffset,
                &GroundProbe,
            ), With<Player>>()
            .iter(world)
            .map(|(entity, transform, ..)| (entity, transform.translation))
            .collect()
    }

    fn visible_characters(
        world: &mut World,
    ) -> Vec<(Entity, Vec3, CharacterDisplacement, AssetId)> {
        world
            .query::<(
                Entity,
                &Transform,
                &CharacterDisplacement,
                &MotionMatchingGizmos,
                &SceneSpawnerComponent,
            ), ()>()
            .iter(world)
            .map(|(entity, transform, displacement, _, scene)| {
                (
                    entity,
                    transform.translation,
                    displacement.clone(),
                    scene.0.id(),
                )
            })
            .collect()
    }

    #[test]
    fn spawning_gives_the_capsule_and_the_character_and_no_walls() {
        let (mut world, spawn_point) = world_with_spawner();

        spawned(&mut world);

        let capsules: Vec<Vec3> = capsules(&mut world)
            .into_iter()
            .map(|(_, position)| position)
            .collect();
        assert_eq!(capsules, vec![spawn_point]);
        let characters: Vec<(Vec3, CharacterDisplacement, AssetId)> =
            visible_characters(&mut world)
                .into_iter()
                .map(|(_, position, displacement, scene)| (position, displacement, scene))
                .collect();
        assert_eq!(
            characters,
            vec![(
                spawn_point,
                CharacterDisplacement::default(),
                CHARACTER_SCENE
            )]
        );
        let walls = world
            .query::<(&Collider, &MeshComponent, &MaterialComponent, &Transform), Without<RigidBody>>()
            .iter(&mut world)
            .count();
        assert_eq!(walls, 0);
    }

    #[test]
    fn the_capsule_and_the_camera_take_their_size_from_the_character_config() {
        let (mut world, _) = world_with_spawner();
        world.insert_resource(CharacterConfig::sanatorium());

        spawned(&mut world);

        let offsets: Vec<Vec3> = world
            .query::<&ColliderOffset, With<Player>>()
            .iter(&mut world)
            .map(|offset| offset.0)
            .collect();
        assert_eq!(offsets, vec![Vec3::Y * 0.9]);
        let cameras: Vec<Vec3> = world
            .query::<&Transform, With<VirtualCamera>>()
            .iter(&mut world)
            .map(|transform| transform.translation)
            .collect();
        assert_eq!(cameras, vec![Vec3::NEG_Z * 3.0]);
    }

    fn camera_booms(world: &mut World) -> Vec<Option<CameraBoom>> {
        world
            .query::<(&VirtualCamera, Option<&CameraBoom>), ()>()
            .iter(world)
            .map(|(_, boom)| boom.cloned())
            .collect()
    }

    #[test]
    fn the_sanatorium_camera_has_a_boom_as_long_as_its_distance() {
        let (mut world, _) = world_with_spawner();
        world.insert_resource(CharacterConfig::sanatorium());

        spawned(&mut world);

        assert_eq!(
            camera_booms(&mut world),
            vec![Some(CameraBoom {
                offset: Vec3::NEG_Z * 3.0,
                ..Default::default()
            })]
        );
    }

    #[test]
    fn the_forest_camera_has_no_boom() {
        let (mut world, _) = world_with_spawner();

        spawned(&mut world);

        assert_eq!(camera_booms(&mut world), vec![None]);
    }

    fn follow_distances(world: &mut World) -> Vec<Option<f32>> {
        world
            .query::<(&CameraPivot, Option<&ThirdPersonCamera>), ()>()
            .iter(world)
            .map(|(_, rig)| rig.map(|rig| rig.follow.distance))
            .collect()
    }

    #[test]
    fn the_sanatorium_pivot_has_a_third_person_rig_that_follows_at_its_distance() {
        let (mut world, _) = world_with_spawner();
        world.insert_resource(CharacterConfig::sanatorium());

        spawned(&mut world);

        assert_eq!(follow_distances(&mut world), vec![Some(3.0)]);
    }

    #[test]
    fn the_forest_pivot_has_no_third_person_rig() {
        let (mut world, _) = world_with_spawner();

        spawned(&mut world);

        assert_eq!(follow_distances(&mut world), vec![None]);
    }

    #[test]
    fn the_character_moves_by_the_displacement_the_config_names() {
        let (mut world, _) = world_with_spawner();
        world.insert_resource(CharacterConfig::sanatorium());

        spawned(&mut world);

        let methods: Vec<CharacterDisplacement> = visible_characters(&mut world)
            .into_iter()
            .map(|(_, _, displacement, _)| displacement)
            .collect();
        assert_eq!(
            methods,
            vec![CharacterDisplacement::Synchronization(Synchronization {
                data_factor: 1.0
            })]
        );
    }

    #[test]
    fn the_camera_starts_behind_a_spawner_that_faces_away_from_the_default() {
        let (mut world, _) = world_with_spawner();
        for mut transform in world
            .query::<&mut Transform, With<PlayerSpawner>>()
            .iter(&mut world)
        {
            transform.rotation = Quat::from_rotation_y(std::f32::consts::PI);
        }

        spawned(&mut world);

        let yaws: Vec<f32> = world
            .query::<&CameraPivot, ()>()
            .iter(&mut world)
            .map(|pivot| pivot.yaw().abs())
            .collect();
        assert_eq!(yaws.len(), 1);
        assert!((yaws[0] - std::f32::consts::PI).abs() < 1e-4);
    }

    #[test]
    fn the_forest_walls_are_static_colliders() {
        let (mut world, _) = world_with_spawner();
        let mut system = spawn_forest_walls.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        let walls: Vec<Vec3> = world
            .query::<(&Transform, &Collider, &MeshComponent, &MaterialComponent), Without<RigidBody>>()
            .iter(&mut world)
            .map(|(transform, ..)| transform.translation)
            .collect();
        assert_eq!(
            walls,
            vec![Vec3::new(0.0, 1.0, -18.0), Vec3::new(5.0, 1.0, -28.0)]
        );
    }

    #[test]
    fn the_skeleton_is_linked_and_given_the_graph_once_the_graph_has_landed() {
        let (mut world, _) = world_with_spawner();
        spawned(&mut world);
        let (capsule, _) = capsules(&mut world)[0];
        let (character, ..) = visible_characters(&mut world)[0];
        let skeleton = world.spawn(AnimationPlayer::new(1));

        let mut setup = setup_character_animations.into_system();
        setup.initialize(&mut world);
        setup.run_and_apply((), &mut world);
        assert!(
            world
                .get_component_for_entity::<AnimationHandleComponent>(skeleton)
                .is_none()
        );
        assert!(
            world
                .get_component_for_entity::<CharacterLink>(character)
                .is_none()
        );

        handle_asset_load_events(&mut world);
        setup.run_and_apply((), &mut world);
        setup.run_and_apply((), &mut world);

        assert_eq!(
            world.get_component_for_entity::<CharacterLink>(character),
            Some(&CharacterLink {
                capsule: SceneEntityRef::Entity(capsule),
                skeleton: SceneEntityRef::Entity(skeleton),
            })
        );
        let handle: AssetHandle<AnimationGraph> = (**world
            .get_component_for_entity::<AnimationHandleComponent>(skeleton)
            .unwrap())
        .clone();
        let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
        let nodes: Vec<_> = graphs
            .get(&handle)
            .unwrap()
            .nodes()
            .filter(|node| node.type_name() == std::any::type_name::<MotionMatchingNode>())
            .map(|node| node.value().unwrap().into_owned())
            .collect();
        let expected = MotionMatchingNode {
            database: AssetHandle::weak(DATABASE),
            ..Default::default()
        };
        assert_eq!(nodes, vec![to_value(&expected).unwrap()]);
        assert_eq!(
            world
                .get_component_for_entity::<AnimationPlayer>(skeleton)
                .unwrap()
                .get_float_param(foot_placement_weight()),
            Some(0.0)
        );
    }

    #[test]
    fn the_character_graph_places_the_feet_on_the_motion_matching_pose() {
        let (mut world, _) = world_with_spawner();
        spawned(&mut world);
        handle_asset_load_events(&mut world);
        let handle = world
            .query::<&PendingLink, ()>()
            .iter(&mut world)
            .map(|pending| pending.graph.clone())
            .next()
            .unwrap();

        let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
        let graph = graphs.get(&handle).unwrap();
        let nodes: Vec<_> = graph.nodes().collect();
        let placements: Vec<AnimationNodeIndex> = graph
            .iter()
            .filter(|index| {
                nodes[index.index()].type_name() == std::any::type_name::<FootPlacementNode>()
            })
            .collect();
        assert_eq!(placements.len(), 1);
        let inputs: Vec<&str> = graph
            .get_node_inputs(placements[0])
            .map(|input| nodes[input.index()].type_name())
            .collect();
        assert_eq!(inputs, vec![std::any::type_name::<MotionMatchingNode>()]);
        let expected = FootPlacementNode::new("Hips")
            .with_leg(FootPlacementLeg::new("LeftFoot").with_toe("LeftToe"))
            .with_leg(FootPlacementLeg::new("RightFoot").with_toe("RightToe"))
            .with_weight_param(name!("foot_placement_weight"))
            .with_ignore_param(name!("foot_placement_ignore"));
        assert_eq!(
            nodes[placements[0].index()].value().unwrap().into_owned(),
            to_value(&expected).unwrap()
        );
    }

    struct FedPlayer {
        app: App,
        capsule: Entity,
        player: Entity,
    }

    impl FedPlayer {
        fn weight(&self) -> Option<f32> {
            self.app
                .main()
                .world()
                .get_component_for_entity::<AnimationPlayer>(self.player)
                .unwrap()
                .get_float_param(foot_placement_weight())
        }

        fn ignored(&self) -> Option<BodyId> {
            self.app
                .main()
                .world()
                .get_component_for_entity::<AnimationPlayer>(self.player)
                .unwrap()
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

    fn fed_after(fixed_steps: usize, registered: bool) -> FedPlayer {
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
        ));
        let player = world.spawn(AnimationPlayer::new(1));
        world.spawn(CharacterLink {
            capsule: SceneEntityRef::Entity(capsule),
            skeleton: SceneEntityRef::Entity(player),
        });
        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        if registered {
            world.run_schedule(LateUpdate);
        }
        for _ in 0..fixed_steps {
            world.run_schedule(LateFixedUpdate);
        }

        let mut feed = feed_foot_placement.into_system();
        feed.initialize(world);
        feed.run_and_apply((), world);
        FedPlayer {
            app,
            capsule,
            player,
        }
    }

    #[test]
    fn a_grounded_capsule_gives_foot_placement_full_weight_and_its_body_to_ignore() {
        let grounded = fed_after(60, true);

        assert!(grounded.body().is_some());
        assert_eq!(grounded.ignored(), grounded.body());
        assert_eq!(grounded.weight(), Some(1.0));

        let in_air = fed_after(0, true);

        assert!(in_air.body().is_some());
        assert_eq!(in_air.ignored(), in_air.body());
        assert_eq!(in_air.weight(), Some(0.0));
    }

    #[test]
    fn a_capsule_without_a_body_gives_foot_placement_no_weight() {
        let unregistered = fed_after(0, false);

        assert_eq!(unregistered.body(), None);
        assert_eq!(unregistered.weight(), Some(0.0));
    }

    #[test]
    fn feeding_the_same_probe_state_again_leaves_the_player_unchanged() {
        let mut fed = fed_after(60, true);
        let world = fed.app.main_mut().world_mut();
        world.tick();
        let mut feed = feed_foot_placement.into_system();
        feed.initialize(world);
        feed.run_and_apply((), world);

        let changed = world
            .query::<Entity, Changed<AnimationPlayer>>()
            .iter(world)
            .count();
        assert_eq!(changed, 0);
        assert_eq!(fed.weight(), Some(1.0));
        assert_eq!(fed.ignored(), fed.body());
    }

    fn holds<T>(node: &SceneNode) -> bool {
        node.components
            .iter()
            .any(|component| component.type_name == std::any::type_name::<T>())
    }

    #[test]
    fn the_forest_holds_a_root_spawner_and_a_ground_collider() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/forest/scene.gasset"),
        )
        .unwrap();
        let (_, scene) = decode_content_asset::<Scene>(&bytes).unwrap();

        let children: Vec<usize> = scene
            .nodes
            .iter()
            .flat_map(|node| node.children.iter().copied())
            .collect();
        let spawners: Vec<bool> = scene
            .nodes
            .iter()
            .enumerate()
            .filter(|(index, node)| holds::<PlayerSpawner>(node) && !children.contains(index))
            .flat_map(|(_, node)| &node.components)
            .filter(|component| component.type_name == std::any::type_name::<PlayerSpawner>())
            .map(|component| {
                let mut spawner = PlayerSpawner::default();
                apply_value(&mut spawner, &component.value);
                spawner.should_spawn
            })
            .collect();
        assert_eq!(spawners, vec![true]);
        assert!(
            scene
                .nodes
                .iter()
                .any(|node| node.name == "Plane" && holds::<MeshCollider>(node))
        );
    }

    fn stored<A: Asset + 'static>(world: &mut World, server: &mut AssetServer) {
        let store = AssetStore::<A>::new();
        server.register_asset(&store);
        world.insert_resource(store);
    }

    fn world_with_spawner() -> (World, Vec3) {
        let mut world = World::new();
        world.insert_resource(PhysicsState::new());
        let mut server = AssetServer::new();
        stored::<Scene>(&mut world, &mut server);
        stored::<Mesh>(&mut world, &mut server);
        stored::<StandardMaterial>(&mut world, &mut server);
        stored::<AnimationGraph>(&mut world, &mut server);
        stored::<MotionDatabase>(&mut world, &mut server);
        world.insert_resource(server);
        world.insert_resource(CharacterConfig::forest());
        let spawn_point = Vec3::new(3.0, 1.5, -7.0);
        world.spawn((
            Transform::from_translation_rotation(spawn_point, Quat::IDENTITY),
            PlayerSpawner { should_spawn: true },
        ));
        (world, spawn_point)
    }

    fn characters(world: &mut World) -> Vec<Vec3> {
        world
            .query::<&Transform, With<Player>>()
            .iter(world)
            .map(|transform| transform.translation)
            .collect()
    }

    #[test]
    fn the_character_waits_for_the_ground_collider() {
        let (mut world, spawn_point) = world_with_spawner();
        let ground = world.spawn((MeshCollider, MeshComponent::default()));

        let mut system = spawn_character.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);
        assert!(characters(&mut world).is_empty());

        world.insert(Collider::capsule(2.0, 1.0), ground);
        system.run_and_apply((), &mut world);
        assert_eq!(characters(&mut world), vec![spawn_point]);
    }

    #[test]
    fn a_mesh_collider_without_a_mesh_does_not_hold_the_character() {
        let (mut world, spawn_point) = world_with_spawner();
        world.spawn(MeshCollider);

        let mut system = spawn_character.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(characters(&mut world), vec![spawn_point]);
    }

    fn world_with_player(desired: DesiredMotion) -> World {
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
            desired,
            GroundProbe::default(),
        ));
        world
    }

    fn holding(keys: &[KeyCode]) -> Input {
        let mut input = Input::new();
        for key in keys {
            input.update_key_input(PhysicalKey::Code(*key), true);
        }
        input
    }

    fn walk(input: &Input, yaw: f32) -> Vec3 {
        movement(input, yaw, false, Quat::IDENTITY).0
    }

    #[test]
    fn facing_the_camera_holds_its_yaw_whatever_the_input() {
        let yaw = 0.7;
        let held = Quat::from_rotation_y(-1.2);
        let inputs: [&[KeyCode]; 3] = [&[], &[KeyCode::KeyD], &[KeyCode::KeyS, KeyCode::ShiftLeft]];
        for keys in inputs {
            let (_, rotation) = movement(&holding(keys), yaw, true, held);
            assert!(
                rotation.abs_diff_eq(Quat::from_rotation_y(yaw), 1e-5),
                "{keys:?}"
            );
        }
    }

    #[test]
    fn facing_the_camera_moves_at_the_side_and_back_speeds() {
        let yaw = 0.7;
        let camera = Quat::from_rotation_y(yaw);
        let cases = [
            (vec![KeyCode::KeyW], Vec3::Z * WALK.forward),
            (vec![KeyCode::KeyS], Vec3::NEG_Z * WALK.back),
            (vec![KeyCode::KeyA], Vec3::X * WALK.side),
            (
                vec![KeyCode::KeyD, KeyCode::ShiftLeft],
                Vec3::NEG_X * RUN.side,
            ),
            (
                vec![KeyCode::KeyS, KeyCode::ShiftRight],
                Vec3::NEG_Z * RUN.back,
            ),
        ];
        for (keys, local) in cases {
            let (velocity, _) = movement(&holding(&keys), yaw, true, Quat::IDENTITY);
            assert!(velocity.abs_diff_eq(camera * local, 1e-5), "{keys:?}");
        }
    }

    #[test]
    fn f_toggles_facing_the_camera() {
        let mut world = world_with_player(DesiredMotion::default());
        let faces_camera = |world: &mut World| {
            world
                .query::<&Player, ()>()
                .iter(world)
                .next()
                .unwrap()
                .faces_camera
        };

        for expected in [true, false] {
            world.insert_resource(holding(&[KeyCode::KeyF]));
            let mut system = update_movement.into_system();
            system.initialize(&mut world);
            system.run_and_apply((), &mut world);
            assert_eq!(faces_camera(&mut world), expected);
        }
    }

    #[test]
    fn aiming_turns_the_character_to_the_camera_yaw() {
        let yaw = 0.7;
        let mut world = world_with_player(DesiredMotion::default());
        world.spawn((
            CameraPivot::facing(yaw),
            ThirdPersonCamera::default().with_stance(CameraStance::Aim),
        ));
        world.insert_resource(Input::new());

        let mut system = update_movement.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        let players: Vec<bool> = world
            .query::<&Player, ()>()
            .iter(&mut world)
            .map(|player| player.faces_camera)
            .collect();
        assert_eq!(players, vec![false]);
        let rotations: Vec<Quat> = world
            .query::<&DesiredMotion, With<Player>>()
            .iter(&mut world)
            .map(|desired| desired.rotation)
            .collect();
        assert_eq!(rotations.len(), 1);
        assert!(rotations[0].abs_diff_eq(Quat::from_rotation_y(yaw), 1e-5));
    }

    fn world_with_rig() -> World {
        let mut world = World::new();
        world.spawn(Player::default());
        world.spawn(ThirdPersonCamera::default());
        world
    }

    fn aiming(keys: &[KeyCode]) -> Input {
        let mut input = holding(keys);
        input.update_mouse_button(MouseButton::Right, true);
        input
    }

    fn rig_after(world: &mut World, input: Input) -> ThirdPersonCamera {
        world.insert_resource(input);
        let mut system = update_camera_stance.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
        world
            .query::<&ThirdPersonCamera, ()>()
            .iter(world)
            .next()
            .unwrap()
            .clone()
    }

    #[test]
    fn holding_the_right_mouse_button_aims() {
        let mut world = world_with_rig();

        assert_eq!(rig_after(&mut world, aiming(&[])).stance, CameraStance::Aim);
        assert_eq!(
            rig_after(&mut world, Input::new()).stance,
            CameraStance::Follow
        );
    }

    #[test]
    fn c_toggles_the_crouch_stance() {
        let mut world = world_with_rig();

        assert_eq!(
            rig_after(&mut world, holding(&[KeyCode::KeyC])).stance,
            CameraStance::Crouch
        );
        assert_eq!(
            rig_after(&mut world, holding(&[KeyCode::KeyC])).stance,
            CameraStance::Follow
        );
    }

    #[test]
    fn aiming_while_crouched_aims_and_releasing_crouches_again() {
        let mut world = world_with_rig();
        rig_after(&mut world, holding(&[KeyCode::KeyC]));

        assert_eq!(rig_after(&mut world, aiming(&[])).stance, CameraStance::Aim);
        assert_eq!(
            rig_after(&mut world, Input::new()).stance,
            CameraStance::Crouch
        );
    }

    #[test]
    fn q_swaps_the_shoulder() {
        let mut world = world_with_rig();

        assert_eq!(
            rig_after(&mut world, holding(&[KeyCode::KeyQ])).side,
            ShoulderSide::Left
        );
        assert_eq!(
            rig_after(&mut world, holding(&[KeyCode::KeyQ])).side,
            ShoulderSide::Right
        );
    }

    #[test]
    fn arrow_input_maps_to_a_world_velocity_at_player_speed() {
        assert!(walk(&holding(&[KeyCode::ArrowUp]), 0.0).abs_diff_eq(Vec3::Z * WALK.forward, 1e-5));
        assert!(
            walk(&holding(&[KeyCode::ArrowRight]), 0.0)
                .abs_diff_eq(Vec3::NEG_X * WALK.forward, 1e-5)
        );
        assert_eq!(walk(&Input::new(), 0.0), Vec3::ZERO);
    }

    #[test]
    fn wasd_and_the_arrows_move_the_same_way_relative_to_the_camera_yaw() {
        let yaw = 0.7;
        let keys = [
            (KeyCode::KeyW, KeyCode::ArrowUp, Vec3::Z),
            (KeyCode::KeyS, KeyCode::ArrowDown, Vec3::NEG_Z),
            (KeyCode::KeyA, KeyCode::ArrowLeft, Vec3::X),
            (KeyCode::KeyD, KeyCode::ArrowRight, Vec3::NEG_X),
        ];
        for (letter, arrow, direction) in keys {
            let expected = Quat::from_rotation_y(yaw) * direction * WALK.forward;
            assert!(walk(&holding(&[letter]), yaw).abs_diff_eq(expected, 1e-5));
            assert!(walk(&holding(&[arrow]), yaw).abs_diff_eq(expected, 1e-5));
        }
    }

    #[test]
    fn a_diagonal_is_no_faster_than_a_straight_line() {
        let velocity = walk(&holding(&[KeyCode::KeyW, KeyCode::KeyD]), 0.0);

        assert!(velocity.abs_diff_eq(Vec3::new(-1.0, 0.0, 1.0).normalize() * WALK.forward, 1e-5));
    }

    #[test]
    fn shift_runs() {
        for shift in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
            let velocity = walk(&holding(&[KeyCode::KeyW, shift]), 0.0);
            assert!(velocity.abs_diff_eq(Vec3::Z * RUN.forward, 1e-5));
        }
    }

    #[test]
    fn the_desired_rotation_turns_forward_onto_the_velocity_and_holds_without_input() {
        let held = Quat::from_rotation_y(0.3);
        for velocity in [
            Vec3::Z,
            Vec3::NEG_Z,
            Vec3::X * RUN.forward,
            Vec3::new(-1.0, 0.0, -2.0),
        ] {
            let forward = facing(velocity, held) * Vec3::Z;
            assert!(forward.cross(velocity).length() < 1e-5);
            assert!(forward.dot(velocity) > 0.0);
        }
        assert_eq!(facing(Vec3::ZERO, held), held);
    }

    fn debugged_character(world: &mut World) -> Entity {
        world.spawn((
            MotionMatchingGizmos::default(),
            CharacterDisplacement::Adjustment(Adjustment::default()),
        ))
    }

    fn pressed<M>(world: &mut World, key: KeyCode, system: impl IntoSystem<(), M>) {
        world.insert_resource(holding(&[key]));
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn displacement(world: &World, character: Entity) -> CharacterDisplacement {
        world
            .get_component_for_entity::<CharacterDisplacement>(character)
            .unwrap()
            .clone()
    }

    fn layers(world: &World, character: Entity) -> [bool; 4] {
        let gizmos = world
            .get_component_for_entity::<MotionMatchingGizmos>(character)
            .unwrap();
        [
            gizmos.simulation,
            gizmos.query,
            gizmos.matched,
            gizmos.history,
        ]
    }

    #[test]
    fn each_number_key_toggles_its_gizmo_layer() {
        let keys = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
        ];
        for (layer, key) in keys.into_iter().enumerate() {
            let mut world = World::new();
            let character = debugged_character(&mut world);

            pressed(&mut world, key, debug_keys);
            let mut expected = [true; 4];
            expected[layer] = false;
            assert_eq!(layers(&world, character), expected);

            pressed(&mut world, key, debug_keys);
            assert_eq!(layers(&world, character), [true; 4]);
        }
    }

    #[test]
    fn m_cycles_the_displacement_method() {
        let mut world = World::new();
        let character = debugged_character(&mut world);

        let methods: Vec<CharacterDisplacement> = (0..4)
            .map(|_| {
                pressed(&mut world, KeyCode::KeyM, cycle_displacement);
                displacement(&world, character)
            })
            .collect();

        assert_eq!(
            methods,
            vec![
                CharacterDisplacement::Clamp(Clamp::default()),
                CharacterDisplacement::Synchronization(Synchronization { data_factor: 0.0 }),
                CharacterDisplacement::Synchronization(Synchronization { data_factor: 1.0 }),
                CharacterDisplacement::Adjustment(Adjustment::default()),
            ]
        );
    }

    #[test]
    fn the_gizmo_keys_leave_the_displacement_method_alone() {
        let mut world = World::new();
        let character = debugged_character(&mut world);

        pressed(&mut world, KeyCode::KeyM, debug_keys);

        assert_eq!(
            displacement(&world, character),
            CharacterDisplacement::Adjustment(Adjustment::default())
        );
    }

    #[test]
    fn input_in_the_air_does_not_change_the_walk() {
        let mut world = world_with_player(DesiredMotion::default());
        world.insert_resource(holding(&[KeyCode::ArrowUp]));

        let mut system = update_movement.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        let velocities: Vec<Vec3> = world
            .query::<&DesiredMotion, With<Player>>()
            .iter(&mut world)
            .map(|desired| desired.velocity)
            .collect();
        assert_eq!(velocities, vec![Vec3::ZERO]);
    }
}

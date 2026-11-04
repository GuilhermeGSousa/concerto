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
use concerto::gameplay::camera::{CameraPivot, EntityFollow};
use concerto::mesh::{MeshComponent, primitives::cuboid};
use concerto::motion_matching::{
    Adjustment, CharacterDisplacement, CharacterLink, CharacterSimulation, Clamp, DesiredMotion,
    MotionMatchingNode, Synchronization,
};
use concerto::physics::collider::{Collider, ColliderOffset};
use concerto::physics::ground::GroundProbe;
use concerto::physics::rigid_body::MotionType::Dynamic;
use concerto::physics::rigid_body::{AllowedDofs, RigidBody};
use concerto::physics::shape::MeshCollider;
use concerto::render::{MaterialComponent, assets::material::StandardMaterial};
use concerto::scene::{scene::Scene, spawner::SceneSpawnerComponent};
use concerto::window::input::{Input, KeyCode, PhysicalKey};
use concerto::{
    ecs::{CommandQueue, Component, Res, Resource},
    foundation::assets::asset_server::AssetServer,
    reflect::{Reflect, ReflectDefault},
};
use concerto_motion_matching_gizmos::MotionMatchingGizmos;
use glam::{EulerRot, Quat, Vec2, Vec3};

const CHARACTER_SCENE: AssetId = asset_id!("content/lafan1/model_skeleton/scene.gasset");
const DATABASE: AssetId = asset_id!("content/lafan_database_baked.gasset");

const WALLS: [(Vec3, Vec3); 2] = [
    (Vec3::new(0.0, 1.0, -18.0), Vec3::new(6.0, 2.0, 0.5)),
    (Vec3::new(5.0, 1.0, -28.0), Vec3::new(0.5, 2.0, 8.0)),
];
const WALL_COLOR: Color = Color::srgba(0.45, 0.45, 0.5, 1.0);

const CAMERA_HEIGHT: f32 = 1.4;

/// The player capsule's size and how far behind it the camera sits.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct CharacterConfig {
    pub(crate) radius: f32,
    pub(crate) height: f32,
    pub(crate) camera_distance: f32,
}

impl CharacterConfig {
    pub(crate) const FOREST: Self = Self {
        radius: 0.3,
        height: 1.6,
        camera_distance: 5.0,
    };
    pub(crate) const SANATORIUM: Self = Self {
        radius: 0.35,
        height: 1.8,
        camera_distance: 3.0,
    };
}

#[derive(Component, Default)]
pub(crate) struct Player {
    pub(crate) faces_camera: bool,
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

        let graph = asset_server.add(AnimationGraph::from_node(MotionMatchingNode {
            database: asset_server.load(DATABASE),
            ..Default::default()
        }));
        cmd.spawn((
            placement,
            CharacterDisplacement::default(),
            MotionMatchingGizmos::default(),
            SceneSpawnerComponent(asset_server.load::<Scene>(CHARACTER_SCENE)),
            PendingLink { capsule, graph },
        ));

        let (yaw, _, _) = spawn_point.rotation.to_euler(EulerRot::YXZ);
        cmd.spawn((
            CameraPivot::facing(yaw),
            spawn_point.clone(),
            EntityFollow {
                target: capsule,
                offset: Vec3::Y * CAMERA_HEIGHT,
            },
        ))
        .add_child((
            VirtualCamera::new(0),
            Transform::from_translation_rotation(
                Vec3::NEG_Z * config.camera_distance,
                Quat::from_rotation_y(PI),
            ),
        ));

        spawner.should_spawn = false;
    }
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

/// Gives the sole `AnimationPlayer` its character's graph once that graph is in the store, and links the character to its capsule and skeleton.
pub(crate) fn setup_character_animations(
    players: Query<Entity, With<AnimationPlayer>>,
    characters: Query<(Entity, &PendingLink)>,
    graphs: Res<AssetStore<AnimationGraph>>,
    mut cmd: CommandQueue,
) {
    let Some(skeleton) = players.iter().next() else {
        return;
    };

    for (character, pending) in characters.iter() {
        if graphs.get(&pending.graph).is_none() {
            continue;
        }
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
    pivots: Query<&CameraPivot>,
    input: Res<Input>,
) {
    let yaw = pivots.iter().next().map_or(0.0, |pivot| pivot.yaw());
    let toggled = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyF));

    for (mut player, mut desired, ground) in players.iter() {
        if toggled {
            player.faces_camera = !player.faces_camera;
        }
        let (velocity, rotation) = movement(&input, yaw, player.faces_camera, desired.rotation);
        if ground.is_grounded() {
            desired.velocity = velocity;
        }
        desired.rotation = rotation;
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
    characters: Query<(&mut MotionMatchingGizmos, &mut CharacterDisplacement)>,
    input: Res<Input>,
) {
    let pressed = |key| input.is_just_pressed(PhysicalKey::Code(key));
    for (mut gizmos, mut displacement) in characters.iter() {
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
        if pressed(KeyCode::KeyM) {
            **displacement = next_displacement(&displacement);
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto::animation::graph::AnimationGraph;
    use concerto::animation::player::{AnimationHandleComponent, AnimationPlayer};
    use concerto::ecs::component::scene::SceneEntityRef;
    use concerto::ecs::{Entity, IntoSystem, System, With, Without, World};
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
    use concerto::physics::physics_state::PhysicsState;
    use concerto::physics::{
        collider::{Collider, ColliderOffset},
        shape::MeshCollider,
    };
    use concerto::physics::{ground::GroundProbe, rigid_body::RigidBody};
    use concerto::reflect::{apply_value, to_value};
    use concerto::render::{MaterialComponent, assets::material::StandardMaterial};
    use concerto::scene::{
        scene::{Scene, SceneNode},
        spawner::SceneSpawnerComponent,
    };
    use concerto::window::input::{Input, KeyCode, PhysicalKey};
    use concerto_motion_matching_gizmos::MotionMatchingGizmos;
    use glam::{Quat, Vec3};

    use concerto::director::VirtualCamera;
    use concerto::gameplay::camera::CameraPivot;

    use super::{
        CHARACTER_SCENE, CharacterConfig, DATABASE, Player, PlayerSpawner, RUN, WALK, debug_keys,
        facing, movement, setup_character_animations, spawn_character, spawn_forest_walls,
        update_movement,
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
        world.insert_resource(CharacterConfig::SANATORIUM);

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
        world.insert_resource(CharacterConfig::FOREST);
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
            CharacterDisplacement::default(),
        ))
    }

    fn pressed(world: &mut World, key: KeyCode) {
        world.insert_resource(holding(&[key]));
        let mut system = debug_keys.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
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

            pressed(&mut world, key);
            let mut expected = [true; 4];
            expected[layer] = false;
            assert_eq!(layers(&world, character), expected);

            pressed(&mut world, key);
            assert_eq!(layers(&world, character), [true; 4]);
        }
    }

    #[test]
    fn m_cycles_the_displacement_method() {
        let mut world = World::new();
        let character = debugged_character(&mut world);

        let methods: Vec<CharacterDisplacement> = (0..4)
            .map(|_| {
                pressed(&mut world, KeyCode::KeyM);
                world
                    .get_component_for_entity::<CharacterDisplacement>(character)
                    .unwrap()
                    .clone()
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

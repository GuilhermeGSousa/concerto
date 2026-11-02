use concerto::{asset_id, foundation::assets::AssetId};
use std::f32::consts::PI;

use concerto::animation::graph::AnimationGraph;
use concerto::animation::player::{AnimationHandleComponent, AnimationPlayer};
use concerto::color::Color;
use concerto::director::VirtualCamera;
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
    ecs::{CommandQueue, Component, Res},
    foundation::assets::asset_server::AssetServer,
    reflect::{Reflect, ReflectDefault},
};
use concerto_motion_matching_gizmos::MotionMatchingGizmos;
use glam::{Quat, Vec2, Vec3};

const CHARACTER_SCENE: AssetId = asset_id!("content/lafan1/model_skeleton/scene.gasset");
const DATABASE: AssetId = asset_id!("content/lafan_database_baked.gasset");

const WALLS: [(Vec3, Vec3); 2] = [
    (Vec3::new(0.0, 1.0, -18.0), Vec3::new(6.0, 2.0, 0.5)),
    (Vec3::new(5.0, 1.0, -28.0), Vec3::new(0.5, 2.0, 8.0)),
];
const WALL_COLOR: Color = Color::srgba(0.45, 0.45, 0.5, 1.0);

const CAMERA_HEIGHT: f32 = 1.4;
const CAMERA_DISTANCE: f32 = 5.0;

#[derive(Component)]
pub(crate) struct Player;

/// The capsule a visible character links to once its skeleton spawns, and the graph that skeleton plays.
#[derive(Component)]
pub(crate) struct PendingLink {
    capsule: Entity,
    graph: AssetHandle<AnimationGraph>,
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default, SceneComponent)]
pub(crate) struct PlayerSpawner {
    should_spawn: bool,
}

/// Spawns the capsule, the visible character and the two walls at the spawner, once the ground has its collider.
pub(crate) fn spawn_character(
    spawners: Query<(&Transform, &mut PlayerSpawner)>,
    pending_ground: Query<Entity, (With<MeshCollider>, With<MeshComponent>, Without<Collider>)>,
    asset_server: Res<AssetServer>,
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
        let collider = Collider::capsule(0.5, 0.3);
        let offset = ColliderOffset::bottom_origin(&collider);
        let capsule = cmd
            .spawn((
                Player,
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

        cmd.spawn((
            CameraPivot::default(),
            spawn_point.clone(),
            EntityFollow {
                target: capsule,
                offset: Vec3::Y * CAMERA_HEIGHT,
            },
        ))
        .add_child((
            VirtualCamera::new(0),
            Transform::from_translation_rotation(
                Vec3::NEG_Z * CAMERA_DISTANCE,
                Quat::from_rotation_y(PI),
            ),
        ));

        spawner.should_spawn = false;
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
                capsule: pending.capsule,
                skeleton,
            },
            character,
        );
        cmd.remove::<PendingLink>(character);
    }
}

const WALK_SPEED: f32 = 1.75;
const RUN_SPEED: f32 = 4.0;

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

fn desired_velocity(input: &Input, yaw: f32) -> Vec3 {
    let direction: Vec2 = MOVEMENT_KEYS
        .iter()
        .filter(|(key, _)| held(input, *key))
        .map(|(_, direction)| *direction)
        .sum();
    let speed = if held(input, KeyCode::ShiftLeft) || held(input, KeyCode::ShiftRight) {
        RUN_SPEED
    } else {
        WALK_SPEED
    };
    Quat::from_rotation_y(yaw)
        * Vec3::new(direction.x, 0.0, direction.y).normalize_or_zero()
        * speed
}

fn facing(velocity: Vec3, current: Quat) -> Quat {
    if velocity == Vec3::ZERO {
        current
    } else {
        Quat::from_rotation_y(velocity.x.atan2(velocity.z))
    }
}

pub(crate) fn update_movement(
    movement: Query<(&mut DesiredMotion, &GroundProbe), With<Player>>,
    pivots: Query<&CameraPivot>,
    input: Res<Input>,
) {
    let yaw = pivots.iter().next().map_or(0.0, |pivot| pivot.yaw());
    let velocity = desired_velocity(&input, yaw);

    for (mut desired, ground) in movement.iter() {
        if ground.is_grounded() {
            desired.velocity = velocity;
        }
        desired.rotation = facing(velocity, desired.rotation);
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
    use concerto::ecs::{Entity, IntoSystem, System, With, Without, World};
    use concerto::foundation::{
        assets::{
            Asset, AssetId,
            asset_server::{AssetServer, handle_asset_load_events},
            asset_store::AssetStore,
            content::read_content_asset,
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

    use super::{
        CHARACTER_SCENE, DATABASE, Player, PlayerSpawner, RUN_SPEED, WALK_SPEED, debug_keys,
        desired_velocity, facing, setup_character_animations, spawn_character, update_movement,
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
    fn spawning_gives_the_capsule_the_character_and_two_walls() {
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
        assert_eq!(walls, 2);
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
            Some(&CharacterLink { capsule, skeleton })
        );
        let handle: AssetHandle<AnimationGraph> = (**world
            .get_component_for_entity::<AnimationHandleComponent>(skeleton)
            .unwrap())
        .clone();
        let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
        let nodes: Vec<_> = graphs
            .get(&handle)
            .unwrap()
            .stored_nodes()
            .filter(|(name, _)| *name == std::any::type_name::<MotionMatchingNode>())
            .map(|(_, value)| value.clone())
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
        let (_, payload) = read_content_asset(&bytes).unwrap();
        let scene: Scene = bincode::deserialize(payload).unwrap();

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
            Player,
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

    #[test]
    fn arrow_input_maps_to_a_world_velocity_at_player_speed() {
        assert!(
            desired_velocity(&holding(&[KeyCode::ArrowUp]), 0.0)
                .abs_diff_eq(Vec3::Z * WALK_SPEED, 1e-5)
        );
        assert!(
            desired_velocity(&holding(&[KeyCode::ArrowRight]), 0.0)
                .abs_diff_eq(Vec3::NEG_X * WALK_SPEED, 1e-5)
        );
        assert_eq!(desired_velocity(&Input::new(), 0.0), Vec3::ZERO);
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
            let expected = Quat::from_rotation_y(yaw) * direction * WALK_SPEED;
            assert!(desired_velocity(&holding(&[letter]), yaw).abs_diff_eq(expected, 1e-5));
            assert!(desired_velocity(&holding(&[arrow]), yaw).abs_diff_eq(expected, 1e-5));
        }
    }

    #[test]
    fn a_diagonal_is_no_faster_than_a_straight_line() {
        let velocity = desired_velocity(&holding(&[KeyCode::KeyW, KeyCode::KeyD]), 0.0);

        assert!(velocity.abs_diff_eq(Vec3::new(-1.0, 0.0, 1.0).normalize() * WALK_SPEED, 1e-5));
    }

    #[test]
    fn shift_runs() {
        for shift in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
            let velocity = desired_velocity(&holding(&[KeyCode::KeyW, shift]), 0.0);
            assert!(velocity.abs_diff_eq(Vec3::Z * RUN_SPEED, 1e-5));
        }
    }

    #[test]
    fn the_desired_rotation_turns_forward_onto_the_velocity_and_holds_without_input() {
        let held = Quat::from_rotation_y(0.3);
        for velocity in [
            Vec3::Z,
            Vec3::NEG_Z,
            Vec3::X * RUN_SPEED,
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
    fn input_in_the_air_does_not_change_the_desired_velocity() {
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

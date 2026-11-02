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
    CharacterDisplacement, CharacterLink, CharacterSimulation, DesiredMotion, MotionMatchingNode,
};
use concerto::physics::body::BodyId;
use concerto::physics::collider::{Collider, ColliderOffset};
use concerto::physics::ground::GroundProbe;
use concerto::physics::physics_state::PhysicsState;
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
                offset: Vec3::Y,
            },
        ))
        .add_child((
            VirtualCamera::new(0),
            Transform::from_translation_rotation(Vec3::NEG_Z * 10.0, Quat::from_rotation_y(PI)),
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

/// Locks the character's yaw to the camera's so it always faces away from the
/// camera, into the screen — movement input below is world-axis, not
/// camera-relative, so this is the only thing that turns the model.
///
/// No extra flip needed: the camera child is spawned rotated 180°
/// (`spawn_character`) to look back at the pivot it orbits, and the model's
/// own rest pose already faces the opposite way from the engine's generic
/// `-Z`-forward convention, so the two flips cancel out.
pub(crate) fn face_camera_direction(
    players: Query<(&mut Transform, &mut DesiredMotion), With<Player>>,
    pivots: Query<&CameraPivot>,
) {
    let Some(pivot) = pivots.iter().next() else {
        return;
    };

    let facing = Quat::from_rotation_y(pivot.yaw());
    for (mut transform, mut desired) in players.iter() {
        transform.rotation = facing;
        desired.rotation = facing;
    }
}

const PLAYER_SPEED: f32 = 5.0;

fn desired_velocity(player_input: Vec2) -> Vec3 {
    Vec3::new(player_input.x, 0.0, player_input.y) * PLAYER_SPEED
}

pub(crate) fn update_movement(
    movement: Query<(&mut DesiredMotion, &GroundProbe, &BodyId), With<Player>>,
    anim_players: Query<&mut AnimationPlayer>,
    input: Res<Input>,
    physics: Res<PhysicsState>,
) {
    let mut player_input = Vec2::ZERO;
    if input.is_held(PhysicalKey::Code(KeyCode::ArrowUp)) {
        player_input += Vec2::Y;
    }

    if input.is_held(PhysicalKey::Code(KeyCode::ArrowDown)) {
        player_input -= Vec2::Y;
    }

    if input.is_held(PhysicalKey::Code(KeyCode::ArrowRight)) {
        player_input -= Vec2::X;
    }

    if input.is_held(PhysicalKey::Code(KeyCode::ArrowLeft)) {
        player_input += Vec2::X;
    }

    for (mut desired, ground, body_id) in movement.iter() {
        // One skinned character, so the sole AnimationPlayer (on a spawned
        // descendant of the Player entity) is this character's.
        let Some(mut animation_player) = anim_players.iter().next() else {
            continue;
        };

        let current_vel = physics.linear_velocity(*body_id);

        animation_player.set_vec2_param(
            "movement",
            Vec2::new(-current_vel.x, current_vel.z) / PLAYER_SPEED,
        );

        if ground.is_grounded() {
            desired.velocity = desired_velocity(player_input);
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
    use concerto::gameplay::camera::CameraPivot;
    use concerto::mesh::{Mesh, MeshComponent};
    use concerto::motion_matching::{
        CharacterDisplacement, CharacterLink, CharacterSimulation, DesiredMotion, MotionDatabase,
        MotionMatchingNode,
    };
    use concerto::physics::body::BodyId;
    use concerto::physics::physics_state::PhysicsState;
    use concerto::physics::{
        collider::register_colliders, ground::GroundProbe, rigid_body::RigidBody,
    };
    use concerto::physics::{
        collider::{Collider, ColliderOffset},
        shape::MeshCollider,
    };
    use concerto::reflect::{apply_value, to_value};
    use concerto::render::{MaterialComponent, assets::material::StandardMaterial};
    use concerto::scene::{
        scene::{Scene, SceneNode},
        spawner::SceneSpawnerComponent,
    };
    use concerto::window::input::{Input, KeyCode, PhysicalKey};
    use concerto_motion_matching_gizmos::MotionMatchingGizmos;
    use glam::{Quat, Vec2, Vec3};

    use super::{
        CHARACTER_SCENE, DATABASE, PLAYER_SPEED, Player, PlayerSpawner, desired_velocity,
        face_camera_direction, setup_character_animations, spawn_character, update_movement,
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
        world.spawn(AnimationPlayer::new(1));
        let mut registration = register_colliders.into_system();
        registration.initialize(&mut world);
        registration.run_and_apply((), &mut world);
        world
    }

    fn holding_arrow_up() -> Input {
        let mut input = Input::new();
        input.update_key_input(PhysicalKey::Code(KeyCode::ArrowUp), true);
        input
    }

    #[test]
    fn arrow_input_maps_to_a_world_velocity_at_player_speed() {
        assert_eq!(
            desired_velocity(Vec2::new(-1.0, 1.0)),
            Vec3::new(-PLAYER_SPEED, 0.0, PLAYER_SPEED)
        );
        assert_eq!(desired_velocity(Vec2::ZERO), Vec3::ZERO);
    }

    #[test]
    fn the_movement_parameter_reads_the_body_velocity() {
        let mut world = world_with_player(DesiredMotion::default());
        world.insert_resource(Input::new());
        let body = world
            .query::<&BodyId, With<Player>>()
            .iter(&mut world)
            .next()
            .copied()
            .unwrap();
        world
            .get_resource_mut::<PhysicsState>()
            .unwrap()
            .set_linear_velocity(body, Vec3::new(2.5, 0.0, 1.0));

        let mut system = update_movement.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        let parameters: Vec<Option<Vec2>> = world
            .query::<&AnimationPlayer, ()>()
            .iter(&mut world)
            .map(|player| player.get_vec2_param("movement"))
            .collect();
        assert_eq!(parameters, vec![Some(Vec2::new(-2.5, 1.0) / PLAYER_SPEED)]);
    }

    #[test]
    fn input_in_the_air_does_not_change_the_desired_velocity() {
        let mut world = world_with_player(DesiredMotion::default());
        world.insert_resource(holding_arrow_up());

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

    #[test]
    fn the_desired_rotation_follows_the_camera_yaw() {
        let desired = DesiredMotion {
            velocity: Vec3::ZERO,
            rotation: Quat::from_rotation_y(1.0),
        };
        let mut world = world_with_player(desired);
        world.spawn(CameraPivot::default());

        let mut system = face_camera_direction.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        let rotations: Vec<Quat> = world
            .query::<&DesiredMotion, With<Player>>()
            .iter(&mut world)
            .map(|desired| desired.rotation)
            .collect();
        assert_eq!(rotations, vec![Quat::from_rotation_y(0.0)]);
    }
}

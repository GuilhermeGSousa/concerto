use concerto::{asset_id, foundation::assets::AssetId};
use std::f32::consts::PI;

use concerto::animation::clip::AnimationClip;
use concerto::animation::graph::AnimationGraph;
use concerto::animation::node::AnimationClipNode;
use concerto::animation::node::AnimationPlayMode::PlayOnce;
use concerto::animation::node::state_machine::{AnimationFSMTrigger, AnimationStateMachine};
use concerto::animation::player::{AnimationHandleComponent, AnimationPlayer};
use concerto::director::VirtualCamera;
use concerto::ecs::reflect::{ReflectComponent, ReflectSceneComponent};
use concerto::ecs::{Entity, Query, ResMut, With, Without};
use concerto::foundation::transform::Transform;
use concerto::gameplay::camera::{CameraPivot, EntityFollow};
use concerto::mesh::MeshComponent;
use concerto::motion_matching::{CharacterSimulation, DesiredMotion};
use concerto::physics::body::BodyId;
use concerto::physics::collider::{Collider, ColliderOffset};
use concerto::physics::ground::GroundProbe;
use concerto::physics::physics_state::PhysicsState;
use concerto::physics::rigid_body::MotionType::Dynamic;
use concerto::physics::rigid_body::{AllowedDofs, RigidBody};
use concerto::physics::shape::MeshCollider;
use concerto::scene::{scene::Scene, spawner::SceneSpawnerComponent};
use concerto::window::input::{Input, KeyCode, PhysicalKey};
use concerto::{
    ecs::{CommandQueue, Component, Res},
    foundation::assets::asset_server::AssetServer,
    reflect::{Reflect, ReflectDefault},
};
use glam::{Quat, Vec2, Vec3};

const CHAR_SCENE: AssetId = asset_id!("content/UAL1/scene.gasset");

const IDLE_LOOP: AssetId = asset_id!("content/UAL1/animation_Idle_Loop.gasset");
const JOG_FWD_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Fwd_Loop.gasset");
const JOG_FWD_L_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Fwd_L_Loop.gasset");
const JOG_FWD_R_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Fwd_R_Loop.gasset");
const JOG_LEFT_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Left_Loop.gasset");
const JOG_RIGHT_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Right_Loop.gasset");
const JOG_BWD_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Bwd_Loop.gasset");
const JOG_BWD_L_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Bwd_L_Loop.gasset");
const JOG_BWD_R_LOOP: AssetId = asset_id!("content/UAL1/animation_Jog_Bwd_R_Loop.gasset");
const JUMP_START: AssetId = asset_id!("content/UAL1/animation_Jump_Start.gasset");
const JUMP_LOOP: AssetId = asset_id!("content/UAL1/animation_Jump_Loop.gasset");
const JUMP_LAND: AssetId = asset_id!("content/UAL1/animation_Jump_Land.gasset");

#[derive(Component)]
pub(crate) struct Player;

#[derive(Component)]
pub(crate) struct AnimationsReady;

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default, SceneComponent)]
pub(crate) struct PlayerSpawner {
    should_spawn: bool,
}

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

        let collider = Collider::capsule(2.0, 1.0);
        let offset = ColliderOffset::bottom_origin(&collider);
        let character = cmd
            .spawn((
                Player,
                SceneSpawnerComponent(asset_server.load::<Scene>(CHAR_SCENE)),
                RigidBody {
                    density: 1000.0,
                    allowed_dofs: AllowedDofs::TRANSLATION | AllowedDofs::ROTATION_Y,
                    motion_type: Dynamic,
                },
                CharacterSimulation::default(),
                collider,
                // Origin at the capsule's bottom so the GLTF skeleton root sits on
                // the ground instead of floating at the capsule center.
                offset,
                Transform::from_translation_rotation(spawn_point.translation, spawn_point.rotation),
                GroundProbe::default(),
            ))
            .entity();

        cmd.spawn((
            CameraPivot::default(),
            spawn_point.clone(),
            EntityFollow {
                target: character,
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

/// Builds the movement blend space + jump FSM once the spawned scene has
/// inserted its `AnimationPlayer`. Exactly one skinned character exists, so the
/// sole player (on a spawned descendant of the `Player` entity) is
/// unambiguously ours; `AnimationsReady` guards against re-running.
///
// TODO(asset-import-pipeline): the jump FSM's runtime behaviour is unverified —
// the hard-coded UAL1 animation indices compile and import, but the state
// transitions have not been exercised in this task.
pub(crate) fn setup_character_animations(
    players: Query<(Entity, &AnimationPlayer), Without<AnimationsReady>>,
    server: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    let Some((player_entity, _player)) = players.iter().next() else {
        return;
    };

    let idle = server.load::<AnimationClip>(IDLE_LOOP);
    let jog = server.load::<AnimationClip>(JOG_FWD_LOOP);
    let jog_fw_l = server.load::<AnimationClip>(JOG_FWD_L_LOOP);
    let jog_fw_r = server.load::<AnimationClip>(JOG_FWD_R_LOOP);
    let jog_l = server.load::<AnimationClip>(JOG_LEFT_LOOP);
    let jog_r = server.load::<AnimationClip>(JOG_RIGHT_LOOP);
    let jog_bw = server.load::<AnimationClip>(JOG_BWD_LOOP);
    let job_bw_l = server.load::<AnimationClip>(JOG_BWD_L_LOOP);
    let job_bw_r = server.load::<AnimationClip>(JOG_BWD_R_LOOP);
    let _jump_start = server.load::<AnimationClip>(JUMP_START);
    let jump_loop = server.load::<AnimationClip>(JUMP_LOOP);
    let jump_land = server.load::<AnimationClip>(JUMP_LAND);

    let clips = [
        idle.clone(),
        jog.clone(),
        jog_fw_l.clone(),
        jog_fw_r.clone(),
        jog_l.clone(),
        jog_r.clone(),
        jog_bw.clone(),
        job_bw_l.clone(),
        job_bw_r.clone(),
        jump_loop.clone(),
        jump_land.clone(),
    ];

    let mut movement_graph = AnimationGraph::new();
    movement_graph
        .result_node()
        .with_blend_space_2d_input("movement", |context| {
            context
                .animation_clip_input(idle.clone(), Vec2::ZERO)
                .animation_clip_input(jog.clone(), Vec2::new(0.0, 1.0))
                .animation_clip_input(jog_fw_l.clone(), Vec2::new(-1.0, 1.0))
                .animation_clip_input(jog_fw_r.clone(), Vec2::new(1.0, 1.0))
                .animation_clip_input(jog_l.clone(), Vec2::new(-1.0, 0.0))
                .animation_clip_input(jog_r.clone(), Vec2::new(1.0, 0.0))
                .animation_clip_input(jog_bw.clone(), Vec2::new(0.0, -1.0))
                .animation_clip_input(job_bw_l.clone(), Vec2::new(-1.0, -1.0))
                .animation_clip_input(job_bw_r.clone(), Vec2::new(1.0, -1.0));
        });

    let movement_handle = server.add(movement_graph);
    let air_handle = server.add(AnimationGraph::from_node(AnimationClipNode::new(jump_loop)));
    let land_handle = server.add(AnimationGraph::from_node(
        AnimationClipNode::new(jump_land)
            .with_play_mode(PlayOnce)
            .with_start_time(0.1),
    ));
    let state_graphs = [
        movement_handle.clone(),
        air_handle.clone(),
        land_handle.clone(),
    ];

    let mut graph = AnimationGraph::new();
    graph.result_node().with_input(
        AnimationStateMachine::from_initial_state("movement", movement_handle, |transition| {
            transition
                // TODO: not supported yet, jump need to be triggered by an animation event
                // .to(
                //     "jump_start",
                //     AnimationFSMTrigger::on_bool("jumped", true),
                //     0.01,
                // );
                .to(
                    "air",
                    AnimationFSMTrigger::on_bool("is_grounded", false),
                    0.1,
                );
        })
        .state("air", air_handle, |transition| {
            transition
                .to(
                    "land",
                    AnimationFSMTrigger::on_bool("is_grounded", true),
                    0.1,
                )
                .to(
                    "movement",
                    AnimationFSMTrigger::on_bool("is_grounded", true),
                    0.1,
                );
        })
        .state("land", land_handle, |transition| {
            transition
                .to("movement", AnimationFSMTrigger::OnAnimationEnd, 0.1)
                .to(
                    "jump_start",
                    AnimationFSMTrigger::on_bool("jumped", true),
                    0.1,
                )
                .to(
                    "movement",
                    AnimationFSMTrigger::on_non_zero_vec("movement"),
                    0.1,
                );
        })
        .build(),
        |_node_context| {},
    );

    let graph_handle = server.add(graph);
    drop((clips, state_graphs));
    cmd.insert(AnimationHandleComponent::new(graph_handle), player_entity);
    cmd.insert(AnimationsReady, player_entity);
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
    movement: Query<
        (
            &CharacterSimulation,
            &mut DesiredMotion,
            &GroundProbe,
            &BodyId,
        ),
        With<Player>,
    >,
    anim_players: Query<&mut AnimationPlayer>,
    input: Res<Input>,
    mut physics: ResMut<PhysicsState>,
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

    for (simulation, mut desired, ground, body_id) in movement.iter() {
        // One skinned character, so the sole AnimationPlayer (on a spawned
        // descendant of the Player entity) is this character's.
        let Some(mut animation_player) = anim_players.iter().next() else {
            continue;
        };

        let current_vel = simulation.state.velocity;

        let just_pressed_jump = input.is_just_pressed(PhysicalKey::Code(KeyCode::Space));

        animation_player.set_bool_param("jumped", just_pressed_jump);

        animation_player.set_vec2_param(
            "movement",
            Vec2::new(-current_vel.x, current_vel.z) / PLAYER_SPEED,
        );

        let is_grounded = ground.is_grounded();

        if is_grounded && just_pressed_jump {
            physics.add_impulse(*body_id, Vec3::Y * 100000.0);
        }

        animation_player.set_bool_param("is_grounded", is_grounded);

        if is_grounded {
            desired.velocity = desired_velocity(player_input);
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto::animation::player::AnimationPlayer;
    use concerto::ecs::{IntoSystem, System, With, World};
    use concerto::foundation::{
        assets::{asset_server::AssetServer, asset_store::AssetStore},
        transform::Transform,
    };
    use concerto::gameplay::camera::CameraPivot;
    use concerto::mesh::MeshComponent;
    use concerto::motion_matching::{CharacterSimulation, DesiredMotion};
    use concerto::physics::physics_state::PhysicsState;
    use concerto::physics::{collider::Collider, shape::MeshCollider};
    use concerto::physics::{
        collider::register_colliders, ground::GroundProbe, rigid_body::RigidBody,
    };
    use concerto::scene::scene::Scene;
    use concerto::window::input::{Input, KeyCode, PhysicalKey};
    use glam::{Quat, Vec2, Vec3};

    use super::{
        PLAYER_SPEED, Player, PlayerSpawner, desired_velocity, face_camera_direction,
        spawn_character, update_movement,
    };

    #[test]
    fn the_character_spawns_at_the_spawn_point() {
        let mut world = World::new();
        world.insert_resource(PhysicsState::new());
        let mut server = AssetServer::new();
        server.register_asset(&AssetStore::<Scene>::new());
        world.insert_resource(server);
        let spawn_point = Vec3::new(3.0, 1.5, -7.0);
        world.spawn((
            Transform::from_translation_rotation(spawn_point, Quat::IDENTITY),
            PlayerSpawner { should_spawn: true },
        ));

        let mut system = spawn_character.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        let characters: Vec<Vec3> = world
            .query::<&Transform, With<Player>>()
            .iter(&mut world)
            .map(|transform| transform.translation)
            .collect();
        assert_eq!(characters, vec![spawn_point]);
    }

    fn world_with_spawner() -> (World, Vec3) {
        let mut world = World::new();
        world.insert_resource(PhysicsState::new());
        let mut server = AssetServer::new();
        server.register_asset(&AssetStore::<Scene>::new());
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

    fn world_with_player(simulation: CharacterSimulation, desired: DesiredMotion) -> World {
        let mut world = World::new();
        world.register_component::<Collider>();
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        world.spawn((
            Player,
            RigidBody::default(),
            Collider::capsule(2.0, 1.0),
            Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Quat::IDENTITY),
            simulation,
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
    fn the_movement_parameter_reads_the_simulation_velocity() {
        let mut simulation = CharacterSimulation::default();
        simulation.state.velocity = Vec3::new(2.5, 0.0, 1.0);
        let mut world = world_with_player(simulation, DesiredMotion::default());
        world.insert_resource(Input::new());

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
        let mut world = world_with_player(CharacterSimulation::default(), DesiredMotion::default());
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
        let mut world = world_with_player(CharacterSimulation::default(), desired);
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

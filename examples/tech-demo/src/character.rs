use concerto::{asset_id, core::assets::AssetId};
use std::f32::consts::PI;

use concerto::animation::clip::AnimationClip;
use concerto::animation::graph::AnimationGraph;
use concerto::animation::node::AnimationPlayMode::PlayOnce;
use concerto::animation::node::state_machine::{AnimationFSMTrigger, AnimationStateMachine};
use concerto::animation::node::{AnimationClipNode, AnimationNodeKind};
use concerto::animation::player::{AnimationHandleComponent, AnimationPlayer};
use concerto::core::transform::Transform;
use concerto::director::VirtualCamera;
use concerto::ecs::component::scene::{SceneComponent, SceneSpawnContext};
use concerto::ecs::{Entity, Query, ResMut, With, Without};
use concerto::gameplay::camera::{CameraPivot, EntityFollow};
use concerto::physics::body::BodyId;
use concerto::physics::collider::{Collider, ColliderOffset};
use concerto::physics::ground::GroundProbe;
use concerto::physics::movement::CharacterMovement;
use concerto::physics::physics_state::PhysicsState;
use concerto::physics::rigid_body::MotionType::Dynamic;
use concerto::physics::rigid_body::{AllowedDofs, RigidBody};
use concerto::scene::{scene::Scene, spawner::SceneSpawnerComponent};
use concerto::window::input::{Input, KeyCode, PhysicalKey};
use concerto::{
    core::assets::asset_server::AssetServer,
    ecs::{CommandQueue, Component, Res},
};
use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

const CHAR_SCENE: AssetId = asset_id!("content/UAL1/scene.gasset");

// TODO(asset-import-pipeline): magic indices — an animation-name manifest sub-asset would replace these.
const IDLE_LOOP: AssetId = asset_id!("content/UAL1/animation_53.gasset");
const JOG_FWD_LOOP: AssetId = asset_id!("content/UAL1/animation_67.gasset");
const JOG_FWD_L_LOOP: AssetId = asset_id!("content/UAL1/animation_64.gasset");
const JOG_FWD_R_LOOP: AssetId = asset_id!("content/UAL1/animation_68.gasset");
const JOG_LEFT_LOOP: AssetId = asset_id!("content/UAL1/animation_69.gasset");
const JOG_RIGHT_LOOP: AssetId = asset_id!("content/UAL1/animation_70.gasset");
const JOG_BWD_LOOP: AssetId = asset_id!("content/UAL1/animation_62.gasset");
const JOG_BWD_L_LOOP: AssetId = asset_id!("content/UAL1/animation_61.gasset");
const JOG_BWD_R_LOOP: AssetId = asset_id!("content/UAL1/animation_63.gasset");
const JUMP_START: AssetId = asset_id!("content/UAL1/animation_73.gasset");
const JUMP_LOOP: AssetId = asset_id!("content/UAL1/animation_72.gasset");
const JUMP_LAND: AssetId = asset_id!("content/UAL1/animation_71.gasset");

#[derive(Component)]
pub(crate) struct Player;

#[derive(Component)]
pub(crate) struct AnimationsReady;

#[derive(Component, Serialize, Deserialize)]
pub(crate) struct PlayerSpawner {
    should_spawn: bool,
}

impl SceneComponent for PlayerSpawner {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

pub(crate) fn spawn_character(
    spawners: Query<(&Transform, &mut PlayerSpawner)>,
    asset_server: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
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
                CharacterMovement::new(0.1),
                collider,
                // Origin at the capsule's bottom so the GLTF skeleton root sits on
                // the ground instead of floating at the capsule center.
                offset,
                Transform::from_translation_rotation(Vec3::new(0.0, 10.0, -5.0), Quat::IDENTITY),
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

    let mut movement_graph = AnimationGraph::new();
    movement_graph
        .result_node()
        .with_blend_space_2d_input("movement", |context| {
            context
                .animation_clip_input(idle, Vec2::ZERO)
                .animation_clip_input(jog, Vec2::new(0.0, 1.0))
                .animation_clip_input(jog_fw_l, Vec2::new(-1.0, 1.0))
                .animation_clip_input(jog_fw_r, Vec2::new(1.0, 1.0))
                .animation_clip_input(jog_l, Vec2::new(-1.0, 0.0))
                .animation_clip_input(jog_r, Vec2::new(1.0, 0.0))
                .animation_clip_input(jog_bw, Vec2::new(0.0, -1.0))
                .animation_clip_input(job_bw_l, Vec2::new(-1.0, -1.0))
                .animation_clip_input(job_bw_r, Vec2::new(1.0, -1.0));
        });

    let mut graph = AnimationGraph::new();
    graph.result_node().with_input(
        AnimationNodeKind::StateMachine(
            AnimationStateMachine::from_initial_state(
                "movement",
                server.add(movement_graph),
                |transition| {
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
                },
            )
            .state(
                "air",
                server.add(AnimationGraph::from_node(AnimationNodeKind::Clip(
                    AnimationClipNode::new(jump_loop),
                ))),
                |transition| {
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
                },
            )
            .state(
                "land",
                server.add(AnimationGraph::from_node(AnimationNodeKind::Clip(
                    AnimationClipNode::new(jump_land)
                        .with_play_mode(PlayOnce)
                        .with_start_time(0.1),
                ))),
                |transition| {
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
                },
            )
            .build(),
        ),
        |_node_context| {},
    );

    cmd.insert(
        AnimationHandleComponent::new(server.add(graph)),
        player_entity,
    );
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
    players: Query<&mut Transform, With<Player>>,
    pivots: Query<&CameraPivot>,
) {
    let Some(pivot) = pivots.iter().next() else {
        return;
    };

    let facing = Quat::from_rotation_y(pivot.yaw());
    for mut transform in players.iter() {
        transform.rotation = facing;
    }
}

pub(crate) fn update_movement(
    movement: Query<(&mut CharacterMovement, &GroundProbe, &BodyId), With<Player>>,
    anim_players: Query<&mut AnimationPlayer>,
    input: Res<Input>,
    mut physics: ResMut<PhysicsState>,
) {
    const PLAYER_SPEED: f32 = 5.0;
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

    for (mut movement, ground, body_id) in movement.iter() {
        // One skinned character, so the sole AnimationPlayer (on a spawned
        // descendant of the Player entity) is this character's.
        let Some(mut animation_player) = anim_players.iter().next() else {
            continue;
        };

        let current_vel = movement.current_velocity();

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
            movement
                .set_target_velocity(Vec3::new(player_input.x, 0.0, player_input.y) * PLAYER_SPEED);
        }
    }
}

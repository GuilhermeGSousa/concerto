//! A ragdoll chain falling on a floor, for looking at.

use std::f32::consts::FRAC_PI_3;

use concerto::{
    DefaultPlugins,
    animation::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        graph::AnimationGraph,
        node::{
            AnimationClipNode, AnimationNode,
            state_machine::{AnimationFSMTrigger, AnimationStateMachine, TransitionBuilder},
        },
        player::{AnimationHandleComponent, AnimationPlayer},
    },
    app::{
        App,
        plugins::ContentSettings,
        schedule_groups::{Startup, Update},
    },
    color::Color,
    ecs::{
        CommandQueue, Component, Entity, Query, Res, ResMut, Resource,
        component::scene::SceneEntityRef, entity::hierarchy::ChildOf,
    },
    foundation::{
        assets::{asset_server::AssetServer, handle::AssetHandle},
        time::Time,
        transform::{GlobalTransform, Transform},
    },
    mesh::{JointPose, MeshComponent, Skeleton, SkeletonComponent, primitives},
    name::{Name, name},
    physics::{
        collider::Collider,
        contact::Contacts,
        physics_state::PhysicsState,
        ragdoll::{
            BodyPose, JointDrive, JointSettings, MotorSettings, PartSettings, PartShape, RagdollId,
            RagdollSettings,
        },
        rigid_body::MotionType,
    },
    render::{
        MaterialComponent,
        assets::material::StandardMaterial,
        components::light::{Light, LightType},
    },
    window::input::{Input, KeyCode, PhysicalKey},
};
use concerto_debug_gizmos::DebugGizmos;
use concerto_physics_gizmos::{PhysicsGizmos, PhysicsGizmosPlugin};
use concerto_ragdoll::{
    BRACED, BalanceNode, BalanceSettings, BodyHit, CharacterRagdoll, Death, Die, DyingNode, FLINCH,
    FallNode, HoldNode, LimpNode, RagdollBody, ShotNode, StaggerNode,
    messages::{Shot, Stagger},
    testing::reference_humanoid,
};
use glam::{Quat, Vec3};

const LINK: f32 = 0.5;
const SWING: f32 = 0.6;
const HALF_HEIGHT: f32 = 0.15;
const RADIUS: f32 = 0.08;
const DROP_HEIGHT: f32 = 2.5;
const POKE: Vec3 = Vec3::new(0.0, 6.0, 3.0);
const HOLD_BEND: f32 = -0.4;
const HOLD_HEIGHT: f32 = 2.0;
const CHAIN_X: f32 = -3.0;
const HUMANOID_CENTER: Vec3 = Vec3::new(1.5, 0.0, 0.0);
const WALK_RADIUS: f32 = 1.0;
const WALK_SPEED: f32 = 1.0;
const PUSH: f32 = 60.0;
const DYING_SECONDS: f32 = 1.0;
const SHOT: f32 = 40.0;
const STAGGER_SEVERITY: f32 = 0.5;
const ARM_SWING: f32 = 0.8;

struct PostureKey {
    key: KeyCode,
    name: &'static str,
}

impl PostureKey {
    const fn new(key: KeyCode, name: &'static str) -> Self {
        Self { key, name }
    }
}

#[derive(Resource)]
struct Humanoid {
    skeleton: Entity,
    bones: Vec<Entity>,
    parents: Vec<Option<u16>>,
    rest: Vec<JointPose>,
    left_arm: usize,
    right_arm: usize,
    elapsed: f32,
    walked: f32,
    limp_after: Option<f32>,
    walking: bool,
}

#[derive(Component)]
struct ChainRagdoll {
    id: RagdollId,
    settings: RagdollSettings,
    start: Vec<BodyPose>,
}

#[derive(Component)]
struct RagdollVisual {
    owner: Entity,
    part: usize,
}

fn chain_settings() -> RagdollSettings {
    let link = |index: usize| {
        PartSettings::new(
            BodyPose::from_translation(Vec3::new(0.0, -LINK * index as f32, 0.0)),
            PartShape::Capsule {
                half_height: HALF_HEIGHT,
                radius: RADIUS,
            },
        )
        .with_shape_offset(BodyPose::from_translation(Vec3::new(0.0, -LINK / 2.0, 0.0)))
        .with_mass(2.0)
    };
    let joint = JointSettings::new(Vec3::NEG_Y, Vec3::X)
        .with_swing(SWING, SWING)
        .with_twist(-0.3, 0.3)
        .with_max_friction_torque(0.5)
        .with_motor(MotorSettings {
            frequency: 10.0,
            ..MotorSettings::default()
        });
    RagdollSettings::new()
        .with_part(link(0))
        .with_part(link(1).with_parent(0, joint))
        .with_part(link(2).with_parent(1, joint))
}

fn spawn_chain(
    cmd: &mut CommandQueue,
    state: &mut PhysicsState,
    assets: &AssetServer,
    root: BodyPose,
) {
    let settings = chain_settings();
    let built = settings.build().expect("the chain should build");
    let start: Vec<BodyPose> = settings
        .parts
        .iter()
        .map(|part| root.then(part.pose))
        .collect();
    let owner = cmd.spawn(Transform::default()).entity();
    let Some(id) = state.create_ragdoll(&built, owner, &start) else {
        eprintln!("this physics backend has no ragdolls");
        return;
    };

    let mesh = assets.add(primitives::capsule(RADIUS, HALF_HEIGHT, 6, 16));
    let material = assets.add(
        StandardMaterial::new(None, None).with_base_color_factor(Color::rgba(0.9, 0.35, 0.2, 1.0)),
    );
    for (part, (pose, settings)) in start.iter().zip(&settings.parts).enumerate() {
        cmd.spawn((
            RagdollVisual { owner, part },
            MeshComponent {
                handle: mesh.clone(),
            },
            MaterialComponent {
                handle: material.clone(),
            },
            pose.then(settings.shape_offset).to_transform(),
        ));
    }
    cmd.entity(owner).insert((
        id,
        ChainRagdoll {
            id,
            settings,
            start,
        },
    ));
}

fn spawn_scene(mut cmd: CommandQueue, mut state: ResMut<PhysicsState>, assets: Res<AssetServer>) {
    cmd.spawn((
        concerto::director::MainCamera,
        Transform::from_translation_rotation(
            Vec3::new(-0.5, 1.5, 6.4),
            Quat::from_rotation_x(-0.08),
        ),
    ));
    cmd.spawn((
        Light {
            color: Color::rgba(1.0, 0.95, 0.9, 1.0),
            intensity: 3.0,
            light_type: LightType::Directional,
            range: 0.0,
            shadowmaps_enabled: true,
        },
        Transform::from_translation_rotation(Vec3::new(0.0, 8.0, 4.0), Quat::from_rotation_x(-0.8)),
    ));
    cmd.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        Contacts::default(),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        MeshComponent {
            handle: assets.add(primitives::cuboid(Vec3::new(40.0, 1.0, 40.0))),
        },
        MaterialComponent {
            handle: assets.add(
                StandardMaterial::new(None, None)
                    .with_base_color_factor(Color::rgba(0.4, 0.45, 0.5, 1.0)),
            ),
        },
    ));

    let root = BodyPose::new(
        Vec3::new(CHAIN_X, DROP_HEIGHT, 0.0),
        Quat::from_rotation_z(FRAC_PI_3),
    );
    spawn_chain(&mut cmd, &mut state, &assets, root);
    spawn_humanoid(&mut cmd, &assets);
}

const POSTURES: [&str; 6] = ["walk", "limp", "flinch", "braced", "half", "balance"];

struct Posture {
    name: Name,
    graph: AssetHandle<AnimationGraph>,
}

fn standing_clip(skeleton: &Skeleton) -> AnimationClip {
    let pelvis = skeleton
        .bone_index(name!("pelvis"))
        .expect("the reference humanoid has a pelvis");
    let mut clip = AnimationClip::default();
    clip.add_channel(
        skeleton.bone_ids[pelvis],
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::from_rotation(
                [Quat::IDENTITY.to_array(), Quat::IDENTITY.to_array()].into_iter(),
            ),
        ),
    );
    clip
}

fn over_clip(node: impl AnimationNode, clip: &AssetHandle<AnimationClip>) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(node)
        .input(AnimationClipNode::new(clip.clone()));
    graph
}

fn reacts(to: &mut TransitionBuilder, from: Name, postures: &[Posture]) {
    for posture in postures.iter().filter(|posture| posture.name != from) {
        to.to(
            posture.name,
            AnimationFSMTrigger::on_bool(posture.name, true),
            0.0,
        );
    }
    to.to(
        name!("shot"),
        AnimationFSMTrigger::on_message::<Shot>(),
        0.0,
    )
    .to(
        name!("stagger"),
        AnimationFSMTrigger::on_message::<Stagger>(),
        0.0,
    )
    .to(
        name!("dying"),
        AnimationFSMTrigger::on_message::<Death>(),
        0.0,
    );
}

fn humanoid_graph(assets: &AssetServer, skeleton: &Skeleton) -> AnimationGraph {
    let clip = assets.add(standing_clip(skeleton));
    let walk = assets.add(AnimationGraph::from_node(AnimationClipNode::new(
        clip.clone(),
    )));
    let postures = [
        Posture {
            name: name!("walk"),
            graph: walk.clone(),
        },
        Posture {
            name: name!("limp"),
            graph: assets.add(over_clip(LimpNode::default(), &clip)),
        },
        Posture {
            name: name!("flinch"),
            graph: assets.add(over_clip(HoldNode::new(FLINCH), &clip)),
        },
        Posture {
            name: name!("braced"),
            graph: assets.add(over_clip(HoldNode::new(BRACED), &clip)),
        },
        Posture {
            name: name!("half"),
            graph: assets.add(over_clip(
                HoldNode::new(BRACED).with_weight(name!("half")),
                &clip,
            )),
        },
        Posture {
            name: name!("balance"),
            graph: assets.add(over_clip(
                BalanceNode::default()
                    .with_settings(BalanceSettings::default().with_settle_seconds(f32::MAX)),
                &clip,
            )),
        },
    ];
    let mut machine = AnimationStateMachine::from_initial_state(
        postures[0].name,
        postures[0].graph.clone(),
        |to| reacts(to, postures[0].name, &postures),
    );
    for posture in &postures[1..] {
        machine = machine.state(posture.name, posture.graph.clone(), |to| {
            reacts(to, posture.name, &postures)
        });
    }
    let steady = |to: &mut TransitionBuilder| {
        to.to(name!("walk"), AnimationFSMTrigger::OnSucceeded, 0.0)
            .to(name!("fall"), AnimationFSMTrigger::OnFailed, 0.0)
            .to(
                name!("dying"),
                AnimationFSMTrigger::on_message::<Death>(),
                0.0,
            );
    };
    AnimationGraph::from_node(
        machine
            .state(
                name!("shot"),
                assets.add(over_clip(ShotNode::default(), &clip)),
                steady,
            )
            .state(
                name!("stagger"),
                assets.add(over_clip(StaggerNode::default(), &clip)),
                steady,
            )
            .state(
                name!("fall"),
                assets.add(over_clip(FallNode::default(), &clip)),
                |to| {
                    to.to(name!("down"), AnimationFSMTrigger::OnSucceeded, 0.0)
                        .to(
                            name!("dying"),
                            AnimationFSMTrigger::on_message::<Death>(),
                            0.0,
                        );
                },
            )
            .state(
                name!("down"),
                assets.add(over_clip(LimpNode::default(), &clip)),
                |to| {
                    to.to(
                        name!("dying"),
                        AnimationFSMTrigger::on_message::<Death>(),
                        0.0,
                    )
                    .to(
                        name!("walk"),
                        AnimationFSMTrigger::on_bool(name!("walk"), true),
                        0.0,
                    );
                },
            )
            .state(
                name!("dying"),
                assets.add(over_clip(DyingNode::default(), &clip)),
                |to| {
                    to.to(name!("dead"), AnimationFSMTrigger::OnSucceeded, 0.0);
                },
            )
            .state(
                name!("dead"),
                assets.add(over_clip(LimpNode::default(), &clip)),
                |_| {},
            )
            .build(),
    )
}

fn take_posture(player: &mut AnimationPlayer, posture: &str) {
    for candidate in POSTURES {
        player.set_bool_param(Name::from(candidate), candidate == posture);
    }
}

fn spawn_humanoid(cmd: &mut CommandQueue, assets: &AssetServer) {
    let reference = reference_humanoid();
    let parents = reference.skeleton.parents.to_vec();
    let rest = reference.skeleton.rest_pose.to_vec();
    let index_of = |name: &str| {
        reference
            .skeleton
            .names
            .iter()
            .position(|bone| bone == name)
            .expect("the reference humanoid has the arm bones")
    };
    let left_arm = index_of("upperarm_l");
    let right_arm = index_of("upperarm_r");
    let graph = assets.add(humanoid_graph(assets, &reference.skeleton));
    let skeleton = assets.add(reference.skeleton);
    let mut ragdoll = reference.ragdoll;
    ragdoll.skeleton = skeleton.clone();
    let ragdoll = assets.add(ragdoll);

    let root = cmd
        .spawn(Transform::from_translation(
            HUMANOID_CENTER + Vec3::X * WALK_RADIUS,
        ))
        .entity();
    let mut bones: Vec<Entity> = Vec::with_capacity(rest.len());
    for (index, local) in rest.iter().enumerate() {
        let parent = parents[index].map_or(root, |parent| bones[parent as usize]);
        bones.push(
            cmd.spawn((
                Transform::from_translation_rotation(local.translation, local.rotation),
                ChildOf::new(parent),
            ))
            .entity(),
        );
    }
    cmd.entity(root).insert((
        SkeletonComponent::new(
            skeleton,
            bones.iter().copied().map(SceneEntityRef::Entity).collect(),
            bones.first().copied().map(SceneEntityRef::Entity),
        ),
        CharacterRagdoll::new(ragdoll),
        AnimationHandleComponent::new(graph),
    ));
    let limp_after = std::env::var("RAGDOLL_TEST_LIMP_AFTER")
        .ok()
        .and_then(|seconds| seconds.parse().ok());
    cmd.insert_resource(Humanoid {
        skeleton: root,
        bones,
        parents,
        rest,
        left_arm,
        right_arm,
        elapsed: 0.0,
        walked: 0.0,
        limp_after,
        walking: true,
    });
}

fn animate_humanoid(
    time: Res<Time>,
    humanoid: Option<ResMut<Humanoid>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(mut humanoid) = humanoid else {
        return;
    };
    let delta = time.delta().as_secs_f32();
    humanoid.elapsed += delta;
    for (index, &bone) in humanoid.bones.iter().enumerate() {
        if let Some(mut transform) = transforms.get_entity_mut(bone) {
            let rest = &humanoid.rest[index];
            transform.translation = rest.translation;
            transform.rotation = rest.rotation;
        }
    }
    if !humanoid.walking {
        return;
    }
    humanoid.walked += delta;
    let swing = ARM_SWING * (humanoid.elapsed * 2.0).sin();
    for (bone, sign) in [(humanoid.left_arm, 1.0), (humanoid.right_arm, -1.0)] {
        if let Some(mut transform) = transforms.get_entity_mut(humanoid.bones[bone]) {
            transform.rotation = Quat::from_rotation_z(sign * swing);
        }
    }
    let angle = humanoid.walked * WALK_SPEED / WALK_RADIUS;
    if let Some(mut transform) = transforms.get_entity_mut(humanoid.skeleton) {
        transform.translation =
            HUMANOID_CENTER + WALK_RADIUS * Vec3::new(angle.cos(), 0.0, angle.sin());
        transform.rotation = Quat::from_rotation_y((-angle.sin()).atan2(angle.cos()));
    }
}

fn control_humanoid(
    input: Res<Input>,
    humanoid: Option<ResMut<Humanoid>>,
    bodies: Query<(&RagdollBody, &RagdollId)>,
    mut players: Query<&mut AnimationPlayer>,
    mut physics: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let Some(mut humanoid) = humanoid else {
        return;
    };
    let Some(mut player) = players.get_entity_mut(humanoid.skeleton) else {
        return;
    };
    let pressed = |key: KeyCode| input.is_just_pressed(PhysicalKey::Code(key));
    let timed = humanoid
        .limp_after
        .is_some_and(|after| humanoid.elapsed >= after && humanoid.elapsed - after < 1.0);
    let posture = [
        PostureKey::new(KeyCode::KeyH, "limp"),
        PostureKey::new(KeyCode::KeyJ, "walk"),
        PostureKey::new(KeyCode::KeyB, "half"),
        PostureKey::new(KeyCode::KeyP, "flinch"),
        PostureKey::new(KeyCode::KeyO, "braced"),
        PostureKey::new(KeyCode::KeyK, "balance"),
    ]
    .into_iter()
    .find_map(|posture| pressed(posture.key).then_some(posture.name))
    .or(timed.then_some("limp"));
    if let Some(posture) = posture {
        if posture == "half" {
            player.set_float_param(name!("half"), 0.5);
        }
        take_posture(&mut player, posture);
        humanoid.walking = posture == "walk";
    }
    if pressed(KeyCode::KeyX) {
        cmd.trigger(Die {
            character: humanoid.skeleton,
            seconds: DYING_SECONDS,
        });
    }
    let Some((body, &ragdoll)) = bodies.get_entity(humanoid.skeleton) else {
        return;
    };
    let Some(chest) = body.resolved().landmarks().chest else {
        return;
    };
    if pressed(KeyCode::KeyI) {
        let mut pose = Vec::new();
        physics.ragdoll_pose(ragdoll, &mut pose);
        if let Some(chest_pose) = pose.get(chest) {
            humanoid.walking = false;
            cmd.trigger(
                BodyHit::new(
                    humanoid.skeleton,
                    chest,
                    chest_pose.translation + Vec3::Z * 0.12,
                    SHOT * Vec3::NEG_Z,
                )
                .shot(),
            );
        }
    }
    let shove = pressed(KeyCode::KeyU);
    let stagger = pressed(KeyCode::KeyY);
    if shove || stagger {
        let part = physics.ragdoll_bodies(ragdoll)[chest];
        physics.add_impulse(part, PUSH * Vec3::Z);
        if stagger {
            humanoid.walking = false;
            player.send(Stagger {
                direction: Vec3::Z,
                severity: STAGGER_SEVERITY,
            });
        }
    }
}

fn draw_skeleton(
    humanoid: Option<Res<Humanoid>>,
    globals: Query<&GlobalTransform>,
    mut gizmos: DebugGizmos,
) {
    let Some(humanoid) = humanoid else {
        return;
    };
    let white = Color::rgba(1.0, 1.0, 1.0, 1.0);
    let position = |bone: Entity| globals.get_entity(bone).map(|global| global.translation());
    for (index, &bone) in humanoid.bones.iter().enumerate() {
        let Some(at) = position(bone) else {
            continue;
        };
        gizmos.cross(at, 0.02, white);
        if let Some(parent) = humanoid.parents[index]
            && let Some(parent_at) = position(humanoid.bones[parent as usize])
        {
            gizmos.line(at, parent_at, white);
        }
    }
}

fn sync_visuals(
    state: Res<PhysicsState>,
    chains: Query<&ChainRagdoll>,
    mut visuals: Query<(&RagdollVisual, &mut Transform)>,
) {
    let mut pose = Vec::new();
    for (visual, mut transform) in visuals.iter_mut() {
        let Some(chain) = chains.get_entity(visual.owner) else {
            continue;
        };
        state.ragdoll_pose(chain.id, &mut pose);
        let Some(part_pose) = pose.get(visual.part) else {
            continue;
        };
        **transform = part_pose
            .then(chain.settings.parts[visual.part].shape_offset)
            .to_transform();
    }
}

fn control_chains(
    input: Res<Input>,
    mut state: ResMut<PhysicsState>,
    chains: Query<&ChainRagdoll>,
) {
    let reset = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyR));
    let poke = input.is_just_pressed(PhysicalKey::Code(KeyCode::Space));
    let hold = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyM));
    let limp = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyL));
    for chain in chains.iter() {
        if reset {
            state.set_ragdoll_pose(chain.id, &chain.start);
        }
        if poke && let Some(&body) = state.ragdoll_bodies(chain.id).last() {
            state.add_impulse(body, POKE);
        }
        if hold {
            let root = BodyPose::from_translation(Vec3::new(CHAIN_X, HOLD_HEIGHT, 0.0));
            let hanging: Vec<BodyPose> = chain
                .settings
                .parts
                .iter()
                .map(|part| root.then(part.pose))
                .collect();
            let target: Vec<BodyPose> = hanging
                .iter()
                .enumerate()
                .map(|(part, pose)| {
                    BodyPose::new(
                        pose.translation,
                        Quat::from_rotation_z(HOLD_BEND * part as f32),
                    )
                })
                .collect();
            state.set_ragdoll_pose(chain.id, &hanging);
            state.set_ragdoll_part_motion(chain.id, 0, MotionType::Kinematic);
            state.drive_ragdoll_motors(chain.id, &target);
        }
        if limp {
            state.set_ragdoll_part_motion(chain.id, 0, MotionType::Dynamic);
            for part in 1..chain.settings.parts.len() {
                state.set_ragdoll_joint_drive(chain.id, part, JointDrive::Off);
            }
        }
    }
}

fn toggle_gizmos(input: Res<Input>, mut shown: ResMut<PhysicsGizmos>) {
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyG)) {
        *shown = if *shown == PhysicsGizmos::default() {
            PhysicsGizmos::all()
        } else {
            PhysicsGizmos::default()
        };
    }
}

fn show_gizmos(mut shown: ResMut<PhysicsGizmos>) {
    *shown = PhysicsGizmos::all();
}

fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(PhysicsGizmosPlugin)
        .add_system(Startup, (spawn_scene, show_gizmos))
        .add_system(
            Update,
            (
                control_chains,
                sync_visuals,
                toggle_gizmos,
                animate_humanoid,
                control_humanoid,
                draw_skeleton,
            ),
        );
}

fn main() {
    env_logger::init();
    concerto::run(
        game,
        ContentSettings {
            registry: "",
            project_root: None,
        },
    );
}

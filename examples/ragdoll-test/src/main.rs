//! A ragdoll chain falling on a floor, for looking at.

use std::f32::consts::FRAC_PI_3;

use concerto::{
    DefaultPlugins,
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
        assets::asset_server::AssetServer,
        time::Time,
        transform::{GlobalTransform, Transform},
    },
    mesh::{JointPose, MeshComponent, SkeletonComponent, primitives},
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
    Balance, BodyDrive, BodyHit, BodySense, CharacterRagdoll, FallReaction, PhysicalState,
    RagdollBody, RagdollMode, ShotReaction, Stagger, testing::reference_humanoid,
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
const SHOT: f32 = 40.0;
const STAGGER_SEVERITY: f32 = 0.5;
const ARM_SWING: f32 = 0.8;

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
    for part in 0..settings.parts.len() {
        cmd.spawn((
            RagdollVisual { owner, part },
            MeshComponent {
                handle: mesh.clone(),
            },
            MaterialComponent {
                handle: material.clone(),
            },
            start[part]
                .then(settings.parts[part].shape_offset)
                .to_transform(),
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
        BodySense::default(),
        ShotReaction::default(),
        FallReaction::default(),
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
    });
}

fn animate_humanoid(
    time: Res<Time>,
    humanoid: Option<ResMut<Humanoid>>,
    characters: Query<&CharacterRagdoll>,
    transforms: Query<&mut Transform>,
) {
    let Some(mut humanoid) = humanoid else {
        return;
    };
    let delta = time.delta().as_secs_f32();
    humanoid.elapsed += delta;
    let limp = characters
        .get_entity(humanoid.skeleton)
        .is_some_and(|character| character.mode != RagdollMode::FollowsAnimation);
    for (index, &bone) in humanoid.bones.iter().enumerate() {
        if let Some(mut transform) = transforms.get_entity(bone) {
            let rest = &humanoid.rest[index];
            transform.translation = rest.translation;
            transform.rotation = rest.rotation;
        }
    }
    if limp {
        return;
    }
    humanoid.walked += delta;
    let swing = ARM_SWING * (humanoid.elapsed * 2.0).sin();
    for (bone, sign) in [(humanoid.left_arm, 1.0), (humanoid.right_arm, -1.0)] {
        if let Some(mut transform) = transforms.get_entity(humanoid.bones[bone]) {
            transform.rotation = Quat::from_rotation_z(sign * swing);
        }
    }
    let angle = humanoid.walked * WALK_SPEED / WALK_RADIUS;
    if let Some(mut transform) = transforms.get_entity(humanoid.skeleton) {
        transform.translation =
            HUMANOID_CENTER + WALK_RADIUS * Vec3::new(angle.cos(), 0.0, angle.sin());
        transform.rotation = Quat::from_rotation_y((-angle.sin()).atan2(angle.cos()));
    }
}

fn balance_humanoid(
    input: Res<Input>,
    humanoid: Option<Res<Humanoid>>,
    bodies: Query<(&RagdollBody, &RagdollId)>,
    mut physics: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let Some(humanoid) = humanoid else {
        return;
    };
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyK)) {
        cmd.entity(humanoid.skeleton).insert((
            Balance::default().with_settle_seconds(f32::MAX),
            BodyDrive::default(),
            PhysicalState::default(),
        ));
    }
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyJ)) {
        cmd.remove::<Balance>(humanoid.skeleton);
        cmd.remove::<Stagger>(humanoid.skeleton);
    }
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyI))
        && let Some((body, &ragdoll)) = bodies.get_entity(humanoid.skeleton)
        && let Some(chest) = body.resolved().landmarks().chest
    {
        let mut pose = Vec::new();
        physics.ragdoll_pose(ragdoll, &mut pose);
        cmd.trigger(
            BodyHit::new(
                humanoid.skeleton,
                chest,
                pose[chest].translation + Vec3::Z * 0.12,
                SHOT * Vec3::NEG_Z,
            )
            .shot(),
        );
    }
    let shove = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyU));
    let stagger = input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyY));
    if (shove || stagger)
        && let Some((body, &ragdoll)) = bodies.get_entity(humanoid.skeleton)
        && let Some(chest) = body.resolved().landmarks().chest
    {
        let part = physics.ragdoll_bodies(ragdoll)[chest];
        physics.add_impulse(part, PUSH * Vec3::Z);
        if stagger {
            cmd.entity(humanoid.skeleton)
                .insert(Stagger::new(Vec3::Z, STAGGER_SEVERITY));
        }
    }
}

fn control_humanoid(
    input: Res<Input>,
    humanoid: Option<Res<Humanoid>>,
    characters: Query<&mut CharacterRagdoll>,
) {
    let Some(humanoid) = humanoid else {
        return;
    };
    let Some(mut character) = characters.get_entity(humanoid.skeleton) else {
        return;
    };
    let timed = humanoid
        .limp_after
        .is_some_and(|after| humanoid.elapsed >= after && humanoid.elapsed - after < 1.0)
        && character.mode != RagdollMode::Limp;
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyH)) || timed {
        character.go_limp();
    }
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyJ)) {
        character.mode = RagdollMode::FollowsAnimation;
        character.weight = 0.0;
    }
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyB)) {
        character.weight = 0.5;
    }
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyP)) {
        character.power(concerto_ragdoll::FLINCH);
    }
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyO)) {
        character.power(concerto_ragdoll::BRACED);
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
    visuals: Query<(&RagdollVisual, &mut Transform)>,
) {
    let mut pose = Vec::new();
    for (visual, mut transform) in visuals.iter() {
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
                balance_humanoid,
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

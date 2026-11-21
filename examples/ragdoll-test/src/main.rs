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
    ecs::{CommandQueue, Component, Entity, Query, Res, ResMut},
    foundation::{assets::asset_server::AssetServer, transform::Transform},
    mesh::{MeshComponent, primitives},
    physics::{
        collider::Collider,
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
use glam::{Quat, Vec3};

const LINK: f32 = 0.5;
const SWING: f32 = 0.6;
const HALF_HEIGHT: f32 = 0.15;
const RADIUS: f32 = 0.08;
const DROP_HEIGHT: f32 = 2.5;
const POKE: Vec3 = Vec3::new(0.0, 6.0, 3.0);
const HOLD_BEND: f32 = -0.4;
const HOLD_HEIGHT: f32 = 2.0;

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
    cmd.entity(owner).insert(ChainRagdoll {
        id,
        settings,
        start,
    });
}

fn spawn_scene(mut cmd: CommandQueue, mut state: ResMut<PhysicsState>, assets: Res<AssetServer>) {
    cmd.spawn((
        concerto::director::MainCamera,
        Transform::from_translation_rotation(
            Vec3::new(0.0, 2.0, 6.0),
            Quat::from_rotation_x(-0.25),
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
        Vec3::new(0.0, DROP_HEIGHT, 0.0),
        Quat::from_rotation_z(FRAC_PI_3),
    );
    spawn_chain(&mut cmd, &mut state, &assets, root);
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
            let root = BodyPose::from_translation(Vec3::new(0.0, HOLD_HEIGHT, 0.0));
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

fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .add_system(Startup, spawn_scene)
        .add_system(Update, (control_chains, sync_visuals));
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

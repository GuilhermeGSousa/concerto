//! One standing humanoid and a key for every physical behaviour, with its state on screen.

use concerto::{
    DefaultPlugins,
    app::{
        App,
        plugins::ContentSettings,
        schedule_groups::{Startup, Update},
    },
    color::Color,
    ecs::{
        CommandQueue, Component, Entity, Query, Res, ResMut, Resource, With,
        component::scene::SceneEntityRef, entity::hierarchy::ChildOf,
    },
    foundation::{assets::asset_server::AssetServer, transform::Transform},
    mesh::{JointPose, MeshComponent, SkeletonComponent, primitives},
    physics::{collider::Collider, physics_state::PhysicsState, ragdoll::RagdollId},
    render::{
        MaterialComponent,
        assets::material::StandardMaterial,
        components::light::{Light, LightType},
    },
    ui::{
        material::UIMaterial,
        node::{UINode, UIRect},
        text::{FontFamily, UIText},
        transform::UIValue,
    },
    window::input::{Input, KeyCode, PhysicalKey},
};
use concerto_physics_gizmos::{PhysicsGizmos, PhysicsGizmosPlugin};
use concerto_ragdoll::{
    Balance, BodyDrive, BodyHit, BodySense, CharacterRagdoll, Die, Fall, FallReaction,
    PhysicalState, RagdollBody, Settled, ShotReaction, Stagger, describe_body,
    testing::reference_humanoid,
};
use glam::{Quat, Vec3};

const PUSH: f32 = 70.0;
const HARD_PUSH: f32 = 300.0;
const BULLET: f32 = 40.0;
const WAKING_HIT: f32 = 300.0;
const STAGGER_SEVERITY: f32 = 0.5;
const DYING_SECONDS: f32 = 1.0;
const SKIN: f32 = 0.12;

const HELP: &str = "\
1  balance: stand on its own feet
2  push the chest (70 N.s)
3  push and stagger
4  shot: chest        5  shot: left thigh
6  shot: both thighs  7  shot: left arm
8  hard push (300 N.s): the body falls
9  die over one second
0  head shot: dead at once
F  fall now
H  hard hit on the pelvis: wakes a settled body
D  turn the pushes and shots
G  gizmos             R  start again";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Towards {
    Back,
    Front,
    Left,
    Right,
}

impl Towards {
    fn next(self) -> Self {
        match self {
            Towards::Back => Towards::Front,
            Towards::Front => Towards::Left,
            Towards::Left => Towards::Right,
            Towards::Right => Towards::Back,
        }
    }

    fn direction(self) -> Vec3 {
        match self {
            Towards::Back => Vec3::NEG_Z,
            Towards::Front => Vec3::Z,
            Towards::Left => Vec3::X,
            Towards::Right => Vec3::NEG_X,
        }
    }
}

#[derive(Resource)]
struct Stage {
    character: Entity,
    bones: Vec<Entity>,
    rest: Vec<JointPose>,
    towards: Towards,
}

#[derive(Component)]
struct Report;

#[derive(Clone, Copy)]
enum Struck {
    Chest,
    Head,
    Pelvis,
    LeftThigh,
    RightThigh,
    LeftArm,
}

fn spawn_scene(mut cmd: CommandQueue, assets: Res<AssetServer>) {
    cmd.spawn((
        concerto::director::MainCamera,
        Transform::from_translation_rotation(
            Vec3::new(-1.5, 1.4, 6.0),
            Quat::from_rotation_x(-0.1),
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
    cmd.spawn((
        UINode::default()
            .with_size(UIValue::Px(470.0), UIValue::Px(400.0))
            .with_padding(UIRect::axes(8.0, 12.0))
            .with_margin(UIRect::all(8.0)),
        UIMaterial::flat(Color::rgba(0.0, 0.0, 0.0, 0.6)),
    ))
    .add_child((
        UINode::default().with_flex_grow(1.0),
        UIText {
            text: HELP.to_string(),
            font_size: 13.0,
            line_height: 17.0,
            font_family: FontFamily::Monospace,
            ..Default::default()
        },
        Report,
    ));
    let stage = spawn_humanoid(&mut cmd, &assets, Towards::Back);
    cmd.insert_resource(stage);
}

fn spawn_humanoid(cmd: &mut CommandQueue, assets: &AssetServer, towards: Towards) -> Stage {
    let reference = reference_humanoid();
    let parents = reference.skeleton.parents.to_vec();
    let rest = reference.skeleton.rest_pose.to_vec();
    let skeleton = assets.add(reference.skeleton);
    let mut ragdoll = reference.ragdoll;
    ragdoll.skeleton = skeleton.clone();
    let ragdoll = assets.add(ragdoll);

    let character = cmd.spawn(Transform::default()).entity();
    let mut bones: Vec<Entity> = Vec::with_capacity(rest.len());
    for (index, local) in rest.iter().enumerate() {
        let parent = parents[index].map_or(character, |parent| bones[parent as usize]);
        bones.push(
            cmd.spawn((
                Transform::from_translation_rotation(local.translation, local.rotation),
                ChildOf::new(parent),
            ))
            .entity(),
        );
    }
    cmd.entity(character).insert((
        SkeletonComponent::new(
            skeleton,
            bones.iter().copied().map(SceneEntityRef::Entity).collect(),
            bones.first().copied().map(SceneEntityRef::Entity),
        ),
        CharacterRagdoll::new(ragdoll),
        BodyDrive::default(),
        BodySense::default(),
        PhysicalState::default(),
        ShotReaction::default(),
        FallReaction::default(),
    ));
    Stage {
        character,
        bones,
        rest,
        towards,
    }
}

fn hold_rest_pose(stage: Res<Stage>, transforms: Query<&mut Transform>) {
    for (bone, rest) in stage.bones.iter().zip(&stage.rest) {
        if let Some(mut transform) = transforms.get_entity(*bone) {
            transform.translation = rest.translation;
            transform.rotation = rest.rotation;
        }
    }
}

fn pressed(input: &Input, key: KeyCode) -> bool {
    input.is_just_pressed(PhysicalKey::Code(key))
}

fn start_again(
    mut stage: ResMut<Stage>,
    input: Res<Input>,
    assets: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    if pressed(&input, KeyCode::KeyD) {
        stage.towards = stage.towards.next();
    }
    if pressed(&input, KeyCode::KeyR) {
        cmd.entity(stage.character).despawn_children().despawn();
        *stage = spawn_humanoid(&mut cmd, &assets, stage.towards);
    }
}

fn act(
    stage: Res<Stage>,
    bodies: Query<(&RagdollBody, &RagdollId)>,
    mut physics: ResMut<PhysicsState>,
    input: Res<Input>,
    mut cmd: CommandQueue,
) {
    let character = stage.character;
    let Some((body, &ragdoll)) = bodies.get_entity(character) else {
        return;
    };
    let landmarks = body.resolved().landmarks();
    let along = stage.towards.direction();
    let mut pose = Vec::new();
    physics.ragdoll_pose(ragdoll, &mut pose);
    let part_of = |struck: Struck| match struck {
        Struck::Chest => landmarks.chest,
        Struck::Head => landmarks.head,
        Struck::Pelvis => landmarks.pelvis,
        Struck::LeftThigh => landmarks.left.thigh,
        Struck::RightThigh => landmarks.right.thigh,
        Struck::LeftArm => landmarks.left.upper_arm,
    };
    let hit = |struck: Struck, impulse: Vec3| {
        let part = part_of(struck)?;
        let place = pose.get(part)?.translation;
        Some(BodyHit::new(
            character,
            part,
            place - impulse.normalize_or_zero() * SKIN,
            impulse,
        ))
    };
    let shoot = |struck: Struck, cmd: &mut CommandQueue| {
        if let Some(hit) = hit(struck, along * BULLET) {
            cmd.trigger(hit.shot());
        }
    };

    if pressed(&input, KeyCode::Digit1) {
        cmd.entity(character)
            .insert(Balance::default().with_settle_seconds(f32::MAX));
    }
    let push = pressed(&input, KeyCode::Digit2);
    let stagger = pressed(&input, KeyCode::Digit3);
    let hard = pressed(&input, KeyCode::Digit8);
    if (push || stagger || hard)
        && let Some(chest) = landmarks.chest
    {
        let strength = if hard { HARD_PUSH } else { PUSH };
        let chest = physics.ragdoll_bodies(ragdoll)[chest];
        physics.add_impulse(chest, along * strength);
        if stagger {
            cmd.entity(character)
                .insert(Stagger::new(along, STAGGER_SEVERITY));
        }
    }
    if pressed(&input, KeyCode::Digit4) {
        shoot(Struck::Chest, &mut cmd);
    }
    if pressed(&input, KeyCode::Digit5) {
        shoot(Struck::LeftThigh, &mut cmd);
    }
    if pressed(&input, KeyCode::Digit6) {
        shoot(Struck::LeftThigh, &mut cmd);
        shoot(Struck::RightThigh, &mut cmd);
    }
    if pressed(&input, KeyCode::Digit7) {
        shoot(Struck::LeftArm, &mut cmd);
    }
    if pressed(&input, KeyCode::Digit9) {
        cmd.trigger(Die {
            character,
            seconds: DYING_SECONDS,
        });
    }
    if pressed(&input, KeyCode::Digit0) {
        shoot(Struck::Head, &mut cmd);
        cmd.trigger(Die {
            character,
            seconds: 0.0,
        });
    }
    if pressed(&input, KeyCode::KeyF) {
        cmd.entity(character).insert(Fall::default());
    }
    if pressed(&input, KeyCode::KeyH)
        && let Some(hit) = hit(Struck::Pelvis, (Vec3::Y + along * 0.3) * WAKING_HIT)
    {
        cmd.trigger(hit);
    }
}

fn report(
    stage: Res<Stage>,
    bodies: Query<(
        &RagdollBody,
        &CharacterRagdoll,
        Option<&PhysicalState>,
        Option<&BodyDrive>,
        Option<&BodySense>,
        Option<&Settled>,
    )>,
    texts: Query<&mut UIText, With<Report>>,
) {
    let described = bodies.get_entity(stage.character).map_or_else(
        || "the body is being built".to_string(),
        |(body, character, physical, drive, sense, settled)| {
            format!(
                "mode: {:?}{}\n{}",
                character.mode,
                if settled.is_some() { ", settled" } else { "" },
                describe_body(physical, drive, sense, body)
            )
        },
    );
    let shown = format!(
        "{HELP}\n\npushes and shots travel towards its {:?}\n{described}",
        stage.towards
    );
    for mut text in texts.iter() {
        if text.text != shown {
            text.text = shown.clone();
        }
    }
}

fn toggle_gizmos(input: Res<Input>, mut shown: ResMut<PhysicsGizmos>) {
    if pressed(&input, KeyCode::KeyG) {
        *shown = if *shown == PhysicsGizmos::all() {
            PhysicsGizmos::default()
        } else {
            PhysicsGizmos::all()
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
            (hold_rest_pose, start_again, act, report, toggle_gizmos),
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

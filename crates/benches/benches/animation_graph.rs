use criterion::{criterion_group, criterion_main, Criterion};
use glam::{Mat4, Quat, Vec2, Vec3};
use uuid::Uuid;

use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::AnimationPlugin,
};
use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
    App,
};
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_foundation::{assets::asset_server::AssetServer, transform::Transform};
use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
use concerto_name::name;

const BONES: usize = 60;
const KEYS: usize = 30;
const SIDE: usize = 3;
const CHARACTERS: usize = 16;

fn bone_id(bone: usize) -> Uuid {
    Uuid::from_u128(bone as u128 + 1)
}

fn clip(seed: f32) -> AnimationClip {
    let times: Vec<f32> = (0..KEYS).map(|key| key as f32 / KEYS as f32).collect();
    let mut clip = AnimationClip::default();
    for bone in 0..BONES {
        let phase = seed + bone as f32 * 0.1;
        let translations = times.iter().map(|time| [phase, time * 0.5, 0.0]);
        let rotations = times
            .iter()
            .map(|time| Quat::from_rotation_z(phase + time).to_array());
        clip.add_channel(
            bone_id(bone),
            AnimationChannel::new(
                times.clone(),
                AnimationChanelOutput::from_translation(translations),
            ),
        );
        clip.add_channel(
            bone_id(bone),
            AnimationChannel::new(
                times.clone(),
                AnimationChanelOutput::from_rotation(rotations),
            ),
        );
    }
    clip
}

fn skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; BONES],
        (0..BONES)
            .map(|bone| bone.checked_sub(1).map(|parent| parent as u16))
            .collect(),
        (0..BONES)
            .map(|bone| format!("bone_{bone}").as_str().into())
            .collect(),
        vec![JointPose::IDENTITY; BONES],
        (0..BONES).map(bone_id).collect(),
    )
    .expect("the skeleton is a chain")
}

fn blend_space_graph(server: &AssetServer) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .add_blend_space_2d_input_with(name!("movement"), |space| {
            for point in 0..SIDE * SIDE {
                let at = Vec2::new((point % SIDE) as f32, (point / SIDE) as f32) - Vec2::ONE;
                space.animation_clip_input(server.add(clip(point as f32)), at);
            }
        });
    graph
}

fn crowd() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(AnimationPlugin);
    app.finish_plugin_build();

    let server = app
        .get_resource::<AssetServer>()
        .expect("the asset manager adds the server")
        .clone();
    let graph = server.add(blend_space_graph(&server));
    let skeleton = server.add(skeleton());
    app.update();

    let world = app.main_mut().world_mut();
    for _ in 0..CHARACTERS {
        let bones = (0..BONES)
            .map(|_| SceneEntityRef::Entity(world.spawn(Transform::from_translation(Vec3::ZERO))))
            .collect();
        let mut player = AnimationPlayer::new(BONES);
        player.set_vec2_param(name!("movement"), Vec2::new(0.4, 0.3));
        world.spawn((
            player,
            AnimationHandleComponent::new(graph.clone()),
            SkeletonComponent::new(skeleton.clone(), bones, None),
        ));
    }
    app.update();
    app
}

fn animation_graph(c: &mut Criterion) {
    let mut group = c.benchmark_group("animation_graph");

    group.bench_function("blend_space_9_inputs_60_bones", |b| {
        let mut app = crowd();
        b.iter(|| app.update());
    });

    group.finish();
}

criterion_group!(benches, animation_graph);
criterion_main!(benches);

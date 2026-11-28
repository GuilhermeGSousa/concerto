use std::{f32::consts::FRAC_1_SQRT_2, sync::Arc};

use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_app::{App, plugins::TransformPlugin};
use concerto_ecs::{
    Entity, World,
    component::scene::SceneEntityRef,
    events::{event_channel::EventChannel, event_cursor::EventCursor},
};
use concerto_editor::{
    hosted::testing::plain_recipe, preview::PreviewPlugin, timeline::TimelineScrubbed,
    tools::component_gizmos::ComponentGizmos,
};
use concerto_foundation::{
    assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
    transform::Transform,
};
use concerto_mesh::{Skeleton, SkeletonComponent};
use concerto_motion_matching::{
    BonePositions, FeatureChannelData, JointAxis, MotionDatabase, MotionEntry, TimeRange,
};
use glam::{Quat, Vec3};

use super::*;
use crate::{
    database_preview,
    panels::tests::{Tab, bone_id, skeleton_of, tab_of},
    tests::{fixture, state},
};

const NEAR: f32 = 1e-3;
const ROOT_START: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const ROOT_END: Vec3 = Vec3::new(4.0, 1.0, 0.0);
const SPINE_START: Vec3 = Vec3::new(0.0, 0.5, 0.0);
const SPINE_END: Vec3 = Vec3::new(0.0, 0.75, 0.0);
const ROOT_TURN: Quat = Quat::from_xyzw(0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2);
const RATE: f32 = 30.0;

fn keys(from: Vec3, to: Vec3) -> AnimationChannel {
    AnimationChannel::new(
        vec![0.0, 2.0],
        AnimationChanelOutput::from_translation([from.to_array(), to.to_array()].into_iter()),
    )
}

fn walking() -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(bone_id(0), keys(ROOT_START, ROOT_END));
    clip.add_channel(
        bone_id(0),
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::from_rotation(
                [Quat::IDENTITY.to_array(), ROOT_TURN.to_array()].into_iter(),
            ),
        ),
    );
    clip.add_channel(bone_id(1), keys(SPINE_START, SPINE_END));
    clip
}

fn skeleton() -> Skeleton {
    skeleton_of(&["Hips", "Spine", "Head"])
}

fn entry(range: Option<TimeRange>) -> MotionEntry {
    MotionEntry {
        clip: AssetHandle::weak(AssetId::new()),
        range,
        mirrored: false,
    }
}

fn baked_from(entries: Vec<MotionEntry>, speed: f32) -> BakedPlayback {
    baked_with(entries, speed, Vec::new())
}

fn baked_with(
    entries: Vec<MotionEntry>,
    speed: f32,
    channels: Vec<FeatureChannelData>,
) -> BakedPlayback {
    let mut definition = MotionDatabase {
        skeleton: AssetHandle::weak(AssetId::new()),
        rate: RATE,
        speed,
        channels,
        ..Default::default()
    };
    definition.simulation.position_joint = "Spine".into();
    definition.simulation.facing_axis = JointAxis::Z;
    let clips: Vec<AnimationClip> = entries.iter().map(|_| walking()).collect();
    definition.entries = entries;
    let skeleton = skeleton();
    let sampled: Vec<&AnimationClip> = clips.iter().collect();
    let baked = definition.bake(&skeleton, &sampled).unwrap();
    let database = definition.with_baked(baked);
    let channels: Vec<Box<dyn FeatureChannel>> = database
        .live_channels()
        .unwrap()
        .into_iter()
        .map(|channel| channel.dyn_clone())
        .collect();
    BakedPlayback {
        database: Arc::new(database),
        skeleton: Arc::new(skeleton),
        clips: clips.into_iter().map(Arc::new).collect(),
        channels: channels.into(),
    }
}

fn baked() -> BakedPlayback {
    baked_from(vec![entry(None), entry(None)], 1.0)
}

struct Rig {
    root: Entity,
    spine: Entity,
}

fn spawn_rig(world: &mut World) -> Rig {
    let id = AssetId::new();
    if world.get_resource::<AssetStore<Skeleton>>().is_none() {
        world.insert_resource(AssetStore::<Skeleton>::new());
    }
    world
        .get_resource_mut::<AssetStore<Skeleton>>()
        .unwrap()
        .insert(id, skeleton());
    let parent = world.spawn(Transform::IDENTITY);
    let [root, spine, head] = [(); 3].map(|_| world.spawn(Transform::IDENTITY));
    world.entity_mut(parent).add_child(root);
    world.entity_mut(root).add_child(spine);
    world.entity_mut(spine).add_child(head);
    world.spawn(SkeletonComponent::new(
        AssetHandle::weak(id),
        [root, spine, head].map(SceneEntityRef::Entity).to_vec(),
        Some(SceneEntityRef::Entity(root)),
    ));
    Rig { root, spine }
}

struct Step {
    position: f32,
    delta: f32,
    rate: f32,
    range: Range<usize>,
    stepped: f32,
}

#[test]
fn the_position_steps_at_the_rate_wraps_to_the_start_of_its_range_and_starts_a_range_it_is_outside()
{
    let rows = [
        Step {
            position: 3.0,
            delta: 0.5,
            rate: 30.0,
            range: 0..100,
            stepped: 18.0,
        },
        Step {
            position: 14.5,
            delta: 0.1,
            rate: 10.0,
            range: 10..15,
            stepped: 10.5,
        },
        Step {
            position: 3.0,
            delta: 0.1,
            rate: 10.0,
            range: 10..15,
            stepped: 10.0,
        },
    ];
    for row in rows {
        assert_eq!(
            stepped(row.position, row.delta, row.rate, row.range.clone()),
            row.stepped,
            "{} in {:?}",
            row.position,
            row.range
        );
    }
}

struct Seek {
    database: Arc<MotionDatabase>,
    entry: usize,
    time: f32,
    frame: Option<usize>,
}

#[test]
fn a_seek_lands_on_the_entrys_frame_nearest_the_time_at_the_databases_speed_and_stays_inside_it() {
    let two = baked().database;
    let first = two.ranges().unwrap().next().unwrap();
    let second = two.ranges().unwrap().nth(1).unwrap();
    let slow = baked_from(
        vec![entry(Some(TimeRange {
            start: 0.5,
            end: 2.0,
        }))],
        0.5,
    )
    .database;
    let rows = [
        Seek {
            database: Arc::clone(&two),
            entry: 1,
            time: 1.0,
            frame: Some(second.start + 30),
        },
        Seek {
            database: Arc::clone(&two),
            entry: 0,
            time: 100.0,
            frame: Some(first.end - 1),
        },
        Seek {
            database: Arc::clone(&two),
            entry: 2,
            time: 0.0,
            frame: None,
        },
        Seek {
            database: slow,
            entry: 0,
            time: 1.0,
            frame: Some(30),
        },
    ];
    for row in rows {
        assert_eq!(
            frame_in_entry(&row.database, row.entry, row.time),
            row.frame,
            "entry {} at {}",
            row.entry,
            row.time
        );
    }
}

#[test]
fn a_preview_app_poses_the_sought_entry_with_the_root_placed_by_the_simulation_bone() {
    let mut app = App::for_previewing();
    plain_recipe(&mut app);
    app.register_plugin(TransformPlugin)
        .register_plugin(PreviewPlugin);
    database_preview(&mut app);
    app.finish_plugin_build();
    app.update();
    let world = app.main_mut().world_mut();
    let rig = spawn_rig(world);
    world.trigger(ShowDatabase(Some(baked())));
    world.trigger(SeekDatabase {
        entry: 1,
        time: 1.0,
    });

    app.update();

    let progress: Vec<DatabaseProgress> = app
        .get_resource::<EventChannel<DatabaseProgress>>()
        .unwrap()
        .read_from(&mut EventCursor::default())
        .cloned()
        .collect();
    let shown = progress.last().expect("the playback reports its frame");
    assert_eq!(shown.entry, 1);
    assert!((1.0..1.2).contains(&shown.time), "{}", shown.time);
    let world = app.main().world();
    let posed = |entity: Entity| {
        world
            .get_component_for_entity::<Transform>(entity)
            .unwrap()
            .clone()
    };
    let along = shown.time / 2.0;
    assert!(
        posed(rig.root)
            .translation
            .abs_diff_eq(ROOT_START.lerp(ROOT_END, along), NEAR),
        "{}",
        posed(rig.root).translation
    );
    assert!(
        posed(rig.root)
            .rotation
            .abs_diff_eq(Quat::IDENTITY.slerp(ROOT_TURN, along), NEAR)
    );
    assert!(
        posed(rig.spine)
            .translation
            .abs_diff_eq(SPINE_START.lerp(SPINE_END, along), NEAR)
    );
    assert!(
        world
            .get_resource::<ComponentGizmos>()
            .is_some_and(|gizmos| gizmos.always("Skeleton")),
        "the preview draws its bones"
    );
}

fn derivable(definition: &mut MotionDatabase) {
    definition.simulation.position_joint = "Spine".into();
    definition.channels = vec![FeatureChannelData::live(BonePositions {
        bones: vec!["Head".into()],
        weight: 1.0,
    })];
}

fn preview_frame(tab: &Tab) -> usize {
    tab.f
        .scene_world(tab.document)
        .get_resource::<DatabasePlayback>()
        .unwrap()
        .frame()
}

#[test]
fn a_scrub_seeks_the_derived_frames_of_the_selected_entry() {
    let mut tab = tab_of(fixture(), derivable);
    tab.until_read();
    tab.until_settled();
    tab.state_mut().selected_entry = Some(1);
    tab.f.frame();
    tab.f.frame();

    let timeline = tab.panels().timeline;
    tab.f
        .world
        .trigger_on(timeline, TimelineScrubbed { time: 1.0 });
    tab.f.frame();
    tab.f.frame();

    let database = state(&tab.f, tab.document)
        .playback
        .as_ref()
        .expect("the definition derived a database")
        .database
        .clone();
    let (entry, time) = database.clip_time(preview_frame(&tab)).unwrap();
    assert_eq!(entry, 1);
    assert!((1.0..1.5).contains(&time), "{time}");
}

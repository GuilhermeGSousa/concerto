use std::{f32::consts::FRAC_1_SQRT_2, sync::Arc, time::Duration};

use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_animation_editor::preview::{ClearClip, ShowClip};
use concerto_app::{App, edit_time::AtPreviewTime, plugins::TransformPlugin};
use concerto_debug_gizmos::GizmoStorage;
use concerto_ecs::{
    Entity, IntoSystem, ResMut, Resource, System, World,
    component::scene::SceneEntityRef,
    entity::hierarchy::{ChildOf, Children},
    events::{event_channel::EventChannel, event_cursor::EventCursor},
    signal::On,
    system::BoxedSystem,
};
use concerto_editor::{
    asset_editor::Unsaved,
    guard::DiscardEdits,
    hosted::testing::{SceneFixture, plain_recipe, scene_entry, write_scene_file},
    preview::PreviewPlugin,
    timeline::TimelineScrubbed,
};
use concerto_foundation::{
    assets::{AssetId, asset_store::AssetStore, handle::AssetHandle, registry::AssetEntry},
    time::Time,
    transform::{Transform, systems::propagate_global_transforms},
};
use concerto_mesh::{Skeleton, SkeletonComponent, SkeletonGizmo};
use concerto_motion_matching::{
    BonePositions, BoneVelocities, FeatureChannelData, JointAxis, MotionDatabase, MotionEntry,
    TimeRange, TrajectoryPositions, bake,
};
use glam::{Quat, Vec3};
use uuid::Uuid;

use super::*;
use crate::{
    database_preview,
    panels::tests::{Tab, bone_id, clip_of, skeleton_of, tab_of},
    tests::{database_recipe, fixture_with, save, state, write_asset, write_definition},
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
    let baked = bake(&definition, &skeleton, &sampled).unwrap();
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

fn spine_velocities() -> FeatureChannelData {
    FeatureChannelData::live(BoneVelocities {
        bones: vec!["Spine".into()],
        weight: 1.0,
    })
}

fn trajectory_positions() -> FeatureChannelData {
    FeatureChannelData::live(TrajectoryPositions {
        samples: vec![0, 1, 2],
        weight: 1.0,
    })
}

fn baked() -> BakedPlayback {
    baked_from(vec![entry(None), entry(None)], 1.0)
}

struct Rig {
    owner: Entity,
    root: Entity,
    spine: Entity,
    head: Entity,
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
    let owner = world.spawn(SkeletonComponent::new(
        AssetHandle::weak(id),
        [root, spine, head].map(SceneEntityRef::Entity).to_vec(),
        Some(SceneEntityRef::Entity(root)),
    ));
    Rig {
        owner,
        root,
        spine,
        head,
    }
}

struct Stage {
    world: World,
    play: BoxedSystem,
    propagate: BoxedSystem,
    draw: BoxedSystem,
    features: BoxedSystem,
}

impl Stage {
    fn new() -> Self {
        let mut world = World::new();
        world.register_component::<SkeletonComponent>();
        world.register_component::<Transform>();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(DatabasePlayback::default());
        world.insert_resource(AssetStore::<Skeleton>::new());
        world.insert_resource(Time::new());
        world.insert_resource(GizmoStorage::default());
        world.insert_resource(EventChannel::<DatabaseProgress>::new());
        world.add_listener(show_database);
        world.add_listener(seek_database);
        world.add_listener(show_channel);
        let mut play = play_database.into_boxed_system();
        let mut propagate = propagate_global_transforms.into_boxed_system();
        let mut draw = draw_simulation_bone.into_boxed_system();
        let mut features = draw_feature_channels.into_boxed_system();
        for system in [&mut play, &mut propagate, &mut draw, &mut features] {
            system.initialize(&mut world);
        }
        Self {
            world,
            play,
            propagate,
            draw,
            features,
        }
    }

    fn shown() -> Self {
        let mut stage = Self::new();
        stage.world.trigger(ShowDatabase(Some(baked())));
        stage
    }

    fn frame(&mut self) {
        self.play.run_and_apply((), &mut self.world);
        self.propagate.run_and_apply((), &mut self.world);
        self.draw.run_and_apply((), &mut self.world);
        self.features.run_and_apply((), &mut self.world);
        self.world.tick();
    }

    fn seek(&mut self, entry: usize, time: f32) {
        self.world.trigger(SeekDatabase { entry, time });
    }

    fn playback(&self) -> &DatabasePlayback {
        self.world.get_resource::<DatabasePlayback>().unwrap()
    }

    fn database(&self) -> Arc<MotionDatabase> {
        Arc::clone(&self.playback().shown.as_ref().unwrap().database)
    }

    fn transform(&self, entity: Entity) -> &Transform {
        self.world
            .get_component_for_entity::<Transform>(entity)
            .unwrap()
    }

    fn progress(&self) -> Vec<DatabaseProgress> {
        self.world
            .get_resource::<EventChannel<DatabaseProgress>>()
            .unwrap()
            .read_from(&mut EventCursor::default())
            .cloned()
            .collect()
    }

    fn segments(&self) -> usize {
        self.world
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }
}

#[test]
fn the_position_steps_at_the_rate() {
    assert_eq!(stepped(3.0, 0.5, 30.0, 0..100), 18.0);
}

#[test]
fn the_position_wraps_past_the_last_frame() {
    assert_eq!(stepped(9.5, 0.1, 10.0, 0..10), 0.5);
}

#[test]
fn the_position_wraps_to_the_start_of_its_range() {
    assert_eq!(stepped(14.5, 0.1, 10.0, 10..15), 10.5);
}

#[test]
fn a_position_outside_the_range_starts_it() {
    assert_eq!(stepped(3.0, 0.1, 10.0, 10..15), 10.0);
    assert_eq!(stepped(0.0, 0.1, 10.0, 0..0), 0.0);
}

#[test]
fn with_no_entry_sought_the_whole_database_loops() {
    let database = baked().database;

    assert_eq!(
        looped_range(&database, None),
        0..database.frame_count().unwrap()
    );
    assert_eq!(
        looped_range(&database, Some(7)),
        0..database.frame_count().unwrap()
    );
}

#[test]
fn a_sought_entry_loops_on_itself_and_does_not_run_into_the_next() {
    let mut stage = Stage::shown();
    stage.seek(0, 1.99);
    stage.world.get_resource_mut::<Time>().unwrap().update();
    std::thread::sleep(Duration::from_secs_f32(3.0 / RATE));
    stage.world.get_resource_mut::<Time>().unwrap().update();

    stage.frame();

    let frame = stage.playback().frame();
    assert_eq!(stage.database().entry_of(frame), Some(0));
    assert!(frame < 30, "{frame}");
}

#[test]
fn a_seek_lands_on_the_baked_frame_of_the_entry_nearest_the_time() {
    let database = baked().database;
    let second = database.ranges().unwrap().nth(1).unwrap();

    assert_eq!(frame_in_entry(&database, 0, 1.0), Some(30));
    assert_eq!(frame_in_entry(&database, 1, 1.0), Some(second.start + 30));
}

#[test]
fn a_seek_stays_inside_the_entry() {
    let database = baked().database;
    let first = database.ranges().unwrap().next().unwrap();

    assert_eq!(frame_in_entry(&database, 0, 100.0), Some(first.end - 1));
    assert_eq!(frame_in_entry(&database, 0, -1.0), Some(first.start));
}

#[test]
fn a_seek_past_the_baked_entries_finds_nothing() {
    assert_eq!(frame_in_entry(&baked().database, 2, 0.0), None);
}

#[test]
fn a_seek_counts_clip_seconds_at_the_databases_speed() {
    let range = TimeRange {
        start: 0.5,
        end: 2.0,
    };
    let database = baked_from(vec![entry(Some(range))], 0.5).database;

    assert_eq!(frame_in_entry(&database, 0, 1.0), Some(30));
}

#[test]
fn the_bones_take_the_frames_pose_with_the_root_placed_by_the_simulation_bone() {
    let mut stage = Stage::shown();
    let rig = spawn_rig(&mut stage.world);

    stage.seek(0, 1.0);
    stage.frame();

    assert!(
        stage
            .transform(rig.root)
            .translation
            .abs_diff_eq(ROOT_START.lerp(ROOT_END, 0.5), NEAR)
    );
    assert!(
        stage
            .transform(rig.root)
            .rotation
            .abs_diff_eq(Quat::IDENTITY.slerp(ROOT_TURN, 0.5), NEAR)
    );
    assert!(
        stage
            .transform(rig.spine)
            .translation
            .abs_diff_eq(SPINE_START.lerp(SPINE_END, 0.5), NEAR)
    );
    assert_eq!(stage.transform(rig.head).translation, Vec3::ZERO);
}

#[test]
fn a_skeleton_with_no_stored_asset_is_left_alone() {
    let mut stage = Stage::shown();
    let rig = spawn_rig(&mut stage.world);
    stage.world.insert_resource(AssetStore::<Skeleton>::new());

    stage.seek(0, 1.0);
    stage.frame();

    assert_eq!(stage.transform(rig.root).translation, Vec3::ZERO);
}

#[test]
fn a_bone_the_database_does_not_hold_keeps_its_transform() {
    let mut stage = Stage::shown();
    let rig = spawn_rig(&mut stage.world);
    let owner = stage
        .world
        .get_component_for_entity::<SkeletonComponent>(rig.owner)
        .unwrap()
        .skeleton()
        .clone();
    let mut stranger = skeleton();
    stranger.bone_ids[2] = Uuid::from_u128(99);
    stage
        .world
        .get_resource_mut::<AssetStore<Skeleton>>()
        .unwrap()
        .insert(owner.id(), stranger);
    let moved = Transform::from_translation(Vec3::new(7.0, 8.0, 9.0));
    *stage
        .world
        .get_component_for_entity_mut::<Transform>(rig.head)
        .unwrap() = moved;

    stage.seek(0, 1.0);
    stage.frame();

    assert_eq!(
        stage.transform(rig.head).translation,
        Vec3::new(7.0, 8.0, 9.0)
    );
    assert!(
        stage
            .transform(rig.spine)
            .translation
            .abs_diff_eq(SPINE_START.lerp(SPINE_END, 0.5), NEAR)
    );
}

#[test]
fn nothing_is_posed_with_no_database_shown() {
    let mut stage = Stage::new();
    let rig = spawn_rig(&mut stage.world);

    stage.frame();

    assert_eq!(stage.transform(rig.root).translation, Vec3::ZERO);
    assert!(stage.progress().is_empty());
}

#[test]
fn no_database_stops_the_playback() {
    let mut stage = Stage::shown();
    let rig = spawn_rig(&mut stage.world);
    stage.seek(0, 1.0);

    stage.world.trigger(ShowDatabase(None));
    stage.frame();

    assert_eq!(stage.transform(rig.root).translation, Vec3::ZERO);
    assert_eq!(stage.segments(), 0);
}

#[test]
fn showing_the_database_again_keeps_the_frame() {
    let mut stage = Stage::shown();
    stage.seek(1, 1.0);
    let frame = stage.playback().frame();

    stage.world.trigger(ShowDatabase(Some(baked())));

    assert_ne!(frame, 0);
    assert_eq!(stage.playback().frame(), frame);
}

#[test]
fn a_shorter_database_starts_over() {
    let mut stage = Stage::shown();
    stage.seek(1, 1.0);

    stage
        .world
        .trigger(ShowDatabase(Some(baked_from(vec![entry(None)], 1.0))));

    assert_eq!(stage.playback().frame(), 0);
}

#[test]
fn progress_is_written_once_per_frame_shown_with_the_entry_and_the_clip_time() {
    let mut stage = Stage::shown();
    let second = stage.database().ranges().unwrap().nth(1).unwrap().start;

    stage.frame();
    stage.frame();
    stage.seek(1, 0.5);
    stage.frame();

    let progress = stage.progress();
    assert_eq!(progress.len(), 2);
    assert_eq!(
        progress[0],
        DatabaseProgress {
            entry: 0,
            time: stage.database().clip_time(0).unwrap().1
        }
    );
    assert_eq!(progress[1].entry, 1);
    assert_eq!(
        progress[1].time,
        stage.database().clip_time(second + 15).unwrap().1
    );
}

#[test]
fn the_simulation_bone_is_drawn_while_a_database_plays() {
    let mut stage = Stage::shown();

    stage.frame();

    assert!(stage.segments() > 0);
}

#[test]
fn a_bone_velocity_starts_at_the_bone_of_the_shown_frame() {
    let shown = baked_with(vec![entry(None)], 1.0, vec![spine_velocities()]);

    let shapes = frame_shapes(&shown, 6).unwrap();

    let [FeatureShape::Arrow { start, .. }] = shapes[0][..] else {
        panic!("{shapes:?}");
    };
    assert!(start.distance(Vec3::new(0.0, 1.525, 0.0)) < NEAR, "{start}");
}

#[test]
fn the_shapes_draw_the_features_of_the_shown_frame() {
    let shown = baked_with(vec![entry(None)], 1.0, vec![trajectory_positions()]);

    let shapes = frame_shapes(&shown, 30).unwrap();

    let features = shown.database.features(30).unwrap();
    let FeatureShape::Line { end, .. } = shapes[0][0] else {
        panic!("{shapes:?}");
    };
    assert_eq!(end, Vec3::new(features[0], 0.0, features[1]));
}

#[test]
fn every_channel_is_drawn_while_a_database_plays() {
    let drawn = |channels: Vec<FeatureChannelData>| {
        let mut stage = Stage::new();
        let shown = baked_with(vec![entry(None)], 1.0, channels);
        stage.world.trigger(ShowDatabase(Some(shown)));
        stage.frame();
        stage.segments()
    };

    let none = drawn(Vec::new());
    let one = drawn(vec![spine_velocities()]);
    let both = drawn(vec![spine_velocities(), trajectory_positions()]);

    assert!(one > none);
    assert!(both > one);
}

#[test]
fn the_preview_keeps_the_channel_it_was_told_about() {
    let mut stage = Stage::shown();

    stage.world.trigger(ShowChannel(Some(2)));

    assert_eq!(stage.playback().picked, Some(2));
}

#[test]
fn no_simulation_bone_is_drawn_with_no_database() {
    let mut stage = Stage::new();

    stage.frame();

    assert_eq!(stage.segments(), 0);
}

#[test]
fn a_preview_app_plays_the_database_in_its_schedule() {
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

    std::thread::sleep(Duration::from_secs_f32(2.0 / RATE));
    app.update();

    let world = app.main_mut().world_mut();
    assert!(world.get_resource::<DatabasePlayback>().unwrap().frame() > 0);
    assert!(
        world
            .get_component_for_entity::<Transform>(rig.root)
            .unwrap()
            .translation
            .y
            > 0.5
    );
    assert!(
        world
            .get_component_for_entity::<SkeletonGizmo>(rig.owner)
            .is_some()
    );
}

#[derive(Clone, Debug, PartialEq)]
enum Played {
    Clip(f32),
    Cleared,
    Database,
    NoDatabase,
}

#[derive(Resource, Default)]
struct PlayedLog(Vec<Played>);

fn note_clip(on: On<ShowClip>, mut log: ResMut<PlayedLog>) {
    log.0.push(Played::Clip(on.signal().0.duration()));
}

fn note_cleared(_: On<ClearClip>, mut log: ResMut<PlayedLog>) {
    log.0.push(Played::Cleared);
}

fn note_database(on: On<ShowDatabase>, mut log: ResMut<PlayedLog>) {
    log.0.push(match on.signal().0 {
        Some(_) => Played::Database,
        None => Played::NoDatabase,
    });
}

fn logging_recipe(app: &mut App) {
    database_recipe(app);
    app.insert_resource(PlayedLog::default())
        .add_listener(note_clip.at_preview_time())
        .add_listener(note_cleared.at_preview_time())
        .add_listener(note_database.at_preview_time());
}

fn logging() -> SceneFixture {
    fixture_with(logging_recipe)
}

fn derivable(definition: &mut MotionDatabase) {
    definition.simulation.position_joint = "Spine".into();
    definition.channels = vec![FeatureChannelData::live(BonePositions {
        bones: vec!["Head".into()],
        weight: 1.0,
    })];
}

fn ready(f: SceneFixture) -> Tab {
    let mut tab = tab_of(f, derivable);
    tab.until_read();
    tab.until_settled();
    tab
}

fn underived(f: SceneFixture) -> Tab {
    let mut tab = tab_of(f, |definition| {
        derivable(definition);
        definition.simulation.position_joint = "Nowhere".into();
    });
    tab.until_read();
    tab.until_settled();
    tab
}

fn played(tab: &Tab) -> Vec<Played> {
    tab.f
        .scene_world(tab.document)
        .get_resource::<PlayedLog>()
        .unwrap()
        .0
        .clone()
}

fn preview_frame(tab: &Tab) -> usize {
    tab.f
        .scene_world(tab.document)
        .get_resource::<DatabasePlayback>()
        .unwrap()
        .frame()
}

fn preview_shows_a_database(tab: &Tab) -> bool {
    tab.f
        .scene_world(tab.document)
        .get_resource::<DatabasePlayback>()
        .unwrap()
        .shown
        .is_some()
}

fn picked_in_preview(tab: &Tab) -> Option<usize> {
    tab.f
        .scene_world(tab.document)
        .get_resource::<DatabasePlayback>()
        .unwrap()
        .picked
}

fn scrub(tab: &mut Tab, time: f32) {
    let timeline = tab.panels().timeline;
    tab.f.world.trigger_on(timeline, TimelineScrubbed { time });
}

fn scene_at(f: &mut SceneFixture, address: &str, node: &str) -> AssetEntry {
    let asset = AssetEntry {
        address: address.into(),
        ..scene_entry("fixture")
    };
    let path = f.project_root.join(address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_scene_file(&f.project_root, &asset, &[node]);
    f.add_to_project(&asset);
    asset
}

#[test]
fn the_preview_shows_the_scene_of_the_skeletons_folder() {
    let mut f = logging();
    scene_at(&mut f, "content/hero/scene.gasset", "hero");
    scene_at(&mut f, "content/other/scene.gasset", "stranger");

    let mut tab = ready(f);
    let document = tab.document;

    tab.f
        .frames_until("the scene never reached the preview", |f| {
            f.scene_names(document).contains(&"hero".to_owned())
        });
    assert!(!tab.f.scene_names(document).contains(&"stranger".to_owned()));
}

#[test]
fn a_scene_that_cannot_be_read_says_so_under_the_picker() {
    let mut f = logging();
    let scene = scene_at(&mut f, "content/hero/scene.gasset", "hero");
    std::fs::write(f.project_root.join(&scene.address), b"not a scene").unwrap();

    let tab = ready(f);

    let error = tab
        .state()
        .shown_skeleton
        .as_ref()
        .unwrap()
        .error
        .as_deref();
    assert!(
        error.is_some_and(|error| error.contains("content asset")),
        "{error:?}"
    );
}

#[test]
fn with_no_derived_data_the_selected_entrys_clip_plays_and_the_timeline_follows_it() {
    let mut tab = underived(logging());
    let document = tab.document;

    assert_eq!(played(&tab).last(), Some(&Played::Clip(10.0)));
    tab.f
        .frames_until("the head never moved", |f| state(f, document).time > 0.0);
    tab.f.frame();
    assert_eq!(tab.timeline().time, tab.state().time);
}

#[test]
fn selecting_another_entry_plays_its_clip() {
    let mut tab = underived(logging());

    tab.state_mut().selected_entry = Some(1);
    tab.until_settled();

    assert_eq!(played(&tab).last(), Some(&Played::Clip(2.0)));
}

#[test]
fn no_clip_is_wanted_while_a_derive_is_wanted() {
    let mut state = DefinitionDocument {
        entries: vec![entry(None)],
        selected_entry: Some(0),
        derive_wanted: true,
        ..Default::default()
    };

    assert_eq!(wanted_clip(&state), None);

    state.derive_wanted = false;

    assert_eq!(wanted_clip(&state), Some(state.entries[0].clip.id()));
}

#[test]
fn a_clip_that_cannot_be_read_empties_the_preview() {
    let mut tab = underived(logging());
    let run = tab.f.project_root.join(&tab.run.address);
    std::fs::remove_file(run).unwrap();

    tab.state_mut().selected_entry = Some(1);
    tab.until_settled();

    assert_eq!(played(&tab).last(), Some(&Played::Cleared));
}

#[test]
fn no_entry_clears_the_clip() {
    let mut tab = underived(logging());

    tab.state_mut().selected_entry = None;
    tab.until_settled();

    assert_eq!(played(&tab).last(), Some(&Played::Cleared));
}

#[test]
fn an_opened_definition_plays_its_derived_database_and_never_a_clip() {
    let tab = ready(logging());

    let played = played(&tab);
    assert!(played.contains(&Played::Database), "{played:?}");
    assert!(
        !played.iter().any(|shown| matches!(shown, Played::Clip(_))),
        "{played:?}"
    );
    assert!(preview_shows_a_database(&tab));
    assert!(tab.state().playback.is_some());
}

#[test]
fn a_definition_opened_again_never_plays_a_clip() {
    let mut tab = ready(logging());
    let document = tab.document;
    let first = tab.asset.clone();
    let other = write_definition(
        &mut tab.f,
        "content/hero/other.gasset",
        &MotionDatabase::default(),
    );
    tab.f.request(&other);
    tab.f
        .frames_until("the other definition never opened", |f| {
            f.document(document).current.as_ref() == Some(&other)
        });
    tab.until_settled();
    let before = played(&tab).len();

    tab.f.request(&first);
    tab.f.frames_until("the definition never reopened", |f| {
        f.document(document).current.as_ref() == Some(&first)
    });
    tab.until_settled();

    assert!(tab.state().playback.is_some());
    assert!(
        !played(&tab)[before..]
            .iter()
            .any(|shown| matches!(shown, Played::Clip(_))),
        "{:?}",
        played(&tab)
    );
}

#[test]
fn a_definition_that_derives_nothing_stops_the_database() {
    let mut tab = ready(logging());
    let document = tab.document;
    let other = write_definition(
        &mut tab.f,
        "content/hero/other.gasset",
        &MotionDatabase::default(),
    );

    tab.f.request(&other);
    tab.f
        .frames_until("the other definition never opened", |f| {
            f.document(document).current.as_ref() == Some(&other)
        });
    tab.until_settled();

    assert!(tab.state().playback.is_none());
    assert!(!preview_shows_a_database(&tab));
}

#[test]
fn another_skeleton_stops_the_database_and_plays_the_clip() {
    let mut tab = ready(logging());
    let other = write_asset(
        &mut tab.f,
        "content/other/skeleton.gasset",
        &skeleton_of(&["Hips", "Spine", "Head"]),
    );

    tab.state_mut().skeleton = AssetHandle::weak(other.id);
    tab.until_read();
    tab.until_settled();

    assert!(tab.state().playback.is_none());
    assert!(!preview_shows_a_database(&tab));
    assert_eq!(played(&tab).last(), Some(&Played::Clip(10.0)));
}

#[test]
fn a_derive_that_cannot_read_a_clip_falls_back_to_the_selected_clip() {
    let mut tab = ready(logging());
    let document = tab.document;
    std::fs::remove_file(tab.f.project_root.join(&tab.run.address)).unwrap();

    save(&mut tab.f, document);
    tab.until_settled();

    assert!(!preview_shows_a_database(&tab));
    assert_eq!(played(&tab).last(), Some(&Played::Clip(10.0)));
}

#[test]
fn a_saved_entry_removal_plays_the_database_without_it() {
    let mut tab = ready(logging());
    let document = tab.document;

    tab.state_mut().entries.remove(0);
    tab.state_mut().selected_entry = Some(0);
    tab.f.world.insert(Unsaved, document);
    save(&mut tab.f, document);
    tab.until_settled();

    let playback = tab.state().playback.as_ref().expect("it still plays");
    assert_eq!(playback.clips.len(), 1);
    assert!(preview_shows_a_database(&tab));
}

#[test]
fn scrubbing_seeks_the_clip() {
    let mut tab = underived(logging());
    let document = tab.document;

    scrub(&mut tab, 5.0);
    tab.f.frames_until("the scrub never came back", |f| {
        state(f, document).time >= 5.0
    });

    assert!(tab.state().time < 6.0, "{}", tab.state().time);
}

#[test]
fn scrubbing_seeks_the_baked_frames_of_the_selected_entry() {
    let mut tab = ready(logging());
    tab.state_mut().selected_entry = Some(1);
    tab.f.frame();
    tab.f.frame();

    scrub(&mut tab, 1.0);
    tab.f.frame();
    tab.f.frame();

    let database = tab.state().playback.as_ref().unwrap().database.clone();
    let (entry, time) = database.clip_time(preview_frame(&tab)).unwrap();
    assert_eq!(entry, 1);
    assert!((1.0..1.5).contains(&time), "{time}");
}

#[test]
fn a_scrub_shows_on_the_timeline_at_once() {
    let mut tab = ready(logging());

    scrub(&mut tab, 5.0);

    assert_eq!(tab.state().time, 5.0);
}

#[test]
fn selecting_an_entry_moves_the_baked_playback_to_it() {
    let mut tab = ready(logging());

    tab.state_mut().selected_entry = Some(1);
    tab.f.frame();
    tab.f.frame();

    let database = tab.state().playback.as_ref().unwrap().database.clone();
    assert_eq!(database.entry_of(preview_frame(&tab)), Some(1));
}

#[test]
fn the_timeline_follows_the_baked_frames_of_the_selected_entry() {
    let mut tab = ready(logging());
    let document = tab.document;

    scrub(&mut tab, 1.5);
    tab.f
        .frames_until("the head never moved", |f| state(f, document).time > 1.5);

    let database = tab.state().playback.as_ref().unwrap().database.clone();
    assert_eq!(database.entry_of(preview_frame(&tab)), Some(0));
}

#[test]
fn the_timeline_head_comes_back_without_the_playback_leaving_the_selected_entry() {
    let mut tab = ready(logging());
    let document = tab.document;
    let database = tab.state().playback.as_ref().unwrap().database.clone();
    let last = database.ranges().unwrap().next().unwrap().end - 1;
    let (_, end) = database.clip_time(last).unwrap();

    scrub(&mut tab, end);

    tab.f.frames_until("the head never came back", |f| {
        let frame = f
            .scene_world(document)
            .get_resource::<DatabasePlayback>()
            .unwrap()
            .frame();
        assert_eq!(database.entry_of(frame), Some(0));
        state(f, document).time < end
    });
}

#[test]
fn the_timeline_ignores_frames_of_other_entries() {
    let mut tab = ready(logging());
    tab.state_mut().selected_entry = Some(1);
    tab.f.frame();
    tab.f.frame();
    tab.state_mut().time = 0.25;

    tab.f
        .scene_world_mut(tab.document)
        .get_resource_mut::<EventChannel<DatabaseProgress>>()
        .unwrap()
        .push_event(DatabaseProgress {
            entry: 0,
            time: 9.0,
        });
    tab.f.frame();

    assert_ne!(tab.state().time, 9.0);
}

#[test]
fn an_entry_whose_clip_moved_since_the_build_is_not_sought() {
    let mut tab = ready(logging());
    let walk = tab.state().entries[0].clip.clone();
    tab.state_mut().entries[1].clip = walk;

    assert_eq!(baked_entry(tab.state()), Some(0));
    tab.state_mut().selected_entry = Some(1);

    assert_eq!(baked_entry(tab.state()), None);
}

#[test]
fn an_entry_mirrored_since_the_build_is_not_sought() {
    let mut tab = ready(logging());
    tab.state_mut().selected_entry = Some(1);
    assert_eq!(baked_entry(tab.state()), Some(1));

    tab.state_mut().entries[1].mirrored = true;

    assert_eq!(baked_entry(tab.state()), None);
}

#[test]
fn a_clip_being_read_marks_the_document() {
    let mut tab = ready(logging());
    let (release, wait) = std::sync::mpsc::channel::<()>();
    tab.state_mut().clip_job = Some(ClipJob {
        handle: std::thread::spawn(move || {
            let _ = wait.recv();
            Ok(clip_of(1, 0..1))
        }),
    });

    tab.f.frame();

    assert!(tab.working());
    release.send(()).unwrap();
    tab.until_settled();
    assert!(!tab.working());
}

#[test]
fn picking_a_channel_row_tells_the_preview_and_settings_clears_it() {
    let mut tab = ready(fixture_with(database_recipe));
    assert_eq!(picked_in_preview(&tab), None);

    let row = tab.channel_row_entity(0);
    tab.clicked(row);
    tab.f.frame();
    assert_eq!(picked_in_preview(&tab), Some(0));

    let settings = tab.settings_row();
    tab.clicked(settings);
    tab.f.frame();
    assert_eq!(picked_in_preview(&tab), None);
}

#[test]
fn a_channel_whose_type_differs_from_the_baked_one_is_not_highlighted() {
    let mut tab = ready(fixture_with(database_recipe));
    let row = tab.channel_row_entity(0);
    tab.clicked(row);
    tab.f.frame();

    tab.f
        .world
        .get_component_for_entity_mut::<DefinitionDocument>(tab.document)
        .unwrap()
        .channels[0]
        .type_name = "SomethingElse".into();
    tab.f.frame();
    tab.f.frame();

    assert_eq!(picked_in_preview(&tab), None);
}

#[test]
fn discarding_another_skeleton_plays_the_database_again() {
    let mut tab = ready(logging());
    let document = tab.document;
    let other = write_asset(
        &mut tab.f,
        "content/other/skeleton.gasset",
        &skeleton_of(&["Hips", "Spine", "Head"]),
    );
    tab.state_mut().skeleton = AssetHandle::weak(other.id);
    tab.f.world.insert(Unsaved, document);
    tab.until_read();
    tab.until_settled();
    assert!(!preview_shows_a_database(&tab));

    tab.f.world.trigger(DiscardEdits(document));
    tab.until_read();
    tab.until_settled();

    assert!(tab.state().playback.is_some());
    assert!(preview_shows_a_database(&tab));
}

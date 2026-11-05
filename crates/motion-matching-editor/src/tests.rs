use std::{
    any::{Any, TypeId},
    sync::Arc,
    time::{Duration, Instant},
};

use concerto_animation::clip::AnimationClip;
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
};
use concerto_ecs::{
    Entity, IntoSystem, Res, System, command::CommandQueue, events::event_channel::EventChannel,
};
use concerto_editor::{
    EditorPlugin,
    asset_editor::{
        ActiveEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry, EditorDocument,
        SaveRequested, Unsaved,
    },
    guard::DiscardEdits,
    hosted::{
        HostedApp, HostedPlugin,
        input::{ViewportShown, ViewportView},
        testing::*,
    },
    inspector::{
        InspectedSnapshots, InspectedValues, InspectorRegistry, PropertyCommits, PropertyRow,
        PropertyTarget, ShownValue, ValueCard, commit_number, spawn_panel, value_systems,
    },
    preview::{PreviewFailed, PreviewHostPlugin},
    project::{AssetsWritten, Project, ProjectState},
    scene::{SaveFailed, ScenePlugin, SceneRecipe, SceneSaved},
};
use concerto_foundation::assets::{
    Asset, AssetId,
    asset_server::AssetServer,
    content::save_content_asset_into,
    derived::DerivedDataCache,
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::JointName;
use concerto_motion_matching::{
    BonePositions, BoneVelocities, FeatureChannel, FeatureChannelData, FrameContext, MirrorAxis,
    MotionDatabase, MotionEntry, ReflectFeatureChannel, TimeRange, TrajectoryDirections,
    TrajectoryPositions,
};
use concerto_reflect::{Reflect, ReflectedValue};
use concerto_ui::{node::UINode, theme::UITheme};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::Vec2;

use crate::{
    DatabaseSettings, DefinitionDocument, MotionDatabaseEditor,
    derive::{derive_definitions, follow_written_inputs, mark_background_work},
    document::{
        clear_status_on_edit, follow_definition_edits, load_definition, restore_discarded_edits,
    },
    facts::{follow_skeleton, read_entry_facts},
    install,
    joint::{JointNameEditor, refresh_joint_rows},
    menu::close_name_menu,
    panels::{
        build_definition_panels, sync_channel_rows, sync_definition_panels,
        tests::{clip_of, skeleton_of},
    },
    playback::{follow_playback, follow_preview_time},
    read_definition,
    save::save_definitions,
};

#[derive(Reflect, Default, Clone)]
#[reflect(FeatureChannel)]
struct Unplaced;

impl FeatureChannel for Unplaced {
    fn dims(&self) -> usize {
        0
    }

    fn extract(&self, _: &FrameContext, _: &mut [f32]) {}
}

#[derive(Reflect, Default, Clone)]
#[reflect(FeatureChannel)]
struct Settings;

impl FeatureChannel for Settings {
    fn dims(&self) -> usize {
        0
    }

    fn extract(&self, _: &FrameContext, _: &mut [f32]) {}
}

#[derive(Reflect, Default)]
struct NoChannel;

pub(crate) fn database_recipe(app: &mut App) {
    scene_recipe(app);
    app.register_type::<BonePositions>()
        .register_type::<BoneVelocities>()
        .register_type::<TrajectoryPositions>()
        .register_type::<TrajectoryDirections>()
        .register_type::<Unplaced>()
        .register_type::<NoChannel>();
}

fn settings_channel_recipe(app: &mut App) {
    database_recipe(app);
    app.register_type::<Settings>();
}

pub(crate) fn broken_recipe(_: &mut App) {
    panic!("the game broke");
}

fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let parent = cmd.spawn(UINode::default()).entity();
    spawn_panel(&mut cmd, parent, &theme);
}

pub(crate) fn fixture_with(recipe: fn(&mut App)) -> SceneFixture {
    let mut f = SceneFixture::new();
    f.world
        .get_resource::<AssetServer>()
        .unwrap()
        .set_derived_cache(DerivedDataCache::in_project(&f.project_root));
    f.world
        .insert_resource(EventChannel::<AssetsWritten>::new());
    f.world.insert_resource(SceneRecipe {
        recipe,
        gpu: None,
        clipboard: None,
    });
    f.world
        .get_resource_mut::<AssetEditorRegistry>()
        .unwrap()
        .register_creatable::<MotionDatabase, MotionDatabaseEditor>()
        .unwrap();
    let mut inspector = InspectorRegistry::default();
    inspector.register_property_editor::<JointName, JointNameEditor>();
    f.world.insert_resource(inspector);
    f.world.insert_resource(InspectedSnapshots::default());
    let mut panel = build_inspector.into_system();
    panel.initialize(&mut f.world);
    panel.run_and_apply((), &mut f.world);
    f.world.add_listener(clear_status_on_edit);
    f.world.add_listener(restore_discarded_edits);
    let mut value_systems = value_systems();
    let cards = value_systems.split_off(1);
    for system in value_systems {
        f.add_delivery(system);
    }
    f.add_delivery(follow_definition_edits.into_boxed_system());
    f.add_delivery(save_definitions.into_boxed_system());
    f.add_delivery(load_definition.into_boxed_system());
    f.add_delivery(follow_skeleton.into_boxed_system());
    f.add_delivery(read_entry_facts.into_boxed_system());
    f.add_delivery(follow_written_inputs.into_boxed_system());
    f.add_delivery(derive_definitions.into_boxed_system());
    f.add_delivery(follow_playback.into_boxed_system());
    f.add_delivery(follow_preview_time.into_boxed_system());
    f.add_delivery(mark_background_work.into_boxed_system());
    f.add_delivery(build_definition_panels.into_boxed_system());
    f.add_delivery(sync_definition_panels.into_boxed_system());
    f.add_delivery(sync_channel_rows.into_boxed_system());
    f.add_delivery(close_name_menu.into_boxed_system());
    for system in cards {
        f.add_delivery(system);
    }
    f.add_delivery(refresh_joint_rows.into_boxed_system());
    f
}

pub(crate) fn fixture() -> SceneFixture {
    fixture_with(database_recipe)
}

pub(crate) fn entry(seconds: f32) -> MotionEntry {
    MotionEntry {
        clip: AssetHandle::weak(AssetId::new()),
        range: Some(TimeRange {
            start: 0.0,
            end: seconds,
        }),
        mirrored: false,
    }
}

pub(crate) fn definition() -> MotionDatabase {
    MotionDatabase {
        skeleton: AssetHandle::weak(AssetId::new()),
        entries: vec![entry(1.0), entry(2.0)],
        rate: 30.0,
        speed: 1.25,
        ..Default::default()
    }
}

pub(crate) fn write_asset<A: Asset>(f: &mut SceneFixture, address: &str, asset: &A) -> AssetEntry {
    let root = f.project_root.clone();
    let assets = f.world.get_resource_mut::<AssetRegistry>().unwrap();
    let id = save_content_asset_into(asset, &root, address, assets).unwrap();
    assets.get(id).unwrap().clone()
}

pub(crate) fn write_definition(
    f: &mut SceneFixture,
    address: &str,
    definition: &MotionDatabase,
) -> AssetEntry {
    write_asset(f, address, definition)
}

pub(crate) fn request(f: &mut SceneFixture, asset: &AssetEntry) -> Entity {
    f.request(asset);
    let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
    f.frames_until("the definition never finished opening", |f| {
        f.document(document).pending.is_none()
    });
    document
}

pub(crate) fn open(f: &mut SceneFixture, asset: &AssetEntry) -> Entity {
    let document = request(f, asset);
    f.frame();
    f.frame();
    f.frames_until("the derive never ended", |f| {
        let state = state(f, document);
        !state.derive_wanted && state.derive_job.is_none()
    });
    document
}

pub(crate) fn state(f: &SceneFixture, document: Entity) -> &DefinitionDocument {
    f.world
        .get_component_for_entity::<DefinitionDocument>(document)
        .expect("the document is a definition's")
}

fn state_mut(f: &mut SceneFixture, document: Entity) -> &mut DefinitionDocument {
    f.world
        .get_component_for_entity_mut::<DefinitionDocument>(document)
        .expect("the document is a definition's")
}

pub(crate) fn value_names(f: &SceneFixture, document: Entity) -> Vec<String> {
    f.world
        .get_component_for_entity::<InspectedValues>(document)
        .map_or_else(Vec::new, |values| {
            values.0.iter().map(|value| value.name()).collect()
        })
}

fn holds_an_empty_list_of_values(f: &SceneFixture, document: Entity) -> bool {
    f.world
        .get_component_for_entity::<InspectedValues>(document)
        .is_some_and(|values| values.0.is_empty())
}

pub(crate) fn shown_name(f: &SceneFixture, document: Entity) -> Option<String> {
    f.world
        .get_component_for_entity::<ShownValue>(document)
        .map(|shown| shown.0.name())
}

pub(crate) fn card_names(f: &mut SceneFixture) -> Vec<String> {
    let world = &mut f.world;
    world
        .query::<&ValueCard, ()>()
        .iter(world)
        .map(|card| card.name.clone())
        .collect()
}

fn value_named(f: &SceneFixture, document: Entity, name: &str) -> Arc<dyn PropertyTarget> {
    f.world
        .get_component_for_entity::<InspectedValues>(document)
        .and_then(|values| values.0.iter().find(|value| value.name() == name))
        .cloned()
        .expect("the document publishes the value")
}

pub(crate) fn show(f: &mut SceneFixture, document: Entity, name: &str) {
    let value = value_named(f, document, name);
    f.world.insert(ShownValue(value), document);
    f.frame();
    f.frame();
    f.frame();
}

pub(crate) fn row_of(
    f: &mut SceneFixture,
    document: Entity,
    name: &str,
    path: &str,
) -> PropertyRow {
    show(f, document, name);
    let world = &mut f.world;
    world
        .query::<&PropertyRow, ()>()
        .iter(world)
        .find(|row| row.address.target.name() == name && row.address.path.to_string() == path)
        .cloned()
        .expect("the card shows the row")
}

pub(crate) fn queue_edit(
    f: &mut SceneFixture,
    document: Entity,
    name: &str,
    path: &str,
    number: f64,
) {
    let row = row_of(f, document, name, path);
    let commits = f.world.get_resource_mut::<PropertyCommits>().unwrap();
    commit_number(commits, &row, number).unwrap();
}

pub(crate) fn edit(f: &mut SceneFixture, document: Entity, name: &str, path: &str, number: f64) {
    queue_edit(f, document, name, path, number);
    f.frame();
}

pub(crate) fn channel_at<T: FeatureChannel>(
    f: &SceneFixture,
    document: Entity,
    index: usize,
) -> &T {
    let channel: &dyn Any = state(f, document).channels[index].value.as_ref();
    channel.downcast_ref().expect("the channel is of this type")
}

pub(crate) fn save(f: &mut SceneFixture, document: Entity) {
    f.world.insert(SaveRequested, document);
    f.deliver();
}

#[test]
fn the_testing_feature_opens_a_scene_in_an_extension_crate() {
    let mut fixture = SceneFixture::new();

    let document = fixture.open();

    assert!(
        fixture
            .scene_names(document)
            .contains(&"second".to_string())
    );
}

#[test]
fn install_registers_a_creatable_editor_with_a_preview() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(EditorPlugin {
            project: None,
            decorated: false,
            recipe: plain_recipe,
        });

    install(&mut app);

    let editors = app.get_resource::<AssetEditorRegistry>().unwrap();
    let kind = MotionDatabase::name();
    assert!(editors.preview(kind).is_some());
    assert!(editors.bottom_host(kind));
    assert!(editors.creatable().any(|(creatable, _)| creatable == kind));
}

#[test]
fn install_alone_plays_a_clip_in_a_definitions_preview_and_registers_no_clip_editor() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut assets = AssetRegistry::default();
    let skeleton = skeleton_of(&["Hips", "Spine", "Head"]);
    let skeleton =
        save_content_asset_into(&skeleton, root, "content/hero/skeleton.gasset", &mut assets);
    let walk = clip_of(10, 0..3);
    let walk = save_content_asset_into(&walk, root, "content/hero/walk.gasset", &mut assets);
    let opened = MotionDatabase {
        skeleton: AssetHandle::weak(skeleton.unwrap()),
        entries: vec![MotionEntry {
            clip: AssetHandle::weak(walk.unwrap()),
            range: None,
            mirrored: false,
        }],
        ..Default::default()
    };
    let id = save_content_asset_into(&opened, root, "content/hero/database.gasset", &mut assets);
    let asset = assets.get(id.unwrap()).unwrap().clone();
    let mut project = ProjectState::default();
    project.project = Some(Project {
        root: root.into(),
        content_root: "content".into(),
    });
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(EditorPlugin {
            project: None,
            decorated: false,
            recipe: database_recipe,
        })
        .insert_resource(ViewportShown(Some(ViewportView {
            origin: Vec2::ZERO,
            size: (64, 64),
            over: false,
            typing: false,
        })));

    install(&mut app);
    app.finish_plugin_build();

    let editors = app.get_resource::<AssetEditorRegistry>().unwrap();
    assert_eq!(editors.editor_kind(AnimationClip::name()), None);
    app.insert_resource(project).insert_resource(assets);
    let world = app.main_mut().world_mut();
    world
        .get_resource_mut::<AssetEditorCommands>()
        .unwrap()
        .0
        .push_back(AssetEditorCommand::Open(asset));

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.update();
        let world = app.main_mut().world_mut();
        let played = world
            .query::<&DefinitionDocument, ()>()
            .iter(world)
            .any(|state| state.time > 0.0);
        if played {
            break;
        }
        assert!(Instant::now() < deadline, "the head never moved");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn opening_a_definition_holds_its_settings_and_its_channels_in_order() {
    let mut f = fixture();
    let opened = definition();
    let asset = write_definition(&mut f, "content/database.gasset", &opened);

    let document = open(&mut f, &asset);

    assert_eq!(
        state(&f, document).settings,
        Some(DatabaseSettings::of(&opened))
    );
    assert_eq!(state(&f, document).channels.len(), 5);
    assert_eq!(
        channel_at::<BonePositions>(&f, document, 0).bones,
        [JointName::from("LeftFoot"), "RightFoot".into()]
    );
    assert_eq!(
        channel_at::<BoneVelocities>(&f, document, 1).bones,
        [JointName::from("LeftFoot"), "RightFoot".into()]
    );
    assert_eq!(
        channel_at::<BoneVelocities>(&f, document, 2).bones,
        [JointName::from("Hips")]
    );
    assert_eq!(
        channel_at::<TrajectoryPositions>(&f, document, 3).samples,
        [0, 1, 2]
    );
    assert_eq!(
        channel_at::<TrajectoryDirections>(&f, document, 4).weight,
        1.5
    );
    assert_eq!(f.document(document).current, Some(asset));
    assert_eq!(f.document(document).failed, None);
    assert!(!f.unsaved(document));
}

#[test]
fn a_definition_opens_on_its_first_entry_with_what_was_read() {
    let mut f = fixture();
    let opened = definition();
    let asset = write_definition(&mut f, "content/database.gasset", &opened);

    let document = open(&mut f, &asset);

    let held = state(&f, document);
    assert_eq!(held.skeleton, opened.skeleton);
    assert_eq!(held.entries, opened.entries);
    assert_eq!(held.selected_entry, Some(0));
    assert_eq!(held.loaded, Some(opened));
}

#[test]
fn every_value_is_published_with_a_name_of_its_own_and_the_settings_show_first() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());

    let document = open(&mut f, &asset);
    f.frame();

    assert_eq!(
        value_names(&f, document),
        [
            "Settings",
            "BonePositions",
            "BoneVelocities",
            "BoneVelocities 2",
            "TrajectoryPositions",
            "TrajectoryDirections",
        ]
    );
    assert_eq!(shown_name(&f, document).as_deref(), Some("Settings"));
    assert_eq!(card_names(&mut f), ["Settings"]);
}

#[test]
fn a_channel_named_like_the_settings_gets_a_name_of_its_own() {
    let mut f = fixture_with(settings_channel_recipe);
    let opened = with_channel("Settings", ReflectedValue::Struct(vec![]));
    let asset = write_definition(&mut f, "content/database.gasset", &opened);

    let document = open(&mut f, &asset);

    assert_eq!(value_names(&f, document).last().unwrap(), "Settings 2");
}

#[test]
fn two_channels_of_one_type_each_reach_their_own_value() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);

    edit(&mut f, document, "BoneVelocities 2", "weight", 2.5);

    assert_eq!(channel_at::<BoneVelocities>(&f, document, 2).weight, 2.5);
    assert_ne!(channel_at::<BoneVelocities>(&f, document, 1).weight, 2.5);
}

#[test]
fn a_definition_with_no_entries_selects_none() {
    let mut f = fixture();
    let asset = write_definition(
        &mut f,
        "content/database.gasset",
        &MotionDatabase::default(),
    );

    let document = open(&mut f, &asset);

    assert_eq!(state(&f, document).selected_entry, None);
}

#[test]
fn a_broken_file_fails_its_request() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    std::fs::write(f.project_root.join(&asset.address), b"not a definition").unwrap();

    let document = request(&mut f, &asset);

    assert!(f.status(document).contains("content asset"));
    assert_eq!(f.document(document).current, None);
    assert!(state(&f, document).loaded.is_none());
}

#[test]
fn a_file_of_another_kind_fails_its_request() {
    let mut f = fixture();
    let mistaken = AssetEntry {
        kind: MotionDatabase::name().into(),
        ..f.scene.clone()
    };
    f.add_to_project(&mistaken);

    let document = request(&mut f, &mistaken);

    assert!(f.status(document).contains("mismatch"));
    assert!(state(&f, document).loaded.is_none());
}

fn with_channel(type_name: &str, value: ReflectedValue) -> MotionDatabase {
    let mut definition = definition();
    definition.channels.push(FeatureChannelData::Stored {
        type_name: type_name.into(),
        value,
    });
    definition
}

fn refused(type_name: &str, value: ReflectedValue, reason: &str) {
    let mut f = fixture();
    let asset = write_definition(
        &mut f,
        "content/database.gasset",
        &with_channel(type_name, value),
    );

    let document = open(&mut f, &asset);

    let status = f.status(document);
    assert!(status.contains(type_name), "{status}");
    assert!(status.contains(reason), "{status}");
    assert_eq!(f.document(document).current, None);
    assert!(state(&f, document).loaded.is_none());
    assert!(state(&f, document).channels.is_empty());
    assert!(holds_an_empty_list_of_values(&f, document));
}

#[test]
fn an_unregistered_channel_refuses_to_open() {
    refused(
        "NeverRegistered",
        ReflectedValue::Struct(vec![]),
        "not registered",
    );
}

#[test]
fn a_registered_channel_opens() {
    let mut f = fixture();
    let opened = with_channel("Unplaced", ReflectedValue::Struct(vec![]));
    let asset = write_definition(&mut f, "content/database.gasset", &opened);

    let document = open(&mut f, &asset);

    assert_eq!(f.document(document).current, Some(asset));
    assert_eq!(state(&f, document).channels.len(), 6);
}

#[test]
fn a_type_that_is_not_a_channel_refuses_to_open() {
    refused(
        "NoChannel",
        ReflectedValue::Struct(vec![]),
        "not a feature channel",
    );
}

#[test]
fn a_channel_whose_value_does_not_fit_its_type_refuses_to_open() {
    refused(
        "BonePositions",
        ReflectedValue::Struct(vec![("gone".to_owned(), ReflectedValue::Struct(vec![]))]),
        "cannot be built",
    );
}

#[test]
fn a_second_definition_reuses_the_tab_and_replaces_the_values() {
    let mut f = fixture();
    let first = write_definition(&mut f, "content/first.gasset", &definition());
    let mut second_definition = definition();
    second_definition.rate = 24.0;
    second_definition.channels.truncate(2);
    let second = write_definition(&mut f, "content/second.gasset", &second_definition);
    let document = open(&mut f, &first);
    let old = state(&f, document).channels[0].id;
    show(&mut f, document, "BoneVelocities 2");

    assert_eq!(open(&mut f, &second), document);
    f.frame();

    assert_eq!(state(&f, document).settings.as_ref().unwrap().rate, 24.0);
    assert_eq!(state(&f, document).channels.len(), 2);
    assert_ne!(state(&f, document).channels[0].id, old);
    assert_eq!(
        value_names(&f, document),
        ["Settings", "BonePositions", "BoneVelocities"]
    );
    assert_eq!(shown_name(&f, document).as_deref(), Some("Settings"));
    assert_eq!(card_names(&mut f), ["Settings"]);
    assert_eq!(state(&f, document).loaded, Some(second_definition));
    assert_eq!(f.document(document).current, Some(second));
}

#[test]
fn a_refused_second_definition_leaves_the_first_open() {
    let mut f = fixture();
    let first_definition = definition();
    let first = write_definition(&mut f, "content/first.gasset", &first_definition);
    let second = write_definition(
        &mut f,
        "content/second.gasset",
        &with_channel("NeverRegistered", ReflectedValue::Struct(vec![])),
    );
    let document = open(&mut f, &first);
    let ids = |f: &SceneFixture| -> Vec<_> {
        state(f, document)
            .channels
            .iter()
            .map(|channel| channel.id)
            .collect()
    };
    let kept = ids(&f);

    open(&mut f, &second);

    assert!(f.status(document).contains("NeverRegistered"));
    assert_eq!(f.document(document).current, Some(first));
    assert_eq!(state(&f, document).loaded, Some(first_definition));
    assert_eq!(ids(&f), kept);
}

#[test]
fn a_card_shows_the_rows_of_its_value() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);

    for path in [
        "rate",
        "speed",
        "simulation.position_window",
        "simulation.facing_window",
        "trajectory.sample_dt",
        "trajectory.sample_count",
    ] {
        row_of(&mut f, document, "Settings", path);
    }
    for path in ["weight", "samples.0"] {
        row_of(&mut f, document, "TrajectoryPositions", path);
    }
}

#[test]
fn an_edit_through_a_card_marks_the_document_unsaved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);

    edit(&mut f, document, "Settings", "rate", 48.0);

    assert_eq!(state(&f, document).settings.as_ref().unwrap().rate, 48.0);
    assert!(f.unsaved(document));
}

#[test]
fn a_refused_edit_leaves_the_document_saved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let row = row_of(&mut f, document, "Settings", "rate");
    state_mut(&mut f, document).settings = None;

    let commits = f.world.get_resource_mut::<PropertyCommits>().unwrap();
    commit_number(commits, &row, 48.0).unwrap();
    f.frame();

    assert!(!f.unsaved(document));
    assert!(f.status(document).starts_with("Set Settings.rate failed"));
}

#[test]
fn saving_writes_the_edits_and_the_document_in_order_and_marks_it_saved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    edit(&mut f, document, "Settings", "rate", 48.0);
    edit(&mut f, document, "BoneVelocities 2", "weight", 2.5);
    {
        let settings = state_mut(&mut f, document).settings.as_mut().unwrap();
        settings.simulation.position_joint = "Head".into();
        settings.trajectory.sample_count = 4;
    }
    assert!(f.unsaved(document));

    save(&mut f, document);
    assert_eq!(f.status(document), "Saved content/database.gasset");

    let saved = read_definition(&f.project_root, &asset).unwrap();
    assert_eq!(saved.rate, 48.0);
    assert_eq!(saved.simulation.position_joint, "Head".into());
    assert_eq!(saved.trajectory.sample_count, 4);
    assert_eq!(saved.skeleton, state(&f, document).skeleton);
    assert_eq!(saved.entries, state(&f, document).entries);
    let names: Vec<&str> = saved
        .channels
        .iter()
        .map(FeatureChannelData::type_name)
        .collect();
    assert_eq!(
        names,
        [
            "BonePositions",
            "BoneVelocities",
            "BoneVelocities",
            "TrajectoryPositions",
            "TrajectoryDirections",
        ]
    );
    assert_eq!(state(&f, document).loaded, Some(saved));
    assert!(
        f.world
            .get_component_for_entity::<SaveRequested>(document)
            .is_none()
    );
    assert!(!f.unsaved(document));
    assert_eq!(
        f.written::<SceneSaved>(),
        [SceneSaved {
            document,
            asset: asset.clone(),
        }]
    );
    assert!(f.written::<SaveFailed>().is_empty());
    assert_eq!(f.document(document).current, Some(asset));
}

#[test]
fn a_saved_channel_edit_reopens_with_its_value() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let other = write_definition(&mut f, "content/other.gasset", &definition());
    let document = open(&mut f, &asset);
    edit(&mut f, document, "BoneVelocities 2", "weight", 2.5);
    save(&mut f, document);

    open(&mut f, &other);
    open(&mut f, &asset);

    assert_eq!(channel_at::<BoneVelocities>(&f, document, 2).weight, 2.5);
}

#[test]
fn discarding_edits_goes_back_to_the_definition_as_last_saved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    edit(&mut f, document, "Settings", "rate", 48.0);
    save(&mut f, document);
    let saved = read_definition(&f.project_root, &asset).unwrap();
    show(&mut f, document, "BoneVelocities 2");
    edit(&mut f, document, "BoneVelocities 2", "weight", 2.5);

    f.world.trigger(DiscardEdits(document));
    f.frame();

    assert_eq!(state(&f, document).definition().unwrap(), saved);
    assert_eq!(shown_name(&f, document).as_deref(), Some("Settings"));
}

#[test]
fn edits_that_cannot_be_discarded_stay_marked_unsaved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    edit(&mut f, document, "Settings", "rate", 48.0);
    let state = f
        .world
        .get_component_for_entity_mut::<DefinitionDocument>(document)
        .unwrap();
    state.loaded.as_mut().unwrap().channels[0] = FeatureChannelData::Stored {
        type_name: "not registered".into(),
        value: ReflectedValue::Struct(Vec::new()),
    };
    f.world.remove_component::<Unsaved>(document);

    f.world.trigger(DiscardEdits(document));
    f.frame();

    assert!(f.unsaved(document));
}

#[test]
fn a_definitions_preview_reflects_its_settings() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);

    let hosted = f
        .world
        .get_component_for_entity::<HostedApp>(document)
        .unwrap();

    let types = hosted.type_registry().unwrap();
    assert!(types.read().get(TypeId::of::<DatabaseSettings>()).is_some());
}

#[test]
fn a_no_op_save_keeps_the_definition() {
    let mut f = fixture();
    let opened = definition();
    let asset = write_definition(&mut f, "content/database.gasset", &opened);
    let document = open(&mut f, &asset);

    save(&mut f, document);

    let saved = read_definition(&f.project_root, &asset).unwrap();
    assert_eq!(saved, opened);
}

#[test]
fn a_saved_definition_keeps_mirrored_entries_and_the_mirror_axis() {
    let mut f = fixture();
    let mut opened = definition();
    opened.entries[1].mirrored = true;
    opened.mirror_axis = MirrorAxis::Z;
    let asset = write_definition(&mut f, "content/database.gasset", &opened);
    let document = open(&mut f, &asset);
    assert_eq!(
        state(&f, document).settings.as_ref().unwrap().mirror_axis,
        MirrorAxis::Z
    );

    save(&mut f, document);
    assert_eq!(read_definition(&f.project_root, &asset).unwrap(), opened);

    state_mut(&mut f, document)
        .settings
        .as_mut()
        .unwrap()
        .mirror_axis = MirrorAxis::X;
    save(&mut f, document);

    let saved = read_definition(&f.project_root, &asset).unwrap();
    assert_eq!(saved.mirror_axis, MirrorAxis::X);
    assert!(saved.entries[1].mirrored);
    assert!(!saved.entries[0].mirrored);
}

#[test]
fn channels_of_interleaved_types_save_in_their_own_order() {
    let mut f = fixture();
    let mut opened = definition();
    opened.channels = [1, 0, 2, 3, 0]
        .map(|index| opened.channels[index].clone())
        .to_vec();
    let asset = write_definition(&mut f, "content/database.gasset", &opened);
    let document = open(&mut f, &asset);

    save(&mut f, document);

    assert_eq!(
        read_definition(&f.project_root, &asset).unwrap().channels,
        opened.channels
    );
}

#[test]
fn a_background_tab_saves_its_own_edits() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    edit(&mut f, document, "Settings", "speed", 2.0);

    let front = f.open();
    assert_ne!(front, document);
    save(&mut f, document);

    let saved = read_definition(&f.project_root, &asset).unwrap();
    assert_eq!(saved.speed, 2.0);
    assert_eq!(saved.channels.len(), 5);
    assert!(!f.unsaved(document));
    assert_eq!(f.written::<SceneSaved>().len(), 1);
}

#[test]
fn a_commit_and_a_save_in_one_frame_leave_the_document_saved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);

    queue_edit(&mut f, document, "Settings", "speed", 2.0);
    f.world.insert(SaveRequested, document);
    f.frame();
    f.frame();

    assert_eq!(read_definition(&f.project_root, &asset).unwrap().speed, 2.0);
    assert!(!f.unsaved(document));
}

#[test]
fn the_next_edit_clears_the_saved_line() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    edit(&mut f, document, "Settings", "speed", 2.0);
    save(&mut f, document);
    assert_eq!(f.status(document), "Saved content/database.gasset");
    f.frames_until("the derive never ended", |f| {
        state(f, document).derive_job.is_none()
    });

    edit(&mut f, document, "Settings", "speed", 3.0);

    assert!(f.unsaved(document));
    assert_eq!(f.status(document), "");
}

#[test]
fn a_save_that_cannot_write_reports_it_and_stays_unsaved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    edit(&mut f, document, "Settings", "speed", 2.0);
    f.make_project_unwritable();

    save(&mut f, document);

    assert!(f.status(document).starts_with("Save failed: "));
    assert_eq!(
        f.written::<SaveFailed>(),
        [SaveFailed {
            document,
            message: f.status(document),
        }]
    );
    assert!(f.written::<SceneSaved>().is_empty());
    assert!(f.unsaved(document));
    assert!(
        f.world
            .get_component_for_entity::<SaveRequested>(document)
            .is_none()
    );
}

#[test]
fn a_failed_preview_keeps_the_definition_and_refuses_save() {
    let mut f = fixture_with(broken_recipe);
    let opened = definition();
    let asset = write_definition(&mut f, "content/database.gasset", &opened);
    let before = std::fs::read(f.project_root.join(&asset.address)).unwrap();

    let document = request(&mut f, &asset);
    f.frame();

    assert!(
        f.world
            .get_component_for_entity::<PreviewFailed>(document)
            .is_some()
    );
    assert!(
        f.world
            .get_component_for_entity::<HostedApp>(document)
            .is_none()
    );
    assert_eq!(f.document(document).current, Some(asset.clone()));
    assert_eq!(state(&f, document).loaded, Some(opened));
    assert!(f.status(document).starts_with("Preview failed"));
    assert!(state(&f, document).settings.is_none());
    assert!(holds_an_empty_list_of_values(&f, document));
    assert!(card_names(&mut f).is_empty());

    save(&mut f, document);

    assert!(f.status(document).starts_with("Save failed: "));
    assert_eq!(f.written::<SaveFailed>().len(), 1);
    assert!(f.written::<SceneSaved>().is_empty());
    assert_eq!(
        std::fs::read(f.project_root.join(&asset.address)).unwrap(),
        before
    );
}

fn edit_speed(app: &mut App, document: Entity) -> bool {
    let world = app.main_mut().world_mut();
    let Some(settings) = world
        .get_component_for_entity_mut::<DefinitionDocument>(document)
        .and_then(|state| state.settings.as_mut())
    else {
        return false;
    };
    settings.speed = 3.0;
    world.insert(Unsaved, document);
    true
}

#[test]
fn install_opens_saves_and_discards_a_definition_in_the_editors_schedule() {
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetRegistry::default();
    let id = save_content_asset_into(
        &definition(),
        directory.path(),
        "content/database.gasset",
        &mut assets,
    )
    .unwrap();
    let asset = assets.get(id).unwrap().clone();
    let mut project = ProjectState::default();
    project.project = Some(Project {
        root: directory.path().into(),
        content_root: "content".into(),
    });
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .insert_resource(Window::new((640, 480), 1.0))
        .insert_resource(ActiveEditor::default())
        .insert_resource(AssetEditorRegistry::default())
        .insert_resource(project)
        .insert_resource(assets)
        .insert_resource(ViewportShown(None))
        .insert_resource(InspectorRegistry::default())
        .insert_resource(PropertyCommits::default())
        .insert_resource(UITheme::default())
        .register_plugin(HostedPlugin)
        .register_plugin(PreviewHostPlugin)
        .register_plugin(ScenePlugin {
            recipe: database_recipe,
            clipboard: None,
        });
    install(&mut app);
    app.finish_plugin_build();
    let world = app.main_mut().world_mut();
    let document = world.spawn((
        EditorDocument {
            asset_type: MotionDatabase::name(),
            title: "database".into(),
            current: None,
            pending: Some(asset.clone()),
            order: 1,
            status: String::new(),
            failed: None,
        },
        DefinitionDocument::default(),
    ));
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);

    let deadline = Instant::now() + Duration::from_secs(20);
    while !edit_speed(&mut app, document) {
        app.update();
        assert!(Instant::now() < deadline, "the definition never opened");
        std::thread::sleep(Duration::from_millis(1));
    }

    let unsaved = |app: &App| {
        app.main()
            .world()
            .get_component_for_entity::<Unsaved>(document)
            .is_some()
    };
    app.main_mut().world_mut().insert(SaveRequested, document);
    while unsaved(&app) {
        app.update();
        assert!(Instant::now() < deadline, "the definition was never saved");
    }

    let saved = read_definition(directory.path(), &asset).unwrap();
    assert_eq!(saved.speed, 3.0);

    let world = app.main_mut().world_mut();
    let state = world
        .get_component_for_entity_mut::<DefinitionDocument>(document)
        .unwrap();
    state.settings.as_mut().unwrap().speed = 7.0;
    world.trigger(DiscardEdits(document));
    app.update();

    let held = app
        .main()
        .world()
        .get_component_for_entity::<DefinitionDocument>(document)
        .unwrap();
    assert_eq!(held.settings.as_ref().unwrap().speed, 3.0);
}

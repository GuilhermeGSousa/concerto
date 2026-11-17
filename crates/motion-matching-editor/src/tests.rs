mod graph;

use std::{
    any::Any,
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
        ActiveEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry, SaveRequested,
        Unsaved,
    },
    guard::DiscardEdits,
    hosted::{
        input::{ViewportShown, ViewportView},
        testing::*,
    },
    inspector::{
        InspectedSnapshots, InspectedValues, InspectorRegistry, PropertyTarget, WriteValue,
        spawn_panel, value_systems,
    },
    name_menu::close_name_menu,
    preview::PreviewFailed,
    project::{AssetsWritten, Project, ProjectState},
    scene::{SaveFailed, SceneRecipe, SceneSaved},
};
use concerto_foundation::assets::{
    Asset, AssetId,
    asset_server::AssetServer,
    asset_store::AssetStore,
    content::save_content_asset_into,
    derived::DerivedDataCache,
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_motion_matching::{
    BonePositions, BoneVelocities, FeatureChannel, FeatureChannelData, FrameContext, MirrorAxis,
    MotionDatabase, MotionEntry, ReflectFeatureChannel, TimeRange, TrajectoryDirections,
    TrajectoryPositions,
};
use concerto_reflect::{Reflect, ReflectedValue};
use concerto_ui::{node::UINode, theme::UITheme};
use concerto_window::plugin::WindowPlugin;
use glam::Vec2;
use serde_json::{Value, json};

use crate::{
    DefinitionDocument, MotionDatabaseEditor,
    derive::{derive_definitions, follow_written_inputs, mark_background_work},
    document::{
        clear_status_on_edit, follow_definition_edits, load_definition, restore_discarded_edits,
    },
    facts::{follow_skeleton, read_entry_facts},
    install,
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
    let clips = AssetStore::<AnimationClip>::new();
    f.world
        .get_resource::<AssetServer>()
        .unwrap()
        .clone()
        .register_derived_asset(&clips);
    f.world.insert_resource(clips);
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
    f.world.insert_resource(InspectorRegistry::default());
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
    let id = save_content_asset_into(asset, None, &root, address, assets).unwrap();
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

pub(crate) fn value_names(f: &SceneFixture, document: Entity) -> Vec<String> {
    f.world
        .get_component_for_entity::<InspectedValues>(document)
        .map_or_else(Vec::new, |values| {
            values.0.iter().map(|value| value.name()).collect()
        })
}

fn value_named(f: &SceneFixture, document: Entity, name: &str) -> Arc<dyn PropertyTarget> {
    f.world
        .get_component_for_entity::<InspectedValues>(document)
        .and_then(|values| values.0.iter().find(|value| value.name() == name))
        .cloned()
        .expect("the document publishes the value")
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

pub(crate) fn write_value(
    f: &mut SceneFixture,
    document: Entity,
    name: &str,
    path: &str,
    json: Value,
) {
    let target = value_named(f, document, name);
    f.world.trigger(WriteValue {
        target,
        path: path.into(),
        json,
    });
    f.frame();
}

#[test]
fn install_alone_plays_a_definitions_preview_and_saves_and_discards_in_the_editors_schedule() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut assets = AssetRegistry::default();
    let skeleton = skeleton_of(&["Hips", "Spine", "Head"]);
    let skeleton = save_content_asset_into(
        &skeleton,
        None,
        root,
        "content/hero/skeleton.gasset",
        &mut assets,
    );
    let walk = clip_of(10, 0..3);
    let walk = save_content_asset_into(&walk, None, root, "content/hero/walk.gasset", &mut assets);
    let opened = MotionDatabase {
        skeleton: AssetHandle::weak(skeleton.unwrap()),
        entries: vec![MotionEntry {
            clip: AssetHandle::weak(walk.unwrap()),
            range: None,
            mirrored: false,
        }],
        ..Default::default()
    };
    let id = save_content_asset_into(
        &opened,
        None,
        root,
        "content/hero/database.gasset",
        &mut assets,
    );
    let asset = assets.get(id.unwrap()).unwrap().clone();
    let mut project = ProjectState::default();
    project.project = Some(Project {
        root: root.into(),
        content_root: "content".into(),
        default_scene: None,
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
    let kind = MotionDatabase::name();
    assert!(editors.creatable().any(|(creatable, _)| creatable == kind));
    app.insert_resource(project).insert_resource(assets);
    let world = app.main_mut().world_mut();
    world
        .get_resource_mut::<AssetEditorCommands>()
        .unwrap()
        .0
        .push_back(AssetEditorCommand::Open(asset.clone()));
    let playing = |app: &mut App| {
        let world = app.main_mut().world_mut();
        world
            .query::<(Entity, &DefinitionDocument), ()>()
            .iter(world)
            .find(|(_, state)| state.time > 0.0)
            .map(|(entity, _)| entity)
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let document = loop {
        app.update();
        if let Some(document) = playing(&mut app) {
            break document;
        }
        assert!(Instant::now() < deadline, "the head never moved");
        std::thread::sleep(Duration::from_millis(1));
    };

    let world = app.main_mut().world_mut();
    world
        .get_component_for_entity_mut::<DefinitionDocument>(document)
        .unwrap()
        .settings
        .as_mut()
        .unwrap()
        .speed = 3.0;
    world.insert(Unsaved, document);
    world.insert(SaveRequested, document);
    let unsaved = |app: &App| {
        app.main()
            .world()
            .get_component_for_entity::<Unsaved>(document)
            .is_some()
    };
    while unsaved(&app) {
        app.update();
        assert!(Instant::now() < deadline, "the definition was never saved");
    }
    assert_eq!(read_definition(root, &asset).unwrap().speed, 3.0);

    let world = app.main_mut().world_mut();
    world
        .get_component_for_entity_mut::<DefinitionDocument>(document)
        .unwrap()
        .settings
        .as_mut()
        .unwrap()
        .speed = 7.0;
    world.trigger(DiscardEdits(document));
    app.update();

    let held = app
        .main()
        .world()
        .get_component_for_entity::<DefinitionDocument>(document)
        .unwrap();
    assert_eq!(held.settings.as_ref().unwrap().speed, 3.0);
}

#[test]
fn a_definitions_values_are_reached_by_name_saved_as_written_and_discarded_back_to_the_file() {
    let mut f = fixture();
    let mut opened = definition();
    opened.entries[1].mirrored = true;
    opened.mirror_axis = MirrorAxis::Z;
    let asset = write_definition(&mut f, "content/database.gasset", &opened);
    let other = write_definition(&mut f, "content/other.gasset", &definition());
    let document = open(&mut f, &asset);

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
    save(&mut f, document);
    assert_eq!(read_definition(&f.project_root, &asset).unwrap(), opened);

    write_value(&mut f, document, "Settings", "rate", json!(48.0));
    write_value(
        &mut f,
        document,
        "Settings",
        "simulation.facing_joint",
        json!("Head"),
    );
    write_value(&mut f, document, "BoneVelocities 2", "weight", json!(2.5));
    assert!(f.unsaved(document));
    assert_ne!(channel_at::<BoneVelocities>(&f, document, 1).weight, 2.5);
    save(&mut f, document);

    let saved = read_definition(&f.project_root, &asset).unwrap();
    assert_eq!(saved.rate, 48.0);
    assert_eq!(saved.simulation.facing_joint, "Head".into());
    assert_eq!(saved.entries, opened.entries);
    assert_eq!(saved.mirror_axis, MirrorAxis::Z);
    assert!(!f.unsaved(document));
    open(&mut f, &other);
    open(&mut f, &asset);
    assert_eq!(channel_at::<BoneVelocities>(&f, document, 2).weight, 2.5);

    write_value(&mut f, document, "Settings", "rate", json!(24.0));
    f.world.trigger(DiscardEdits(document));
    f.frame();

    assert_eq!(state(&f, document).definition().unwrap(), saved);
}

struct Unbuildable {
    type_name: &'static str,
    value: ReflectedValue,
}

#[test]
fn a_definition_holding_a_channel_the_game_cannot_build_refuses_to_open() {
    let rows = [
        Unbuildable {
            type_name: "NeverRegistered",
            value: ReflectedValue::Struct(vec![]),
        },
        Unbuildable {
            type_name: "NoChannel",
            value: ReflectedValue::Struct(vec![]),
        },
        Unbuildable {
            type_name: "BonePositions",
            value: ReflectedValue::Struct(vec![(
                "gone".to_owned(),
                ReflectedValue::Struct(vec![]),
            )]),
        },
    ];
    for row in rows {
        let mut f = fixture();
        let mut refused = definition();
        refused.channels.push(FeatureChannelData::Stored {
            type_name: row.type_name.into(),
            value: row.value,
        });
        let asset = write_definition(&mut f, "content/database.gasset", &refused);

        let document = open(&mut f, &asset);

        assert!(
            f.status(document).contains(row.type_name),
            "{}",
            f.status(document)
        );
        assert_eq!(f.document(document).current, None, "{}", row.type_name);
        assert!(value_names(&f, document).is_empty());
    }
}

#[test]
fn a_definition_whose_preview_failed_refuses_to_save_over_its_file() {
    let mut f = fixture_with(broken_recipe);
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let before = std::fs::read(f.project_root.join(&asset.address)).unwrap();
    let document = request(&mut f, &asset);
    f.frame();
    assert!(
        f.world
            .get_component_for_entity::<PreviewFailed>(document)
            .is_some()
    );

    f.world.insert(Unsaved, document);
    save(&mut f, document);

    assert_eq!(f.written::<SaveFailed>().len(), 1);
    assert!(f.written::<SceneSaved>().is_empty());
    assert_eq!(
        std::fs::read(f.project_root.join(&asset.address)).unwrap(),
        before
    );
}

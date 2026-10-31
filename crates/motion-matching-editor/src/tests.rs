use std::{
    any::TypeId,
    time::{Duration, Instant},
};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
};
use concerto_ecs::{
    Component, Entity, IntoSystem,
    reflect::{AppTypeRegistry, ReflectComponent},
};
use concerto_editor::{
    EditorPlugin,
    asset_editor::{ActiveEditor, AssetEditorRegistry, EditorDocument, SaveRequested, Unsaved},
    guard::DiscardEdits,
    hosted::{HostedApp, HostedPlugin, input::ViewportShown, testing::*},
    inspector::{InspectorData, InspectorRegistry, PropertyKind},
    preview::{PreviewFailed, PreviewHostPlugin},
    project::{Project, ProjectState},
    scene::{SaveFailed, ScenePlugin, SceneRecipe, SceneSaved},
    tools::{
        edits::{ComponentEdit, EditComponent, SetValue},
        entity_ops::{EntityEdit, FixedEntities},
        scene::SceneDirty,
        selection::{Select, Selection},
    },
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::save_content_asset_into,
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::JointName;
use concerto_motion_matching::{
    BonePositions, BoneVelocities, FeatureChannel, FrameContext, MotionDatabaseDefinition,
    MotionEntry, ReflectFeatureChannel, StoredChannel, TimeRange, TrajectoryDirections,
    TrajectoryPositions, weightless_hash,
};
use concerto_reflect::{PropertyPath, Reflect, ReflectedValue};
use concerto_window::plugin::{Window, WindowPlugin};
use serde_json::{Value, json};

use crate::{
    ChannelOrder, DatabaseSettings, DefinitionDocument, MotionDatabaseEditor,
    document::{follow_definition_edits, forget_discarded_edits, load_definition},
    install, read_definition,
    save::{request_definition_saves, save_definition},
};

#[derive(Component, Reflect, Default)]
#[reflect(FeatureChannel)]
struct Unplaced;

impl FeatureChannel for Unplaced {
    fn dims(&self) -> usize {
        0
    }

    fn extract(&self, _: &FrameContext, _: &mut [f32]) {}
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct NoChannel;

fn database_recipe(app: &mut App) {
    scene_recipe(app);
    app.register_type::<BonePositions>()
        .register_type::<BoneVelocities>()
        .register_type::<TrajectoryPositions>()
        .register_type::<TrajectoryDirections>()
        .register_type::<Unplaced>()
        .register_type::<NoChannel>();
}

fn broken_recipe(_: &mut App) {
    panic!("the game broke");
}

fn fixture_with(recipe: fn(&mut App)) -> SceneFixture {
    let mut f = SceneFixture::new();
    f.world.insert_resource(SceneRecipe {
        recipe,
        gpu: None,
        clipboard: None,
    });
    f.world
        .get_resource_mut::<AssetEditorRegistry>()
        .unwrap()
        .register_creatable::<MotionDatabaseDefinition, MotionDatabaseEditor>()
        .unwrap();
    f.world.add_listener(save_definition);
    f.world.add_listener(forget_discarded_edits);
    f.add_delivery(follow_definition_edits.into_boxed_system());
    f.add_delivery(request_definition_saves.into_boxed_system());
    f.add_delivery(load_definition.into_boxed_system());
    f
}

fn fixture() -> SceneFixture {
    fixture_with(database_recipe)
}

fn entry(seconds: f32) -> MotionEntry {
    MotionEntry {
        clip: AssetHandle::weak(AssetId::new()),
        range: Some(TimeRange {
            start: 0.0,
            end: seconds,
        }),
    }
}

fn definition() -> MotionDatabaseDefinition {
    MotionDatabaseDefinition {
        skeleton: AssetHandle::weak(AssetId::new()),
        entries: vec![entry(1.0), entry(2.0)],
        rate: 30.0,
        speed: 1.25,
        ..Default::default()
    }
}

fn write_definition(
    f: &mut SceneFixture,
    address: &str,
    definition: &MotionDatabaseDefinition,
) -> AssetEntry {
    let root = f.project_root.clone();
    let assets = f.world.get_resource_mut::<AssetRegistry>().unwrap();
    let id = save_content_asset_into(definition, &root, address, assets).unwrap();
    assets.get(id).unwrap().clone()
}

fn request(f: &mut SceneFixture, asset: &AssetEntry) -> Entity {
    f.request(asset);
    let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
    f.frames_until("the definition never finished opening", |f| {
        f.document(document).pending.is_none()
    });
    document
}

fn open(f: &mut SceneFixture, asset: &AssetEntry) -> Entity {
    let document = request(f, asset);
    f.frame();
    f.frame();
    document
}

fn state(f: &SceneFixture, document: Entity) -> &DefinitionDocument {
    f.world
        .get_component_for_entity::<DefinitionDocument>(document)
        .expect("the document is a definition's")
}

fn settings_entity(f: &mut SceneFixture, document: Entity) -> Entity {
    let world = f.scene_world_mut(document);
    let found: Vec<Entity> = world
        .query::<(Entity, &DatabaseSettings), ()>()
        .iter(world)
        .map(|(entity, _)| entity)
        .collect();
    assert_eq!(found.len(), 1, "a preview holds one settings entity");
    found[0]
}

fn channels(f: &mut SceneFixture, document: Entity) -> Vec<Entity> {
    let world = f.scene_world_mut(document);
    let mut found: Vec<(ChannelOrder, Entity)> = world
        .query::<(Entity, &ChannelOrder), ()>()
        .iter(world)
        .map(|(entity, order)| (*order, entity))
        .collect();
    found.sort_by_key(|(order, _)| *order);
    found.into_iter().map(|(_, entity)| entity).collect()
}

fn component<T: Component + Clone>(f: &SceneFixture, document: Entity, entity: Entity) -> T {
    f.scene_world(document)
        .get_component_for_entity::<T>(entity)
        .cloned()
        .expect("the entity carries it")
}

fn set<T: 'static>(
    f: &mut SceneFixture,
    entity: Entity,
    path: impl IntoIterator<Item = &'static str>,
    value: Value,
) {
    f.signal(SetValue {
        entity,
        component: TypeId::of::<T>(),
        path: PropertyPath::new(path),
        value,
    });
}

fn save(f: &mut SceneFixture, document: Entity) {
    f.world.insert(SaveRequested, document);
    f.deliver();
}

fn scene_dirty(f: &SceneFixture, document: Entity) -> bool {
    f.scene_world(document)
        .get_resource::<SceneDirty>()
        .unwrap()
        .0
}

fn property_rows<T: Component + Reflect>(
    f: &SceneFixture,
    document: Entity,
    entity: Entity,
) -> Vec<String> {
    let world = f.scene_world(document);
    let value = world.get_component_for_entity::<T>(entity).unwrap();
    let types = world.get_resource::<AppTypeRegistry>().unwrap().read();
    InspectorRegistry::default()
        .collect(value, &types)
        .iter()
        .filter(|property| matches!(property.kind, PropertyKind::Value(_)))
        .map(|property| property.path.to_string())
        .collect()
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
fn install_registers_a_creatable_editor_that_edits_in_its_preview() {
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
    let kind = MotionDatabaseDefinition::name();
    assert!(editors.preview(kind).is_some());
    assert!(editors.bottom_host(kind));
    assert!(editors.edits_in_preview(kind));
    assert!(editors.creatable().any(|(creatable, _)| creatable == kind));
}

#[test]
fn opening_a_definition_spawns_its_settings_and_its_channels_in_order() {
    let mut f = fixture();
    let opened = definition();
    let asset = write_definition(&mut f, "content/database.gasset", &opened);

    let document = open(&mut f, &asset);

    let settings = settings_entity(&mut f, document);
    assert_eq!(
        component::<DatabaseSettings>(&f, document, settings),
        DatabaseSettings::of(&opened)
    );
    let channels = channels(&mut f, document);
    assert_eq!(channels.len(), 5);
    assert_eq!(
        component::<BonePositions>(&f, document, channels[0]).bones,
        [JointName::from("LeftFoot"), "RightFoot".into()]
    );
    assert_eq!(
        component::<BoneVelocities>(&f, document, channels[1]).bones,
        [JointName::from("LeftFoot"), "RightFoot".into()]
    );
    assert_eq!(
        component::<BoneVelocities>(&f, document, channels[2]).bones,
        [JointName::from("Hips")]
    );
    assert_eq!(
        component::<TrajectoryPositions>(&f, document, channels[3]).samples,
        [0, 1, 2]
    );
    assert_eq!(
        component::<TrajectoryDirections>(&f, document, channels[4]).weight,
        1.5
    );
    assert_eq!(f.document(document).current, Some(asset));
    assert_eq!(f.status(document), "");
    assert!(!f.unsaved(document));
}

#[test]
fn only_the_entries_and_the_skeleton_live_in_the_document() {
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
fn a_definition_with_no_entries_selects_none() {
    let mut f = fixture();
    let asset = write_definition(
        &mut f,
        "content/database.gasset",
        &MotionDatabaseDefinition::default(),
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
        kind: MotionDatabaseDefinition::name().into(),
        ..f.scene.clone()
    };
    f.add_to_project(&mistaken);

    let document = request(&mut f, &mistaken);

    assert!(f.status(document).contains("mismatch"));
    assert!(state(&f, document).loaded.is_none());
}

fn with_channel(type_name: &str, value: ReflectedValue) -> MotionDatabaseDefinition {
    let mut definition = definition();
    definition.channels.push(StoredChannel {
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
    assert!(channels(&mut f, document).is_empty());
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
fn a_channel_that_is_not_a_component_refuses_to_open() {
    refused(
        "Unplaced",
        ReflectedValue::Struct(vec![]),
        "not a component",
    );
}

#[test]
fn a_component_that_is_not_a_channel_refuses_to_open() {
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
fn a_second_definition_reuses_the_tab_and_replaces_the_entities() {
    let mut f = fixture();
    let first = write_definition(&mut f, "content/first.gasset", &definition());
    let mut second_definition = definition();
    second_definition.rate = 24.0;
    second_definition.channels.truncate(2);
    let second = write_definition(&mut f, "content/second.gasset", &second_definition);
    let document = open(&mut f, &first);
    let old_settings = settings_entity(&mut f, document);
    let old_channels = channels(&mut f, document);
    f.signal(Select(Some(old_channels[0])));

    assert_eq!(open(&mut f, &second), document);

    let settings = settings_entity(&mut f, document);
    assert_eq!(
        component::<DatabaseSettings>(&f, document, settings).rate,
        24.0
    );
    assert_eq!(channels(&mut f, document).len(), 2);
    assert_ne!(settings, old_settings);
    assert!(!f.scene_world(document).entity_is_valid(old_channels[4]));
    assert_eq!(
        f.scene_world(document)
            .get_resource::<Selection>()
            .unwrap()
            .entity(),
        None
    );
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
    let kept = channels(&mut f, document);

    open(&mut f, &second);

    assert!(f.status(document).contains("NeverRegistered"));
    assert_eq!(f.document(document).current, Some(first));
    assert_eq!(state(&f, document).loaded, Some(first_definition));
    assert_eq!(channels(&mut f, document), kept);
}

#[test]
fn the_inspector_shows_the_settings_and_a_channel_once_selected() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);
    let channel = channels(&mut f, document)[3];

    f.signal(Select(Some(settings)));
    f.frame();
    assert_eq!(
        f.world
            .get_resource::<InspectorData>()
            .unwrap()
            .scene_entity,
        Some(settings)
    );
    let rows = property_rows::<DatabaseSettings>(&f, document, settings);
    for row in [
        "rate",
        "speed",
        "simulation.position_window",
        "simulation.facing_window",
        "trajectory.sample_dt",
        "trajectory.sample_count",
    ] {
        assert!(rows.contains(&row.to_owned()), "{row} is not in {rows:?}");
    }

    f.signal(Select(Some(channel)));
    f.frame();
    assert_eq!(
        f.world
            .get_resource::<InspectorData>()
            .unwrap()
            .scene_entity,
        Some(channel)
    );
    let rows = property_rows::<TrajectoryPositions>(&f, document, channel);
    assert!(rows.contains(&"weight".to_owned()), "{rows:?}");
    assert!(rows.contains(&"samples.0".to_owned()), "{rows:?}");
}

#[test]
fn an_applied_edit_marks_the_document_unsaved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);

    set::<DatabaseSettings>(&mut f, settings, ["rate"], json!(48.0));

    assert_eq!(
        component::<DatabaseSettings>(&f, document, settings).rate,
        48.0
    );
    assert!(f.unsaved(document));
    assert!(scene_dirty(&f, document));
}

#[test]
fn a_refused_edit_leaves_the_document_saved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);

    set::<DatabaseSettings>(&mut f, settings, ["rate"], json!("fast"));
    set::<DatabaseSettings>(&mut f, settings, ["no_such_field"], json!(1.0));

    assert!(!f.unsaved(document));
}

#[test]
fn structural_edits_change_nothing_and_leave_the_document_saved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);

    f.signal(EntityEdit::Delete(settings));
    f.signal(EditComponent(ComponentEdit::Remove {
        entity: settings,
        component: TypeId::of::<DatabaseSettings>(),
    }));

    assert!(
        f.scene_world(document)
            .get_resource::<FixedEntities>()
            .is_some()
    );
    assert_eq!(settings_entity(&mut f, document), settings);
    assert!(!f.unsaved(document));
}

#[test]
fn an_applied_component_edit_marks_the_document_unsaved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);
    f.scene_world_mut(document)
        .remove_resource::<FixedEntities>();

    f.signal(EditComponent(ComponentEdit::Remove {
        entity: settings,
        component: TypeId::of::<DatabaseSettings>(),
    }));

    assert!(f.unsaved(document));
}

#[test]
fn saving_writes_the_edits_and_the_document_in_order_and_marks_it_saved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);
    let channels = channels(&mut f, document);
    set::<DatabaseSettings>(&mut f, settings, ["rate"], json!(48.0));
    set::<DatabaseSettings>(
        &mut f,
        settings,
        ["simulation", "position_joint"],
        json!("Head"),
    );
    set::<DatabaseSettings>(&mut f, settings, ["trajectory", "sample_count"], json!(4));
    set::<BoneVelocities>(&mut f, channels[2], ["weight"], json!(2.5));
    assert!(f.unsaved(document));

    save(&mut f, document);

    let saved = read_definition(&f.project_root, &asset).unwrap();
    assert_eq!(saved.rate, 48.0);
    assert_eq!(saved.simulation.position_joint, "Head".into());
    assert_eq!(saved.trajectory.sample_count, 4);
    assert_eq!(saved.skeleton, state(&f, document).skeleton);
    assert_eq!(saved.entries, state(&f, document).entries);
    let names: Vec<&str> = saved
        .channels
        .iter()
        .map(|channel| channel.type_name.as_str())
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
    assert!(!scene_dirty(&f, document));
    assert_eq!(
        f.written::<SceneSaved>(),
        [SceneSaved {
            document,
            asset: asset.clone(),
        }]
    );
    assert!(f.written::<SaveFailed>().is_empty());
    assert_eq!(f.status(document), "Saved content/database.gasset");
    assert_eq!(f.document(document).current, Some(asset));
}

#[test]
fn a_saved_channel_edit_reopens_with_its_value() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let other = write_definition(&mut f, "content/other.gasset", &definition());
    let document = open(&mut f, &asset);
    let channel = channels(&mut f, document)[2];
    set::<BoneVelocities>(&mut f, channel, ["weight"], json!(2.5));
    save(&mut f, document);

    open(&mut f, &other);
    open(&mut f, &asset);

    let channel = channels(&mut f, document)[2];
    assert_eq!(
        component::<BoneVelocities>(&f, document, channel).weight,
        2.5
    );
}

#[test]
fn a_no_op_save_keeps_the_definition_and_its_hash() {
    let mut f = fixture();
    let opened = definition();
    let asset = write_definition(&mut f, "content/database.gasset", &opened);
    let document = open(&mut f, &asset);

    save(&mut f, document);

    let saved = read_definition(&f.project_root, &asset).unwrap();
    assert_eq!(saved, opened);
    assert_eq!(
        weightless_hash(&saved, b"skeleton", &[b"clip"]),
        weightless_hash(&opened, b"skeleton", &[b"clip"])
    );
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
    let settings = settings_entity(&mut f, document);
    set::<DatabaseSettings>(&mut f, settings, ["speed"], json!(2.0));

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
fn a_save_that_cannot_write_reports_it_and_stays_unsaved() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);
    set::<DatabaseSettings>(&mut f, settings, ["speed"], json!(2.0));
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
    assert!(scene_dirty(&f, document));
}

#[test]
fn discarding_forgets_that_the_preview_was_edited() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = open(&mut f, &asset);
    let settings = settings_entity(&mut f, document);
    set::<DatabaseSettings>(&mut f, settings, ["speed"], json!(2.0));
    assert!(scene_dirty(&f, document));

    f.world.trigger(DiscardEdits(document));

    assert!(!scene_dirty(&f, document));
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
    let Some(hosted) = app
        .main_mut()
        .world_mut()
        .get_component_for_entity_mut::<HostedApp>(document)
    else {
        return false;
    };
    let preview = hosted.world_mut();
    let settings = preview
        .query::<(Entity, &DatabaseSettings), ()>()
        .iter(preview)
        .map(|(entity, _)| entity)
        .next();
    let Some(entity) = settings else {
        return false;
    };
    preview.trigger(SetValue {
        entity,
        component: TypeId::of::<DatabaseSettings>(),
        path: PropertyPath::new(["speed"]),
        value: json!(3.0),
    });
    true
}

#[test]
fn install_opens_and_saves_a_definition_in_the_editors_schedule() {
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
            asset_type: MotionDatabaseDefinition::name(),
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
    while !unsaved(&app) {
        app.update();
        assert!(Instant::now() < deadline, "the edit never reached the tab");
    }
    app.main_mut().world_mut().insert(SaveRequested, document);
    while unsaved(&app) {
        app.update();
        assert!(Instant::now() < deadline, "the definition was never saved");
    }

    let saved = read_definition(directory.path(), &asset).unwrap();
    assert_eq!(saved.speed, 3.0);
}

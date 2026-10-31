#[allow(dead_code)]
#[path = "../examples/custom_property.rs"]
mod example;

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
    schedule_groups::Update,
};
use concerto_ecs::{
    Component, Entity, World, command::EntityCommandQueue, entity::hierarchy::Children,
    reflect::ReflectComponent,
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
        EditorDocument, process_editor_commands,
    },
    guard::UnsavedGuard,
    hosted::{HostRequest, HostRole, HostedApp, begin, finish_blocking, trigger_on},
    inspector::{
        DocumentValue, InspectedComponent, InspectedValues, NameField, PlainRow, PropertyRow,
        PropertyRowValue, PropertyTarget, RefreshInspector, ValueCard,
    },
    project::ProjectState,
    tools::selection::Select,
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::ImportProvenance,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_ui::{node::UINode, scroll::UIScrollArea};
use concerto_window::plugin::WindowPlugin;
use example::{Setting, SettingButton, SettingEditor};
use std::{marker::PhantomData, sync::Arc};

#[derive(Reflect, Default, Clone, PartialEq, Debug)]
#[reflect(Default)]
enum Mode {
    #[default]
    Calm,
    Loud,
}

#[derive(Reflect, Default)]
struct Tuning {
    setting: Setting,
    samples: Vec<f32>,
}

#[derive(Reflect, Default)]
struct Staged {
    mode: Mode,
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Wave {
    samples: Vec<f32>,
    mode: Mode,
}

#[derive(Component)]
struct Held {
    tuning: Tuning,
    staged: Staged,
}

fn held() -> Held {
    Held {
        tuning: Tuning {
            setting: Setting {
                title: "Shadows".into(),
                enabled: false,
            },
            samples: vec![1.0],
        },
        staged: Staged::default(),
    }
}

fn tuning(document: Entity) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<Held> {
        name: "Tuning".into(),
        document,
        read: Box::new(|held| Some(&held.tuning)),
        edit: Box::new(|held| Some(&mut held.tuning)),
    })
}

fn staged(document: Entity) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<Held> {
        name: "Staged".into(),
        document,
        read: Box::new(|held| Some(&held.staged)),
        edit: Box::new(|held| Some(&mut held.staged)),
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Notes;
impl Asset for Notes {
    fn name() -> &'static str {
        "Notes"
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Sketch;
impl Asset for Sketch {
    fn name() -> &'static str {
        "Sketch"
    }
}

struct ValueEditor<A>(PhantomData<fn() -> A>);
impl<A> Default for ValueEditor<A> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<A: Asset> AssetEditor for ValueEditor<A> {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let document = editor.entity();
        editor.insert((held(), InspectedValues(vec![tuning(document)])));
    }
}

fn preview_recipe(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin);
    app.register_type::<Staged>();
}

fn preview() -> HostedApp {
    finish_blocking(begin(
        HostRequest {
            gpu: None,
            recipe: preview_recipe,
            size: (0, 0),
            scale_factor: 1.0,
            role: HostRole::Preview,
            clipboard: None,
        },
        |_| {},
        |_| {},
    ))
}

struct Editing {
    editor: App,
    scene: Entity,
    setting: Entity,
    wave: Entity,
}

fn editing() -> Editing {
    let mut members = None;
    let (mut editor, scene) = example::headless_editor(
        |scene| {
            scene.register_type::<Wave>();
            let world = scene.main_mut().world_mut();
            let setting = world.spawn(Setting {
                title: "Shadows".into(),
                enabled: false,
            });
            members = Some((setting, world.spawn(Wave::default())));
        },
        |editor| {
            let mut editors = AssetEditorRegistry::default();
            editors.register::<Notes, ValueEditor<Notes>>().unwrap();
            editors.register::<Sketch, ValueEditor<Sketch>>().unwrap();
            editor
                .register_type::<Tuning>()
                .insert_resource(editors)
                .insert_resource(AssetEditorCommands::default())
                .insert_resource(AssetRegistry::default())
                .insert_resource(ProjectState::default())
                .insert_resource(UnsavedGuard::default())
                .add_system(Update, process_editor_commands);
        },
    );
    editor.main_mut().world_mut().insert(
        EditorDocument {
            asset_type: "Scene",
            title: "Scene".into(),
            status: String::new(),
            failed: None,
            current: None,
            pending: None,
            order: 0,
        },
        scene,
    );
    let (setting, wave) = members.unwrap();
    Editing {
        editor,
        scene,
        setting,
        wave,
    }
}

impl Editing {
    fn world(&mut self) -> &mut World {
        self.editor.main_mut().world_mut()
    }

    fn command(&mut self, command: AssetEditorCommand) {
        self.world()
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(command);
        self.editor.update();
    }

    fn open<A: Asset>(&mut self) -> Entity {
        let asset = AssetEntry {
            id: AssetId::new(),
            address: format!("{}.gasset", A::name()),
            kind: A::name().into(),
            provenance: Some(ImportProvenance {
                source: "fixture".into(),
                sub_asset: A::name().into(),
            }),
        };
        self.world()
            .get_resource_mut::<AssetRegistry>()
            .unwrap()
            .insert(asset.clone());
        self.command(AssetEditorCommand::Open(asset));
        self.world()
            .get_resource::<ActiveEditor>()
            .unwrap()
            .0
            .unwrap()
    }

    fn activate(&mut self, document: Entity) {
        self.command(AssetEditorCommand::Activate(document));
    }

    fn select_in_scene(&mut self, entity: Entity) {
        let scene = self.scene;
        trigger_on(self.world(), scene, Select(Some(entity)));
        self.editor.update();
    }

    fn scroll_to(&mut self, offset: f32) {
        let world = self.world();
        let area = world
            .query::<(Entity, &UIScrollArea), ()>()
            .iter(world)
            .map(|(entity, _)| entity)
            .next()
            .unwrap();
        world
            .get_component_for_entity_mut::<UIScrollArea>(area)
            .unwrap()
            .offset = offset;
    }

    fn scroll(&mut self) -> f32 {
        let world = self.world();
        world
            .query::<&UIScrollArea, ()>()
            .iter(world)
            .next()
            .unwrap()
            .offset
    }

    fn value_cards(&mut self) -> Vec<String> {
        let world = self.world();
        let mut names: Vec<_> = world
            .query::<&ValueCard, ()>()
            .iter(world)
            .map(|card| card.name.clone())
            .collect();
        names.sort();
        names
    }

    fn value_card_ids(&mut self) -> Vec<Entity> {
        let world = self.world();
        world
            .query::<(Entity, &ValueCard), ()>()
            .iter(world)
            .map(|(entity, _)| entity)
            .collect()
    }

    fn component_cards(&mut self) -> usize {
        let world = self.world();
        world.query::<&InspectedComponent, ()>().iter(world).count()
    }

    fn rows_with_widgets(&mut self) -> Vec<String> {
        let world = self.world();
        let rows: Vec<_> = world
            .query::<(Entity, &PropertyRow), ()>()
            .iter(world)
            .map(|(entity, row)| (entity, row.address.path.name().to_string()))
            .collect();
        let mut names: Vec<_> = rows
            .into_iter()
            .filter(|(row, _)| {
                world
                    .get_component_for_entity::<Children>(*row)
                    .is_some_and(|children| children.iter().count() > 1)
            })
            .map(|(_, name)| name)
            .collect();
        names.sort();
        names
    }

    fn plain_rows(&mut self) -> Vec<String> {
        let world = self.world();
        world
            .query::<&PlainRow, ()>()
            .iter(world)
            .map(|row| row.path.name().to_string())
            .collect()
    }

    fn shown_enabled(&mut self) -> Vec<bool> {
        let world = self.world();
        world
            .query::<&PropertyRowValue, ()>()
            .iter(world)
            .filter_map(|value| value.snapshot::<Setting, SettingEditor>())
            .map(|snapshot| snapshot.enabled)
            .collect()
    }

    fn set_enabled(&mut self, document: Entity, enabled: bool) {
        self.world()
            .get_component_for_entity_mut::<Held>(document)
            .unwrap()
            .tuning
            .setting
            .enabled = enabled;
    }
}

#[test]
fn a_document_value_shows_one_card_on_the_frame_its_tab_opens() {
    let mut w = editing();

    w.open::<Notes>();

    assert_eq!(w.value_cards(), ["Tuning"]);
    assert_eq!(w.component_cards(), 0);
    assert_eq!(w.rows_with_widgets(), ["samples", "samples", "setting"]);
    let world = w.world();
    let field = world
        .query::<(Entity, &NameField), ()>()
        .iter(world)
        .map(|(entity, _)| entity)
        .next()
        .unwrap();
    assert!(
        !world
            .get_component_for_entity::<UINode>(field)
            .unwrap()
            .visible
    );
}

#[test]
fn replacing_the_values_shows_the_new_cards_on_the_same_frame_and_resets_the_scroll() {
    let mut w = editing();
    let document = w.open::<Notes>();
    w.scroll_to(40.0);

    let list = vec![tuning(document), staged(document)];
    w.world().insert(InspectedValues(list), document);
    w.editor.update();

    assert_eq!(w.value_cards(), ["Staged", "Tuning"]);
    assert_eq!(w.scroll(), 0.0);
}

#[test]
fn the_same_targets_keep_their_cards_and_the_scroll() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let cards = w.value_card_ids();
    w.scroll_to(40.0);

    let same = w
        .world()
        .get_component_for_entity::<InspectedValues>(document)
        .unwrap()
        .0
        .clone();
    w.world().insert(InspectedValues(same), document);
    w.editor.update();

    assert_eq!(w.value_card_ids(), cards);
    assert_eq!(w.scroll(), 40.0);
}

#[test]
fn switching_documents_rebuilds_the_cards_and_resets_the_scroll() {
    let mut w = editing();
    let notes = w.open::<Notes>();
    let first = w.value_card_ids();
    w.scroll_to(40.0);

    let sketch = w.open::<Sketch>();
    assert_ne!(notes, sketch);
    let second = w.value_card_ids();

    assert_eq!(second.len(), 1);
    assert_ne!(first, second);
    assert_eq!(w.scroll(), 0.0);

    w.scroll_to(40.0);
    w.activate(notes);
    assert_eq!(w.value_cards(), ["Tuning"]);
    assert_ne!(w.value_card_ids(), second);
    assert_eq!(w.scroll(), 0.0);
}

#[test]
fn a_write_by_other_means_shows_nothing_until_refresh_inspector() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let cards = w.value_card_ids();
    assert_eq!(w.shown_enabled(), [false]);

    w.set_enabled(document, true);
    w.editor.update();
    assert_eq!(w.shown_enabled(), [false]);

    w.world().trigger(RefreshInspector);
    w.editor.update();
    assert_eq!(w.shown_enabled(), [true]);
    assert_eq!(w.value_card_ids(), cards);
}

#[test]
fn a_target_whose_document_has_gone_shows_a_card_with_no_rows() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let gone = w.world().spawn(held());
    w.world().despawn(gone);

    w.world()
        .insert(InspectedValues(vec![tuning(gone)]), document);
    w.editor.update();

    assert_eq!(w.value_cards(), ["Tuning"]);
    assert!(w.rows_with_widgets().is_empty());
    assert!(w.plain_rows().is_empty());
}

#[test]
fn closing_a_value_tab_takes_its_cards_away() {
    let mut w = editing();
    let notes = w.open::<Notes>();

    w.command(AssetEditorCommand::Close(notes));

    assert_eq!(w.value_cards(), Vec::<String>::new());
    assert!(w.rows_with_widgets().is_empty());
}

#[test]
fn going_to_a_scene_tab_and_back_shows_each_tabs_own_cards_with_their_widgets() {
    let mut w = editing();
    w.select_in_scene(w.setting);
    let scene = w.scene;
    assert_eq!(w.component_cards(), 1);

    let notes = w.open::<Notes>();
    assert_eq!((w.component_cards(), w.value_cards().len()), (0, 1));
    assert_eq!(w.rows_with_widgets(), ["samples", "samples", "setting"]);

    w.activate(scene);
    assert_eq!((w.component_cards(), w.value_cards().len()), (1, 0));
    let world = w.world();
    assert_eq!(world.query::<&SettingButton, ()>().iter(world).count(), 1);

    w.activate(notes);
    assert_eq!((w.component_cards(), w.value_cards().len()), (0, 1));
    assert_eq!(w.rows_with_widgets(), ["samples", "samples", "setting"]);
}

#[test]
fn a_scenes_list_and_enum_rows_build_with_a_value_document_open_behind_them() {
    let mut w = editing();
    w.select_in_scene(w.wave);
    let scene = w.scene;
    w.open::<Notes>();

    w.activate(scene);

    assert_eq!(w.value_cards(), Vec::<String>::new());
    assert_eq!(w.rows_with_widgets(), ["mode", "samples"]);
}

#[test]
fn a_type_only_the_preview_registers_shows_once_the_preview_arrives() {
    let mut w = editing();
    let document = w.open::<Notes>();
    w.world()
        .insert(InspectedValues(vec![staged(document)]), document);
    w.editor.update();
    assert_eq!(w.plain_rows(), ["mode"]);

    w.world().insert(preview(), document);
    w.editor.update();

    assert!(w.plain_rows().is_empty());
    assert_eq!(w.rows_with_widgets(), ["mode"]);
}

#[test]
fn a_type_only_the_preview_registers_shows_when_the_preview_came_first() {
    let mut w = editing();
    let document = w.open::<Notes>();
    w.world().insert(preview(), document);
    w.editor.update();

    w.world()
        .insert(InspectedValues(vec![staged(document)]), document);
    w.editor.update();

    assert!(w.plain_rows().is_empty());
    assert_eq!(w.rows_with_widgets(), ["mode"]);
}

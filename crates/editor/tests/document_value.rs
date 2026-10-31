#[allow(dead_code)]
#[path = "../examples/custom_property.rs"]
mod example;

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Component, Entity, ResMut, Resource, World,
    command::EntityCommandQueue,
    entity::hierarchy::{ChildOf, Children},
    events::{event_channel::EventChannel, event_reader::EventReader},
    reflect::ReflectComponent,
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
        EditorDocument, process_editor_commands,
    },
    guard::UnsavedGuard,
    hosted::{
        HostRequest, HostRole, Hosted, HostedApp, HostedEventsAppExt, SetEditTarget, begin,
        finish_blocking, trigger_on,
    },
    inspector::{
        DocumentValue, EditableApp, InspectedComponent, InspectedValues, NameField, PlainRow,
        PropertyCommits, PropertyRow, PropertyRowValue, PropertyTarget, RefreshInspector,
        ValueCard,
    },
    project::ProjectState,
    tools::{
        edits::{PropertyCommitted, PropertyRefusal},
        scene::SceneEdited,
        selection::Select,
    },
};
use concerto_foundation::assets::{
    Asset, AssetId,
    asset_server::AssetServer,
    asset_store::AssetStore,
    content::ImportProvenance,
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::Mesh;
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_ui::{
    focus::FocusedWidget,
    interaction::UIClick,
    node::UINode,
    scroll::UIScrollArea,
    text::UIText,
    text_input::{UITextInput, UITextInputSubmitted},
};
use concerto_window::{input::MouseButton, plugin::WindowPlugin};
use example::{Setting, SettingButton, SettingEdit, SettingEditor};
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

#[derive(Reflect, Default)]
struct Rates {
    fast: f32,
    slow: f32,
}

#[derive(Reflect, Default)]
struct Look {
    mesh: AssetHandle<Mesh>,
}

#[derive(Component)]
struct Held {
    tuning: Tuning,
    staged: Staged,
    rates: Rates,
    look: Look,
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
        rates: Rates::default(),
        look: Look::default(),
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

fn rates(document: Entity) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<Held> {
        name: "Rates".into(),
        document,
        read: Box::new(|held| Some(&held.rates)),
        edit: Box::new(|held| Some(&mut held.rates)),
    })
}

fn look(document: Entity) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<Held> {
        name: "Look".into(),
        document,
        read: Box::new(|held| Some(&held.look)),
        edit: Box::new(|held| Some(&mut held.look)),
    })
}

#[derive(Resource, Default)]
struct Reports(Vec<(Entity, Result<(), PropertyRefusal>)>);

fn record_reports(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    mut reports: ResMut<Reports>,
) {
    for Hosted { document, event } in committed.read() {
        reports.0.push((*document, event.result.clone()));
    }
}

#[derive(Resource, Default)]
struct SceneEdits(usize);

fn count_scene_edits(mut edited: EventReader<Hosted<SceneEdited>>, mut edits: ResMut<SceneEdits>) {
    edits.0 += edited.read().count();
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
                .register_plugin(AssetManagerPlugin)
                .register_type::<Tuning>()
                .register_type::<Rates>()
                .register_type::<Look>()
                .forward_hosted_event::<SceneEdited>()
                .insert_resource(editors)
                .insert_resource(AssetEditorCommands::default())
                .insert_resource(AssetRegistry::default())
                .insert_resource(ProjectState::default())
                .insert_resource(UnsavedGuard::default())
                .insert_resource(Reports::default())
                .insert_resource(SceneEdits::default())
                .add_system(Update, process_editor_commands)
                .add_system(LateUpdate, record_reports)
                .add_system(LateUpdate, count_scene_edits);
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

    fn held(&mut self, document: Entity) -> &Held {
        self.world()
            .get_component_for_entity::<Held>(document)
            .unwrap()
    }

    fn row_entity(&mut self, path: &str) -> Entity {
        let world = self.world();
        world
            .query::<(Entity, &PropertyRow), ()>()
            .iter(world)
            .find(|(_, row)| row.address.path.to_string() == path)
            .map(|(entity, _)| entity)
            .unwrap()
    }

    fn row(&mut self, path: &str) -> PropertyRow {
        let entity = self.row_entity(path);
        self.world()
            .get_component_for_entity::<PropertyRow>(entity)
            .unwrap()
            .clone()
    }

    fn commit(&mut self, row: &PropertyRow, edit: SettingEdit) {
        self.world()
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .push::<Setting, SettingEditor>(row, edit)
            .unwrap();
    }

    fn queued_commits(&mut self) -> usize {
        self.world()
            .get_resource::<PropertyCommits>()
            .unwrap()
            .0
            .len()
    }

    fn reports(&mut self) -> &[(Entity, Result<(), PropertyRefusal>)] {
        &self.world().get_resource::<Reports>().unwrap().0
    }

    fn status(&mut self, document: Entity) -> String {
        self.world()
            .get_component_for_entity::<EditorDocument>(document)
            .unwrap()
            .status
            .clone()
    }

    fn click(&mut self, entity: Entity) {
        self.world().trigger_on(
            entity,
            UIClick {
                position: Default::default(),
                button: MouseButton::Left,
            },
        );
    }

    fn click_last_child(&mut self, parent: Entity) {
        let last = self
            .world()
            .get_component_for_entity::<Children>(parent)
            .unwrap()
            .iter()
            .copied()
            .last()
            .unwrap();
        self.click(last);
    }

    fn click_labelled(&mut self, label: &str) {
        let world = self.world();
        let text = world
            .query::<(Entity, &UIText), ()>()
            .iter(world)
            .find(|(_, text)| text.text == label)
            .map(|(entity, _)| entity)
            .unwrap();
        let item = world
            .get_component_for_entity::<ChildOf>(text)
            .unwrap()
            .parent();
        self.click(item);
    }

    fn text_field(&mut self, row: Entity) -> Entity {
        let world = self.world();
        let mut pending = vec![row];
        while let Some(entity) = pending.pop() {
            if world
                .get_component_for_entity::<UITextInput>(entity)
                .is_some()
            {
                return entity;
            }
            if let Some(children) = world.get_component_for_entity::<Children>(entity) {
                pending.extend(children.iter().copied());
            }
        }
        panic!("the row has no text field");
    }

    fn field_text(&mut self, field: Entity) -> String {
        self.world()
            .get_component_for_entity::<UITextInput>(field)
            .unwrap()
            .value
            .clone()
    }

    fn type_into(&mut self, field: Entity, text: &str) {
        self.world()
            .get_component_for_entity_mut::<UITextInput>(field)
            .unwrap()
            .value = text.into();
    }
}

fn refused(status: &str) -> Result<(), PropertyRefusal> {
    Err(PropertyRefusal::Refused(status.into()))
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
fn a_value_tab_closing_in_the_frame_a_scene_takes_over_leaves_the_scenes_cards() {
    let mut w = editing();
    w.select_in_scene(w.setting);
    let scene = w.scene;
    let notes = w.open::<Notes>();

    let commands = &mut w
        .world()
        .get_resource_mut::<AssetEditorCommands>()
        .unwrap()
        .0;
    commands.push_back(AssetEditorCommand::Close(notes));
    commands.push_back(AssetEditorCommand::Activate(scene));
    w.editor.update();

    assert_eq!((w.component_cards(), w.value_cards().len()), (1, 0));
    let world = w.world();
    assert_eq!(world.query::<&SettingButton, ()>().iter(world).count(), 1);
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

#[test]
fn editing_a_value_writes_it_and_reports_one_event_and_nothing_to_any_scene() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let row = w.row("setting");

    w.commit(&row, SettingEdit::Toggle);
    w.editor.update();

    assert!(w.held(document).tuning.setting.enabled);
    assert_eq!(w.shown_enabled(), [true]);
    assert_eq!(w.reports(), [(document, Ok(()))]);
    assert_eq!(w.queued_commits(), 0);

    let scene = w.scene;
    w.activate(scene);
    w.editor.update();
    assert_eq!(w.world().get_resource::<SceneEdits>().unwrap().0, 0);
}

#[test]
fn an_edit_followed_by_a_tab_switch_names_the_edited_document() {
    let mut w = editing();
    let notes = w.open::<Notes>();
    let row = w.row("setting");
    let sketch = w.open::<Sketch>();

    w.commit(&row, SettingEdit::Toggle);
    w.editor.update();

    assert!(w.held(notes).tuning.setting.enabled);
    assert!(!w.held(sketch).tuning.setting.enabled);
    assert_eq!(w.reports(), [(notes, Ok(()))]);
    assert_eq!(w.shown_enabled(), [false]);
}

#[test]
fn a_refused_edit_leaves_the_value_reports_it_and_sets_its_own_documents_status() {
    let mut w = editing();
    let notes = w.open::<Notes>();
    let row = w.row("setting");
    let sketch = w.open::<Sketch>();
    let front_status = w.status(sketch);

    w.commit(&row, SettingEdit::Rename(String::new()));
    w.editor.update();

    let status = "Set Tuning.setting failed: the value was refused";
    assert_eq!(w.held(notes).tuning.setting.title, "Shadows");
    assert_eq!(w.reports(), [(notes, refused(status))]);
    assert_eq!(w.status(notes), status);
    assert_eq!(w.status(sketch), front_status);
}

#[test]
fn a_commit_read_before_the_editors_changed_is_refused_and_reported() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let row = w.row("setting");

    w.editor
        .register_property_editor::<Setting, SettingEditor>();
    w.commit(&row, SettingEdit::Toggle);
    w.editor.update();

    let status = "Set Tuning.setting failed: its editor changed since it was read";
    assert!(!w.held(document).tuning.setting.enabled);
    assert_eq!(w.reports(), [(document, refused(status))]);
    assert_eq!(w.status(document), status);
}

#[test]
fn an_edit_of_a_target_whose_document_has_gone_reports_a_missing_target() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let row = w.row("setting");
    w.command(AssetEditorCommand::Close(document));

    w.commit(&row, SettingEdit::Toggle);
    w.editor.update();

    assert_eq!(
        w.reports(),
        [(
            document,
            refused(
                "Set Tuning.setting failed: its entity, component or document no longer exists"
            )
        )]
    );
}

#[test]
fn a_scene_in_front_takes_its_own_commits_and_leaves_a_values_to_its_document() {
    let mut w = editing();
    w.select_in_scene(w.setting);
    let scene = w.scene;
    let component = w.row("");
    let notes = w.open::<Notes>();
    let value = w.row("setting");
    w.activate(scene);

    w.commit(&component, SettingEdit::Toggle);
    w.commit(&value, SettingEdit::Toggle);
    w.editor.update();
    w.editor.update();

    assert!(w.held(notes).tuning.setting.enabled);
    assert_eq!(w.shown_enabled(), [true]);
    assert_eq!(w.queued_commits(), 0);
    assert_eq!(w.reports().len(), 2);
    assert!(w.reports().iter().all(|(_, result)| result.is_ok()));
}

#[test]
fn a_component_commit_left_with_a_value_document_in_front_waits_for_its_scene() {
    let mut w = editing();
    w.select_in_scene(w.setting);
    let scene = w.scene;
    let component = w.row("");
    let notes = w.open::<Notes>();
    let value = w.row("setting");

    w.commit(&component, SettingEdit::Toggle);
    w.commit(&value, SettingEdit::Toggle);
    w.editor.update();

    assert!(w.held(notes).tuning.setting.enabled);
    assert_eq!(w.reports(), [(notes, Ok(()))]);
    assert_eq!(w.queued_commits(), 1);

    w.activate(scene);
    w.editor.update();
    assert_eq!(w.queued_commits(), 0);
    assert_eq!(w.shown_enabled(), [true]);
}

#[test]
fn leaving_a_scene_drops_its_commits_and_keeps_a_values() {
    let mut w = editing();
    w.select_in_scene(w.setting);
    let scene = w.scene;
    let component = w.row("");
    let notes = w.open::<Notes>();
    let value = w.row("setting");
    w.activate(scene);

    w.commit(&component, SettingEdit::Toggle);
    w.commit(&value, SettingEdit::Toggle);
    w.world().trigger(SetEditTarget(None));
    w.editor.update();
    w.editor.update();

    assert!(w.held(notes).tuning.setting.enabled);
    assert_eq!(w.reports(), [(notes, Ok(()))]);
    assert_eq!(w.queued_commits(), 0);
}

#[test]
fn a_lists_buttons_add_to_and_remove_from_a_value() {
    let mut w = editing();
    let document = w.open::<Notes>();

    let list = w.row_entity("samples");
    w.click_last_child(list);
    w.editor.update();
    assert_eq!(w.held(document).tuning.samples, [1.0, 0.0]);
    assert_eq!(w.reports(), [(document, Ok(()))]);

    let element = w.row_entity("samples.0");
    w.click_last_child(element);
    w.editor.update();
    assert_eq!(w.held(document).tuning.samples, [0.0]);
    let world = w.world();
    let rows = world
        .query::<&PropertyRow, ()>()
        .iter(world)
        .filter(|row| row.address.path.name() == "samples")
        .count();
    assert_eq!(rows, 2);
}

#[test]
fn a_value_edit_keeps_the_rows_so_a_focused_field_keeps_its_text() {
    let mut w = editing();
    let document = w.open::<Notes>();
    w.world()
        .insert(InspectedValues(vec![rates(document)]), document);
    w.editor.update();
    let (fast, slow) = (w.row_entity("fast"), w.row_entity("slow"));
    let (fast_field, slow_field) = (w.text_field(fast), w.text_field(slow));

    **w.world().get_resource_mut::<FocusedWidget>().unwrap() = Some(slow_field);
    w.type_into(slow_field, "12");
    w.type_into(fast_field, "3");
    w.world()
        .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
        .unwrap()
        .push_event(UITextInputSubmitted {
            entity: fast_field,
            value: "3".into(),
        });
    w.editor.update();
    w.editor.update();

    assert_eq!(w.held(document).rates.fast, 3.0);
    assert_eq!((w.row_entity("fast"), w.row_entity("slow")), (fast, slow));
    assert_eq!(w.field_text(slow_field), "12");
}

#[test]
fn a_picked_asset_lands_in_a_value_when_the_editors_server_knows_its_type() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let store = AssetStore::<Mesh>::new();
    w.world()
        .get_resource_mut::<AssetServer>()
        .unwrap()
        .register_asset(&store);
    w.world().insert_resource(store);
    let cube = AssetEntry {
        id: AssetId::new(),
        address: "content/cube.gasset".into(),
        kind: Mesh::name().into(),
        provenance: None,
    };
    w.world()
        .get_resource_mut::<AssetRegistry>()
        .unwrap()
        .insert(cube.clone());
    w.world()
        .insert(InspectedValues(vec![look(document)]), document);
    w.editor.update();

    let row = w.row_entity("mesh");
    w.click_last_child(row);
    w.editor.update();
    w.click_labelled(cube.name());
    w.editor.update();

    assert_eq!(w.held(document).look.mesh.id(), cube.id);
    assert_eq!(w.reports(), [(document, Ok(()))]);
}

#[test]
fn a_commit_is_checked_against_the_registry_of_the_values_types() {
    let mut w = editing();
    let document = w.open::<Notes>();
    w.world().insert(preview(), document);
    w.world()
        .insert(InspectedValues(vec![staged(document)]), document);
    w.editor.update();

    let row = w.row_entity("mode");
    w.click_last_child(row);
    w.editor.update();
    w.click_labelled("Loud");
    w.editor.update();

    assert_eq!(w.held(document).staged.mode, Mode::Loud);
    assert_eq!(w.reports(), [(document, Ok(()))]);
}

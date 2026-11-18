use crate::example;

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Changed, Component, Entity, Query, ResMut, Resource, World,
    command::EntityCommandQueue,
    entity::hierarchy::{ChildOf, Children},
    events::{event_channel::EventChannel, event_reader::EventReader},
    reflect::AppTypeRegistry,
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
        EditorDocument, process_editor_commands,
    },
    guard::UnsavedGuard,
    hosted::{
        HostRequest, HostRole, Hosted, HostedApp, HostedEventsAppExt, begin, finish_blocking,
        trigger_on,
    },
    inspector::{
        DocumentValue, InspectedComponent, InspectedValues, PlainRow, PropertyCommits, PropertyRow,
        PropertyRowValue, PropertyTarget, ReflectRows, ReflectRowsPlugin, RefreshInspector,
        ShownValue, ValueCard,
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
    checkbox::{UICheckbox, UICheckboxChanged},
    focus::FocusedWidget,
    interaction::UIClick,
    node::UINode,
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

#[derive(Reflect, Default)]
struct Rates {
    fast: f32,
    slow: f32,
}

#[derive(Reflect, Default)]
struct Look {
    mesh: AssetHandle<Mesh>,
}

#[derive(Reflect, Default, Debug, PartialEq)]
struct Packing {
    mode: Mode,
    limit: Option<f32>,
    mips: bool,
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

#[derive(Clone, Debug, PartialEq)]
struct Report {
    document: Entity,
    result: Result<(), PropertyRefusal>,
}

#[derive(Resource, Default)]
struct Reports(Vec<Report>);

fn record_reports(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    mut reports: ResMut<Reports>,
) {
    for Hosted { document, event } in committed.read() {
        reports.0.push(Report {
            document: *document,
            result: event.result.clone(),
        });
    }
}

#[derive(Resource, Default)]
struct SceneEdits(usize);

fn count_scene_edits(mut edited: EventReader<Hosted<SceneEdited>>, mut edits: ResMut<SceneEdits>) {
    edits.0 += edited.read().count();
}

#[derive(Resource, Default)]
struct RowChanges(usize);

fn count_row_changes(
    changed: Query<&ReflectRows, Changed<ReflectRows>>,
    mut changes: ResMut<RowChanges>,
) {
    changes.0 += changed.iter().count();
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
}

fn editing() -> Editing {
    let mut setting = None;
    let (mut editor, scene) = example::headless_editor(
        |scene| {
            setting = Some(scene.main_mut().world_mut().spawn(Setting {
                title: "Shadows".into(),
                enabled: false,
            }));
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
                .insert_resource(RowChanges::default())
                .register_plugin(ReflectRowsPlugin)
                .add_system(Update, process_editor_commands)
                .add_system(LateUpdate, record_reports)
                .add_system(LateUpdate, count_scene_edits)
                .add_system(LateUpdate, count_row_changes);
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
    Editing {
        editor,
        scene,
        setting: setting.unwrap(),
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
            embedded_in: None,
            id: AssetId::new(),
            address: format!("{}.gasset", A::name()),
            kind: A::name().into(),
            kind_version: None,
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

    fn reports(&mut self) -> &[Report] {
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

    fn check(&mut self, row: Entity, checked: bool) {
        let world = self.world();
        let mut pending = vec![row];
        while let Some(entity) = pending.pop() {
            if world
                .get_component_for_entity::<UICheckbox>(entity)
                .is_some()
            {
                world.trigger_on(entity, UICheckboxChanged { checked });
                return;
            }
            if let Some(children) = world.get_component_for_entity::<Children>(entity) {
                pending.extend(children.iter().copied());
            }
        }
        panic!("the row has no checkbox");
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

fn ok(document: Entity) -> Report {
    Report {
        document,
        result: Ok(()),
    }
}

#[test]
fn a_documents_values_show_in_the_inspector_by_its_tab_its_shown_value_and_its_preview() {
    let mut w = editing();
    w.select_in_scene(w.setting);
    let scene = w.scene;
    let notes = w.open::<Notes>();
    assert_eq!(w.value_cards(), ["Tuning"]);
    assert_eq!(w.component_cards(), 0);
    assert_eq!(w.rows_with_widgets(), ["samples", "samples", "setting"]);

    w.set_enabled(notes, true);
    w.world().trigger(RefreshInspector);
    w.editor.update();
    assert_eq!(w.shown_enabled(), [true]);

    let shown = staged(notes);
    let values = vec![tuning(notes), shown.clone()];
    w.world()
        .insert((InspectedValues(values), ShownValue(shown)), notes);
    w.editor.update();
    assert_eq!(w.value_cards(), ["Staged"]);
    assert_eq!(w.plain_rows(), ["mode"]);
    w.world().insert(preview(), notes);
    w.editor.update();
    assert!(w.plain_rows().is_empty());
    assert_eq!(w.rows_with_widgets(), ["mode"]);

    w.activate(scene);
    assert_eq!(w.component_cards(), 1);
    assert!(w.value_cards().is_empty());
    let world = w.world();
    assert_eq!(world.query::<&SettingButton, ()>().iter(world).count(), 1);
    w.activate(notes);
    assert_eq!(w.component_cards(), 0);
    assert_eq!(w.value_cards(), ["Staged"]);
    w.command(AssetEditorCommand::Close(notes));
    assert!(w.value_cards().is_empty());
}

#[test]
fn a_value_edit_writes_its_own_document_and_reports_there_with_another_tab_in_front() {
    let mut w = editing();
    let notes = w.open::<Notes>();
    let row = w.row("setting");
    w.commit(&row, SettingEdit::Toggle);
    w.editor.update();
    assert!(w.held(notes).tuning.setting.enabled);
    assert_eq!(w.shown_enabled(), [true]);
    assert_eq!(w.reports(), [ok(notes)]);

    let sketch = w.open::<Sketch>();
    let front_status = w.status(sketch);
    w.commit(&row, SettingEdit::Rename(String::new()));
    w.editor.update();
    assert_eq!(w.held(notes).tuning.setting.title, "Shadows");
    assert!(w.reports()[1].result.is_err());
    assert!(!w.status(notes).is_empty());
    assert_eq!(w.status(sketch), front_status);

    w.commit(&row, SettingEdit::Toggle);
    w.editor.update();
    assert!(!w.held(notes).tuning.setting.enabled);
    assert!(!w.held(sketch).tuning.setting.enabled);
    assert_eq!(w.reports()[2], ok(notes));
    assert_eq!(w.world().get_resource::<SceneEdits>().unwrap().0, 0);
}

#[test]
fn a_values_list_asset_and_enum_rows_edit_it() {
    let mut w = editing();
    let document = w.open::<Notes>();
    let list = w.row_entity("samples");
    w.click_last_child(list);
    w.editor.update();
    assert_eq!(w.held(document).tuning.samples, [1.0, 0.0]);
    let element = w.row_entity("samples.0");
    w.click_last_child(element);
    w.editor.update();
    assert_eq!(w.held(document).tuning.samples, [0.0]);

    let store = AssetStore::<Mesh>::new();
    w.world()
        .get_resource_mut::<AssetServer>()
        .unwrap()
        .register_asset(&store);
    w.world().insert_resource(store);
    let cube = AssetEntry {
        embedded_in: None,
        id: AssetId::new(),
        address: "content/cube.gasset".into(),
        kind: Mesh::name().into(),
        kind_version: None,
        provenance: None,
    };
    w.world()
        .get_resource_mut::<AssetRegistry>()
        .unwrap()
        .insert(cube.clone());
    w.world()
        .insert(InspectedValues(vec![look(document)]), document);
    w.editor.update();
    let mesh = w.row_entity("mesh");
    w.click_last_child(mesh);
    w.editor.update();
    w.click_labelled(&cube.name());
    w.editor.update();
    w.world().insert(preview(), document);
    w.world()
        .insert(InspectedValues(vec![staged(document)]), document);
    w.editor.update();
    let mode = w.row_entity("mode");
    w.click_last_child(mode);
    w.editor.update();
    w.click_labelled("Loud");
    w.editor.update();

    assert_eq!(w.held(document).look.mesh.id(), cube.id);
    assert_eq!(w.held(document).staged.mode, Mode::Loud);
    assert_eq!(w.reports(), vec![ok(document); 4]);
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
fn reflect_rows_edit_their_value_and_tell_the_owner_by_change_detection() {
    let mut w = editing();
    let types = AppTypeRegistry::default();
    types.write().register::<Packing>();
    let widget = w.world().spawn((
        UINode::default(),
        ReflectRows {
            value: Box::new(Packing::default()),
            types,
        },
    ));
    w.editor.update();
    let changes = w.world().get_resource::<RowChanges>().unwrap().0;

    let mode = w.row_entity("mode");
    w.click_last_child(mode);
    w.editor.update();
    w.click_labelled("Loud");
    w.editor.update();
    let limit = w.row_entity("limit");
    w.check(limit, true);
    w.editor.update();
    let mips = w.row_entity("mips");
    w.check(mips, true);
    w.editor.update();

    let world = w.world();
    let rows = world
        .get_component_for_entity::<ReflectRows>(widget)
        .unwrap();
    let value: &dyn std::any::Any = rows.value.as_ref();
    assert_eq!(
        value.downcast_ref::<Packing>(),
        Some(&Packing {
            mode: Mode::Loud,
            limit: Some(0.0),
            mips: true,
        })
    );
    assert_eq!(world.get_resource::<RowChanges>().unwrap().0, changes + 3);
}

//! Exercises the downstream API without access to inspector internals.
#[path = "../examples/custom_property.rs"]
mod example;

use concerto_app::{App, edit_time::AtEditTime, schedule_groups::Update};
use concerto_ecs::{
    Component, Entity, IntoSystem, Query, Res, ResMut, Resource, System, World,
    command::CommandQueue,
    events::event_reader::EventReader,
    reflect::{AppTypeRegistry, ReflectComponent},
    signal::{On, Signal},
};
use concerto_editor::hosted::{EditWorld, Hosted, HostedEventsAppExt, trigger_on};
use concerto_editor::inspector::{
    ComponentEdit, EditContext, EditError, EditableApp, InspectorRegistry, PropertyCommit,
    PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};
use concerto_editor::tools::{
    edits::EditComponent,
    scene::SceneEdited,
    selection::{Select, Selection},
};
use concerto_foundation::assets::{asset_server::AssetServer, registry::AssetRegistry};
use concerto_reflect::{PropertyPath, Reflect, ReflectDefault, TypeRegistry};
use concerto_ui::{interaction::UIClick, text::UIText, theme::UITheme};
use concerto_window::input::MouseButton;
use example::{Setting, SettingButton, SettingEdit, SettingEditor};
use std::any::TypeId;

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Container {
    setting: Setting,
    gain: f32,
}

fn setting() -> Setting {
    Setting {
        title: "Shadows".into(),
        enabled: false,
    }
}

struct Despawn(Entity);
impl Signal for Despawn {}

fn despawn(on: On<Despawn>, mut cmd: CommandQueue) {
    cmd.despawn(on.signal().0);
}

#[derive(Resource, Default)]
struct Edits(usize);

fn count_edit(mut edited: EventReader<Hosted<SceneEdited>>, mut edits: ResMut<Edits>) {
    edits.0 += edited.read().count();
}

#[derive(Resource, Default)]
struct Seen {
    containers: Vec<(Entity, String, bool)>,
    settings: Vec<(Entity, bool)>,
    selected: Option<Entity>,
}

fn look(
    containers: EditWorld<Query<(Entity, &Container)>>,
    settings: EditWorld<Query<(Entity, &Setting)>>,
    selection: EditWorld<Res<Selection>>,
    mut seen: ResMut<Seen>,
) {
    seen.containers = containers
        .iter()
        .map(|(entity, container)| {
            (
                entity,
                container.setting.title.clone(),
                container.setting.enabled,
            )
        })
        .collect();
    seen.settings = settings
        .iter()
        .map(|(entity, setting)| (entity, setting.enabled))
        .collect();
    seen.selected = selection.entity();
}

struct Inspecting {
    editor: App,
    document: Entity,
    a: Entity,
    b: Entity,
    root: Entity,
}

fn inspecting() -> Inspecting {
    let mut spawned = None;
    let (editor, document) = example::headless_editor(
        |scene| {
            scene
                .register_type::<Container>()
                .add_listener(despawn.at_edit_time());
            let world = scene.main_mut().world_mut();
            let mut container = |gain| {
                world.spawn(Container {
                    setting: setting(),
                    gain,
                })
            };
            let (a, b) = (container(1.0), container(2.0));
            spawned = Some((a, b, world.spawn(setting())));
        },
        |editor| {
            editor
                .insert_resource(Edits::default())
                .insert_resource(Seen::default())
                .forward_hosted_event::<SceneEdited>()
                .add_system(Update, count_edit);
        },
    );
    let (a, b, root) = spawned.unwrap();
    Inspecting {
        editor,
        document,
        a,
        b,
        root,
    }
}

impl Inspecting {
    fn world(&mut self) -> &mut World {
        self.editor.main_mut().world_mut()
    }
    fn signal<S: Signal>(&mut self, signal: S) {
        let document = self.document;
        trigger_on(self.world(), document, signal);
    }
    fn select(&mut self, entity: Entity) {
        self.signal(Select(Some(entity)));
        self.editor.update();
    }
    fn seen(&mut self) -> &Seen {
        let mut system = look.into_system();
        system.initialize(self.world());
        system.run_and_apply((), self.world());
        self.world().get_resource::<Seen>().unwrap()
    }
    fn container(&mut self, entity: Entity) -> Option<(String, bool)> {
        self.seen()
            .containers
            .iter()
            .find(|(found, ..)| *found == entity)
            .map(|(_, title, enabled)| (title.clone(), *enabled))
    }
    fn enabled(&mut self, entity: Entity) -> bool {
        self.container(entity).unwrap().1
    }
    fn edits(&mut self) -> usize {
        self.editor.update();
        self.world().get_resource::<Edits>().unwrap().0
    }
    fn rows(&mut self) -> Vec<(Entity, PropertyRow, PropertyRowValue)> {
        let world = self.world();
        world
            .query::<(Entity, &PropertyRow, &PropertyRowValue), ()>()
            .iter(world)
            .map(|(entity, row, value)| (entity, row.clone(), value.clone()))
            .collect()
    }
    fn setting_row(&mut self) -> (Entity, PropertyRow, PropertyRowValue) {
        self.rows()
            .into_iter()
            .find(|(_, row, _)| row.type_id == TypeId::of::<Setting>())
            .expect("the inspector shows the selected entity's setting")
    }
    fn send(&mut self, commit: PropertyCommit) {
        self.world()
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .0
            .push(commit);
        self.editor.update();
    }
}

fn commit(row: &PropertyRow, edit: SettingEdit) -> PropertyCommit {
    PropertyCommit::new::<Setting, SettingEditor>(row, edit).unwrap()
}

#[test]
fn registered_composite_is_one_row_and_fallback_retains_unsupported_fields() {
    let registry = InspectorRegistry::default();
    let value = Container {
        setting: setting(),
        gain: 1.0,
    };
    let fallback = registry.collect(&value, &TypeRegistry::default());
    assert_eq!(
        fallback.iter().map(|p| p.path.clone()).collect::<Vec<_>>(),
        vec![
            PropertyPath::new(["setting", "title"]),
            PropertyPath::new(["setting", "enabled"]),
            PropertyPath::new(["gain"])
        ]
    );
    assert!(!fallback[0].has_editor());
    assert!(fallback[1].has_editor());
    assert!(fallback[2].has_editor());
    assert!(
        fallback[0]
            .value
            .snapshot::<Setting, SettingEditor>()
            .is_none()
    );

    let mut registry = registry;
    registry.register_property_editor::<Setting, SettingEditor>();
    let properties = registry.collect(&value, &TypeRegistry::default());
    assert_eq!(properties.len(), 2);
    assert_eq!(properties[0].path, PropertyPath::new(["setting"]));
    assert_eq!(properties[0].type_id, TypeId::of::<Setting>());
    let snapshot = properties[0]
        .value
        .snapshot::<Setting, SettingEditor>()
        .unwrap();
    assert_eq!(snapshot.title, "Shadows");
    assert!(!snapshot.enabled);
    assert!(properties[0].value == registry.collect(&value, &TypeRegistry::default())[0].value);
    let mut changed = value;
    changed.setting.enabled = true;
    assert!(properties[0].value != registry.collect(&changed, &TypeRegistry::default())[0].value);
}

#[test]
fn the_inspector_shows_a_registered_composite_of_the_scene_as_one_row() {
    let mut w = inspecting();
    w.select(w.a);

    let rows = w.rows();
    let paths: Vec<_> = rows.iter().map(|(_, row, _)| row.path.clone()).collect();
    assert_eq!(
        paths,
        [PropertyPath::new(["setting"]), PropertyPath::new(["gain"])]
    );
    let a = w.a;
    assert!(rows.iter().all(|(_, row, _)| row.entity == a
        && row.component == TypeId::of::<Container>()
        && row.has_editor()));
    let snapshot = rows[0].2.snapshot::<Setting, SettingEditor>().unwrap();
    assert_eq!(snapshot.title, "Shadows");
    assert!(!snapshot.enabled);
}

#[test]
fn commits_use_captured_entity_and_live_value_and_validate_before_mutation() {
    let mut w = inspecting();
    w.select(w.a);
    let row = w.setting_row().1;
    w.select(w.b);
    let selected = w.seen().selected;
    assert_eq!(selected, Some(w.b));

    w.send(commit(&row, SettingEdit::Toggle));
    assert!(w.enabled(w.a));
    assert!(!w.enabled(w.b));
    assert_eq!(w.edits(), 1);

    w.send(commit(&row, SettingEdit::Rename(" ".into())));
    assert_eq!(w.container(w.a), Some(("Shadows".into(), true)));
    assert_eq!(w.edits(), 1, "a refused edit is not reported as an edit");

    w.send(commit(&row, SettingEdit::Toggle));
    assert!(
        !w.enabled(w.a),
        "the edit is applied to the value as it is now"
    );
    w.send(commit(&row, SettingEdit::Rename("Lighting".into())));
    assert_eq!(w.container(w.a), Some(("Lighting".into(), false)));
    assert_eq!(w.edits(), 3);
}

#[test]
fn a_value_edit_sets_the_whole_setting_and_is_still_validated() {
    let mut field = setting();
    let lighting = SettingEditor
        .edit_to(&Setting {
            title: "Lighting".into(),
            enabled: true,
        })
        .expect("the setting offers a value edit");
    let blank = SettingEditor
        .edit_to(&Setting {
            title: " ".into(),
            enabled: false,
        })
        .expect("the setting offers a value edit");
    let context = EditContext {
        assets: &AssetServer::new(),
        registry: &AssetRegistry::default(),
    };
    assert_eq!(
        SettingEditor.apply(&mut field, &blank, &context),
        Err(EditError::Rejected)
    );
    assert_eq!((field.title.as_str(), field.enabled), ("Shadows", false));
    SettingEditor
        .apply(&mut field, &lighting, &context)
        .unwrap();
    assert_eq!((field.title.as_str(), field.enabled), ("Lighting", true));

    let mut w = inspecting();
    w.select(w.a);
    let row = w.setting_row().1;
    w.send(commit(&row, blank));
    assert_eq!(w.container(w.a), Some(("Shadows".into(), false)));
    w.send(commit(&row, lighting));
    assert_eq!(w.container(w.a), Some(("Lighting".into(), true)));
    assert_eq!(w.edits(), 1);
}

#[test]
fn root_editors_and_owned_snapshots_work_without_cloning_the_domain_type() {
    let mut w = inspecting();
    w.select(w.root);
    let (_, row, value) = w.setting_row();
    assert_eq!(row.path, PropertyPath::default());
    assert_eq!(row.component, TypeId::of::<Setting>());

    w.send(commit(&row, SettingEdit::Toggle));

    let settings = w.seen().settings.clone();
    assert_eq!(settings, [(w.root, true)]);
    assert!(
        !value.snapshot::<Setting, SettingEditor>().unwrap().enabled,
        "snapshot is independent of world mutation"
    );
    assert!(
        w.setting_row()
            .2
            .snapshot::<Setting, SettingEditor>()
            .unwrap()
            .enabled,
        "the inspector took a fresh snapshot of the changed value"
    );
}

#[test]
fn replacing_an_adapter_invalidates_old_edits_even_for_the_same_adapter_type() {
    let mut w = inspecting();
    w.select(w.a);
    let old = w.setting_row().1;
    w.editor
        .register_property_editor::<Setting, SettingEditor>();

    w.send(commit(&old, SettingEdit::Toggle));
    assert!(!w.enabled(w.a));
    assert_eq!(w.edits(), 0);

    let current = w.setting_row().1;
    assert_ne!(old.registry_tick(), current.registry_tick());
    w.send(commit(&current, SettingEdit::Toggle));
    assert!(w.enabled(w.a));
    assert_eq!(w.edits(), 1);
}

#[test]
fn missing_targets_and_incompatible_paths_change_nothing_and_report_no_edit() {
    let mut w = inspecting();
    w.select(w.a);
    let row = w.setting_row().1;

    let mut wrong_type = commit(&row, SettingEdit::Toggle);
    wrong_type.row.path = PropertyPath::new(["gain"]);
    w.send(wrong_type);
    let mut missing = commit(&row, SettingEdit::Toggle);
    missing.row.path = PropertyPath::new(["absent"]);
    w.send(missing);
    assert_eq!(w.container(w.a), Some(("Shadows".into(), false)));

    w.signal(EditComponent(ComponentEdit::Remove {
        entity: w.a,
        component: TypeId::of::<Container>(),
    }));
    assert_eq!(w.edits(), 1, "removing the component is itself an edit");
    w.send(commit(&row, SettingEdit::Toggle));
    assert_eq!(w.container(w.a), None);

    w.select(w.b);
    let row = w.setting_row().1;
    w.signal(Despawn(w.b));
    w.send(commit(&row, SettingEdit::Toggle));
    let left = w.seen().containers.len();
    assert_eq!(left, 0);
    assert_eq!(w.edits(), 1);
}

#[derive(Resource)]
struct BuildRow {
    row: PropertyRow,
    shown: PropertyRowValue,
}

#[derive(Resource)]
struct Built(Entity);

fn build_widget(data: Res<BuildRow>, mut cmd: CommandQueue) {
    let mut row = cmd.spawn((data.row.clone(), data.shown.clone()));
    SettingEditor.build(
        &mut row,
        data.shown.snapshot::<Setting, SettingEditor>().unwrap(),
        &UITheme::default(),
    );
    let row = row.entity();
    cmd.insert_resource(Built(row));
}

#[test]
fn custom_widget_build_click_commit_and_refresh_smoke_test() {
    let mut w = inspecting();
    w.select(w.a);
    let (_, row, shown) = w.setting_row();
    w.world().insert_resource(BuildRow { row, shown });
    let mut build = build_widget.into_system();
    build.initialize(w.world());
    build.run_and_apply((), w.world());
    let row = w.world().get_resource::<Built>().unwrap().0;
    let world = w.world();
    let button = world
        .query::<(Entity, &SettingButton), ()>()
        .iter(world)
        .find(|(_, button)| button.0 == row)
        .map(|(entity, _)| entity)
        .unwrap();
    let text = |world: &World| {
        world
            .get_component_for_entity::<UIText>(button)
            .unwrap()
            .text
            .clone()
    };
    assert!(text(w.world()).contains("Off"));

    w.world().trigger_on(
        button,
        UIClick {
            position: glam::Vec2::ZERO,
            button: MouseButton::Left,
        },
    );
    assert_eq!(
        w.world().get_resource::<PropertyCommits>().unwrap().0.len(),
        1
    );
    w.editor.update();
    assert!(
        w.world()
            .get_resource::<PropertyCommits>()
            .unwrap()
            .0
            .is_empty()
    );
    assert!(w.enabled(w.a));

    let refreshed = w
        .rows()
        .into_iter()
        .find(|(entity, found, _)| *entity != row && found.type_id == TypeId::of::<Setting>())
        .map(|(_, _, value)| value)
        .expect("the inspector took a fresh snapshot");
    *w.world()
        .get_component_for_entity_mut::<PropertyRowValue>(row)
        .unwrap() = refreshed;
    w.editor.update();
    assert!(text(w.world()).contains("On"));
}

#[test]
fn registration_splits_between_the_recipe_and_the_editors_app() {
    let mut editor = App::new();
    editor.insert_resource(InspectorRegistry::default());
    example::install(&mut editor);
    assert_eq!(
        editor
            .get_resource::<InspectorRegistry>()
            .unwrap()
            .collect(&setting(), &TypeRegistry::default())
            .len(),
        1
    );
    let types = editor.get_resource::<AppTypeRegistry>().unwrap();
    assert!(types.get(TypeId::of::<Setting>()).is_none());

    let mut scene = App::new();
    example::register(&mut scene);
    let types = scene.get_resource::<AppTypeRegistry>().unwrap();
    assert!(types.get(TypeId::of::<Setting>()).is_some());
    assert!(scene.get_resource::<InspectorRegistry>().is_none());
}

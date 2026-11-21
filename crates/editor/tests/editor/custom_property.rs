//! Exercises the downstream API without access to inspector internals.
use crate::example;

use concerto_app::{App, schedule_groups::Update};
use concerto_ecs::{
    Component, Entity, IntoSystem, Query, Res, ResMut, Resource, System, World,
    command::CommandQueue, events::event_reader::EventReader, reflect::ReflectComponent,
};
use concerto_editor::hosted::{EditWorld, Hosted, HostedEventsAppExt, trigger_on};
use concerto_editor::inspector::{
    PropertyCommit, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};
use concerto_editor::tools::{scene::SceneEdited, selection::Select};
use concerto_reflect::{PropertyPath, Reflect, ReflectDefault};
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

#[derive(Resource, Default)]
struct Edits(usize);

fn count_edit(mut edited: EventReader<Hosted<SceneEdited>>, mut edits: ResMut<Edits>) {
    edits.0 += edited.read().count();
}

#[derive(Clone, Debug, PartialEq)]
struct Shown {
    entity: Entity,
    title: String,
    enabled: bool,
}

#[derive(Resource, Default)]
struct Seen(Vec<Shown>);

fn look(containers: EditWorld<Query<(Entity, &Container)>>, mut seen: ResMut<Seen>) {
    seen.0 = containers
        .iter()
        .map(|(entity, container)| Shown {
            entity,
            title: container.setting.title.clone(),
            enabled: container.setting.enabled,
        })
        .collect();
}

struct Inspecting {
    editor: App,
    document: Entity,
    a: Entity,
    b: Entity,
}

fn inspecting() -> Inspecting {
    let mut spawned = Vec::new();
    let (editor, document) = example::headless_editor(
        |scene| {
            scene.register_type::<Container>();
            let world = scene.main_mut().world_mut();
            for gain in [1.0, 2.0] {
                spawned.push(world.spawn(Container {
                    setting: Setting {
                        title: "Shadows".into(),
                        enabled: false,
                    },
                    gain,
                }));
            }
        },
        |editor| {
            editor
                .insert_resource(Edits::default())
                .insert_resource(Seen::default())
                .forward_hosted_event::<SceneEdited>()
                .add_system(Update, count_edit);
        },
    );
    Inspecting {
        editor,
        document,
        a: spawned[0],
        b: spawned[1],
    }
}

impl Inspecting {
    fn world(&mut self) -> &mut World {
        self.editor.main_mut().world_mut()
    }

    fn select(&mut self, entity: Entity) {
        let document = self.document;
        trigger_on(self.world(), document, Select(Some(entity)));
        self.editor.update();
    }

    fn shown(&mut self, entity: Entity) -> Shown {
        let mut system = look.into_system();
        system.initialize(self.world());
        system.run_and_apply((), self.world());
        self.world()
            .get_resource::<Seen>()
            .unwrap()
            .0
            .iter()
            .find(|shown| shown.entity == entity)
            .unwrap()
            .clone()
    }

    fn edits(&mut self) -> usize {
        self.editor.update();
        self.world().get_resource::<Edits>().unwrap().0
    }

    fn rows(&mut self) -> Vec<PropertyRow> {
        let world = self.world();
        world
            .query::<&PropertyRow, ()>()
            .iter(world)
            .cloned()
            .collect()
    }

    fn setting_row(&mut self) -> PropertyRow {
        self.rows()
            .into_iter()
            .find(|row| row.address.type_id == TypeId::of::<Setting>())
            .expect("the inspector shows the selected entity's setting")
    }

    fn inspector_value(&mut self, except: Option<Entity>) -> PropertyRowValue {
        let world = self.world();
        world
            .query::<(Entity, &PropertyRow, &PropertyRowValue), ()>()
            .iter(world)
            .find(|(entity, row, _)| {
                Some(*entity) != except && row.address.type_id == TypeId::of::<Setting>()
            })
            .map(|(_, _, value)| value.clone())
            .expect("the inspector shows the setting")
    }

    fn send(&mut self, row: &PropertyRow, edit: SettingEdit) {
        let commit = PropertyCommit::new::<Setting, SettingEditor>(row, edit).unwrap();
        self.world()
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .0
            .push(commit);
        self.editor.update();
    }
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
fn a_custom_editor_is_one_row_whose_widget_commits_validated_edits_to_its_rows_entity() {
    let mut w = inspecting();
    w.select(w.a);
    let rows = w.rows();
    let paths: Vec<_> = rows.iter().map(|row| row.address.path.clone()).collect();
    assert_eq!(
        paths,
        [PropertyPath::new(["setting"]), PropertyPath::new(["gain"])]
    );
    let row = w.setting_row();
    let shown = w.inspector_value(None);
    w.world().insert_resource(BuildRow {
        row: row.clone(),
        shown,
    });
    let mut build = build_widget.into_system();
    build.initialize(w.world());
    build.run_and_apply((), w.world());
    let built = w.world().get_resource::<Built>().unwrap().0;
    let world = w.world();
    let button = world
        .query::<(Entity, &SettingButton), ()>()
        .iter(world)
        .find(|(_, button)| button.0 == built)
        .map(|(entity, _)| entity)
        .unwrap();
    let label = |world: &World| {
        world
            .get_component_for_entity::<UIText>(button)
            .unwrap()
            .text
            .clone()
    };
    assert!(label(w.world()).contains("Off"));

    w.world().trigger_on(
        button,
        UIClick {
            position: glam::Vec2::ZERO,
            button: MouseButton::Left,
        },
    );
    w.editor.update();
    assert!(w.shown(w.a).enabled);
    assert_eq!(w.edits(), 1);
    let fresh = w.inspector_value(Some(built));
    *w.world()
        .get_component_for_entity_mut::<PropertyRowValue>(built)
        .unwrap() = fresh;
    w.editor.update();
    assert!(label(w.world()).contains("On"));

    w.select(w.b);
    w.send(&row, SettingEdit::Rename(" ".into()));
    assert_eq!(w.shown(w.a).title, "Shadows");
    assert_eq!(w.edits(), 1, "a refused edit is not reported as an edit");
    w.send(&row, SettingEdit::Rename("Lighting".into()));
    assert_eq!(w.shown(w.a).title, "Lighting");
    assert!(w.shown(w.a).enabled);
    assert_eq!(w.shown(w.b).title, "Shadows");
    assert_eq!(w.edits(), 2);
}

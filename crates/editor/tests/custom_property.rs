//! Exercises the downstream API without access to inspector internals.
#[path = "../examples/custom_property.rs"]
mod example;

use concerto_app::App;
use concerto_ecs::{
    Component, Entity, IntoSystem, Res, ResMut, Resource, System, World, command::CommandQueue,
};
use concerto_editable::{Editable, PropertyPath};
use concerto_editor::inspector::{
    EditError, InspectorRegistry, Property, PropertyCommit, PropertyCommits, PropertyEditor,
    PropertyRowValue, apply_property_commit, apply_property_commits,
};
use concerto_ui::{interaction::UIClick, text::UIText, theme::UITheme};
use concerto_window::input::MouseButton;
use example::{Setting, SettingButton, SettingEdit, SettingEditor};
use std::any::TypeId;

#[derive(Component, Editable, Default)]
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

fn world() -> (World, Entity, Entity) {
    let mut registry = InspectorRegistry::default();
    registry.register_component::<Container>();
    registry.register_component::<Setting>();
    registry.register_property_editor::<Setting, SettingEditor>(SettingEditor);
    let mut world = World::default();
    world.insert_resource(registry);
    world.insert_resource(PropertyCommits::default());
    let a = world.spawn(Container {
        setting: setting(),
        gain: 1.0,
    });
    let b = world.spawn(Container {
        setting: setting(),
        gain: 2.0,
    });
    world.tick();
    (world, a, b)
}

fn property(world: &World, entity: Entity) -> Property {
    world
        .get_resource::<InspectorRegistry>()
        .unwrap()
        .collect_component(world, entity, TypeId::of::<Container>())
        .unwrap()
        .remove(0)
}

fn commit(world: &World, entity: Entity, edit: SettingEdit) -> PropertyCommit {
    PropertyCommit::new::<Setting, SettingEditor>(
        &property(world, entity).row(entity, TypeId::of::<Container>()),
        edit,
    )
    .unwrap()
}

#[test]
fn registered_composite_is_one_row_and_fallback_retains_unsupported_fields() {
    let registry = InspectorRegistry::default();
    let value = Container {
        setting: setting(),
        gain: 1.0,
    };
    let fallback = registry.collect(&value);
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
    registry.register_property_editor::<Setting, SettingEditor>(SettingEditor);
    let properties = registry.collect(&value);
    assert_eq!(properties.len(), 2);
    assert_eq!(properties[0].path, PropertyPath::new(["setting"]));
    assert_eq!(properties[0].type_id, TypeId::of::<Setting>());
    let snapshot = properties[0]
        .value
        .snapshot::<Setting, SettingEditor>()
        .unwrap();
    assert_eq!(snapshot.title, "Shadows");
    assert!(!snapshot.enabled);
    assert!(properties[0].value == registry.collect(&value)[0].value);
    let mut changed = value;
    changed.setting.enabled = true;
    assert!(properties[0].value != registry.collect(&changed)[0].value);
}

#[test]
fn commits_use_captured_entity_and_live_value_and_validate_before_mutation() {
    let (mut world, a, b) = world();
    let mut selection = concerto_editor::selection::Selection::default();
    selection.select_entity(b);
    world.insert_resource(selection);
    let toggle = commit(&world, a, SettingEdit::Toggle);
    apply_property_commit(&mut world, toggle).unwrap();
    assert!(
        world
            .get_component_for_entity::<Container>(a)
            .unwrap()
            .setting
            .enabled
    );
    assert!(
        !world
            .get_component_for_entity::<Container>(b)
            .unwrap()
            .setting
            .enabled
    );
    assert!(world.was_component_changed(a, TypeId::of::<Container>()));
    assert!(!world.was_component_changed(b, TypeId::of::<Container>()));

    let invalid = commit(&world, a, SettingEdit::Rename(" ".into()));
    assert_eq!(
        apply_property_commit(&mut world, invalid),
        Err(EditError::Rejected)
    );
    let value = &world
        .get_component_for_entity::<Container>(a)
        .unwrap()
        .setting;
    assert_eq!(value.title, "Shadows");
    assert!(value.enabled);
    let rename = commit(&world, a, SettingEdit::Rename("Lighting".into()));
    apply_property_commit(&mut world, rename).unwrap();
    assert_eq!(
        world
            .get_component_for_entity::<Container>(a)
            .unwrap()
            .setting
            .title,
        "Lighting"
    );
}

#[test]
fn root_editors_and_owned_snapshots_work_without_cloning_the_domain_type() {
    let (mut world, _, _) = world();
    let entity = world.spawn(setting());
    world.tick();
    let property = world
        .get_resource::<InspectorRegistry>()
        .unwrap()
        .collect_component(&world, entity, TypeId::of::<Setting>())
        .unwrap()
        .remove(0);
    assert_eq!(property.path, PropertyPath::default());
    assert!(!world.was_component_changed(entity, TypeId::of::<Setting>()));
    let row = property.row(entity, TypeId::of::<Setting>());
    let edit = PropertyCommit::new::<Setting, SettingEditor>(&row, SettingEdit::Toggle).unwrap();
    apply_property_commit(&mut world, edit).unwrap();
    assert!(
        world
            .get_component_for_entity::<Setting>(entity)
            .unwrap()
            .enabled
    );
    assert!(
        !property
            .value
            .snapshot::<Setting, SettingEditor>()
            .unwrap()
            .enabled,
        "snapshot is independent of world mutation"
    );
}

#[test]
fn replacing_an_adapter_invalidates_old_edits_even_for_the_same_adapter_type() {
    let (mut world, a, _) = world();
    let old = property(&world, a);
    let edit = commit(&world, a, SettingEdit::Toggle);
    world.tick();
    ResMut::<InspectorRegistry>::new(world.as_unsafe_world_cell_mut())
        .register_property_editor::<Setting, SettingEditor>(SettingEditor);
    assert_ne!(old.registry_tick(), property(&world, a).registry_tick());
    assert_eq!(
        apply_property_commit(&mut world, edit),
        Err(EditError::StaleEditor)
    );
    assert!(
        !world
            .get_component_for_entity::<Container>(a)
            .unwrap()
            .setting
            .enabled
    );
    let current = commit(&world, a, SettingEdit::Toggle);
    apply_property_commit(&mut world, current).unwrap();
    assert!(
        world
            .get_component_for_entity::<Container>(a)
            .unwrap()
            .setting
            .enabled
    );
}

#[test]
fn missing_targets_and_incompatible_paths_return_errors() {
    let (mut world, a, b) = world();
    let mut wrong_type = commit(&world, a, SettingEdit::Toggle);
    wrong_type.row.path = PropertyPath::new(["gain"]);
    assert_eq!(
        apply_property_commit(&mut world, wrong_type),
        Err(EditError::TypeMismatch)
    );
    let mut missing = commit(&world, a, SettingEdit::Toggle);
    missing.row.path = PropertyPath::new(["absent"]);
    assert_eq!(
        apply_property_commit(&mut world, missing),
        Err(EditError::NotFound)
    );
    let removed = commit(&world, a, SettingEdit::Toggle);
    world.remove_component::<Container>(a);
    assert_eq!(
        apply_property_commit(&mut world, removed),
        Err(EditError::MissingTarget)
    );
    let despawned = commit(&world, b, SettingEdit::Toggle);
    world.despawn(b);
    assert_eq!(
        apply_property_commit(&mut world, despawned),
        Err(EditError::MissingTarget)
    );
}

#[test]
fn collection_does_not_stamp_changed_ticks() {
    let (world, a, _) = world();
    assert_eq!(property(&world, a).path, PropertyPath::new(["setting"]));
    assert!(!world.was_component_changed(a, TypeId::of::<Container>()));
}

#[derive(Resource)]
struct BuildRow {
    property: Property,
    target: Entity,
}

fn build_widget(data: Res<BuildRow>, mut cmd: CommandQueue) {
    let row = cmd
        .spawn((
            data.property.row(data.target, TypeId::of::<Container>()),
            data.property.value.clone(),
        ))
        .entity();
    SettingEditor.build(
        &mut cmd,
        row,
        data.property
            .value
            .snapshot::<Setting, SettingEditor>()
            .unwrap(),
        &UITheme::default(),
    );
}

#[test]
fn custom_widget_build_click_commit_and_refresh_smoke_test() {
    let (mut world, a, _) = world();
    world.insert_resource(BuildRow {
        property: property(&world, a),
        target: a,
    });
    let mut build = build_widget.into_system();
    build.initialize(&mut world);
    build.run_and_apply((), &mut world);
    let mut query = world.query::<(Entity, &SettingButton), ()>();
    let (button, row) = query
        .iter(&mut world)
        .map(|(entity, button)| (entity, button.0))
        .next()
        .unwrap();
    assert!(
        world
            .get_component_for_entity::<UIText>(button)
            .unwrap()
            .text
            .contains("Off")
    );
    world.trigger_on(
        button,
        UIClick {
            position: glam::Vec2::ZERO,
            button: MouseButton::Left,
        },
    );
    assert_eq!(world.get_resource::<PropertyCommits>().unwrap().0.len(), 1);
    apply_property_commits(&mut world);
    assert!(
        world
            .get_resource::<PropertyCommits>()
            .unwrap()
            .0
            .is_empty()
    );
    assert!(
        world
            .get_component_for_entity::<Container>(a)
            .unwrap()
            .setting
            .enabled
    );
    let refreshed = property(&world, a).value;
    *world
        .get_component_for_entity_mut::<PropertyRowValue>(row)
        .unwrap() = refreshed;
    let mut refresh = example::refresh_settings.into_system();
    refresh.initialize(&mut world);
    refresh.run_and_apply((), &mut world);
    assert!(
        world
            .get_component_for_entity::<UIText>(button)
            .unwrap()
            .text
            .contains("On")
    );
}

#[test]
fn app_registration_is_available_to_downstream_plugins() {
    let mut app = App::new();
    app.insert_resource(InspectorRegistry::default());
    example::install(&mut app);
    assert_eq!(
        app.get_resource::<InspectorRegistry>()
            .unwrap()
            .collect(&setting())
            .len(),
        1
    );
}

use std::any::TypeId;

use concerto_ecs::{Entity, IntoSystem, Res, System, entity::hierarchy::ChildOf};
use concerto_editor::{
    hosted::testing::SceneFixture,
    inspector::{InspectorRegistry, Property, PropertyKind, PropertyRow, row_systems, spawn_panel},
    tools::{edits::SetValue, selection::Select},
};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_mesh::JointName;
use concerto_motion_matching::{BonePositions, MotionDatabaseDefinition};
use concerto_reflect::{PathSegment, PropertyPath, Reflect};
use concerto_ui::{interaction::UIDisabled, node::UINode, text::UIText, theme::UITheme};
use serde_json::json;

use super::*;
use crate::{
    DatabaseSettings,
    panels::tests::{Tab, skeleton_of, tab_with},
    preview::SkeletonBones,
    read_definition,
    tests::{
        channels, component, fixture, save, set, settings_entity, write_asset, write_definition,
    },
};

fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let parent = cmd.spawn(UINode::default()).entity();
    spawn_panel(&mut cmd, parent, &theme);
}

pub(crate) fn with_inspector(f: &mut SceneFixture) {
    let mut panel = build_inspector.into_system();
    panel.initialize(&mut f.world);
    panel.run_and_apply((), &mut f.world);
    for system in row_systems() {
        f.add_delivery(system);
    }
}

fn inspecting(mut f: SceneFixture) -> Tab {
    with_inspector(&mut f);
    f.add_delivery(refresh_joint_rows.into_boxed_system());
    let mut tab = tab_with(f);
    tab.until_read();
    tab
}

fn select(tab: &mut Tab, entity: Entity) {
    tab.f.signal(Select(Some(entity)));
    for _ in 0..3 {
        tab.f.frame();
    }
}

fn select_settings(tab: &mut Tab) {
    let settings = settings_entity(&mut tab.f, tab.document);
    select(tab, settings);
}

fn label_of(tab: &mut Tab, path: &str) -> Entity {
    let world = &mut tab.f.world;
    let labels: Vec<(Entity, Entity)> = world
        .query::<(Entity, &JointNameLabel), ()>()
        .iter(world)
        .map(|(entity, label)| (entity, label.row))
        .collect();
    labels
        .into_iter()
        .find(|(_, row)| {
            world
                .get_component_for_entity::<PropertyRow>(*row)
                .is_some_and(|row| row.path.to_string() == path)
        })
        .map(|(label, _)| label)
        .expect("the inspector shows the joint's row")
}

fn text_of(tab: &Tab, label: Entity) -> &UIText {
    tab.f
        .world
        .get_component_for_entity::<UIText>(label)
        .unwrap()
}

fn button_of(tab: &Tab, label: Entity) -> Entity {
    tab.f
        .world
        .get_component_for_entity::<ChildOf>(label)
        .unwrap()
        .parent()
}

fn bones_in_preview(tab: &Tab) -> Option<Vec<String>> {
    tab.f
        .scene_world(tab.document)
        .get_resource::<SkeletonBones>()
        .map(|bones| bones.0.clone())
}

fn kinds(value: &dyn Reflect, tab: &Tab, registry: &InspectorRegistry) -> Vec<Property> {
    let types = tab
        .f
        .scene_world(tab.document)
        .get_resource::<concerto_ecs::reflect::AppTypeRegistry>()
        .unwrap()
        .read();
    registry.collect(value, &types)
}

#[test]
fn a_joint_name_has_a_row_of_its_own_on_a_field_and_in_a_list() {
    let mut tab = tab_with(fixture());
    tab.until_read();
    let settings = settings_entity(&mut tab.f, tab.document);
    let channel = channels(&mut tab.f, tab.document)[0];
    let registry = tab.f.world.get_resource::<InspectorRegistry>().unwrap();

    let settings: DatabaseSettings = component(&tab.f, tab.document, settings);
    let settings = kinds(&settings, &tab, registry);
    let channel: BonePositions = component(&tab.f, tab.document, channel);
    let channel = kinds(&channel, &tab, registry);

    for (properties, path) in [
        (&settings, "simulation.position_joint"),
        (&settings, "simulation.facing_joint"),
        (&channel, "bones.0"),
    ] {
        let property = properties
            .iter()
            .find(|property| property.path.to_string() == path)
            .unwrap();
        assert!(matches!(property.kind, PropertyKind::Value(_)), "{path}");
    }
}

#[test]
fn a_row_reads_the_stored_name() {
    let mut tab = inspecting(fixture());
    select_settings(&mut tab);

    let label = label_of(&mut tab, "simulation.facing_joint");

    assert_eq!(text_of(&tab, label).text, "Hips");
}

#[test]
fn the_row_follows_an_edit_made_elsewhere() {
    let mut tab = inspecting(fixture());
    select_settings(&mut tab);
    let settings = settings_entity(&mut tab.f, tab.document);

    set::<DatabaseSettings>(
        &mut tab.f,
        settings,
        ["simulation", "facing_joint"],
        json!("Spine"),
    );
    tab.f.frame();
    tab.f.frame();

    let label = label_of(&mut tab, "simulation.facing_joint");
    assert_eq!(text_of(&tab, label).text, "Spine");
}

#[test]
fn the_menu_lists_the_skeletons_bones() {
    let mut tab = inspecting(fixture());
    select_settings(&mut tab);
    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);

    tab.click(button);

    assert!(tab.name_menu_open());
    assert_eq!(tab.menu_names(), ["Hips", "Spine", "Head"]);

    tab.click(button);

    assert!(!tab.name_menu_open());
}

#[test]
fn picking_a_bone_writes_it_marks_the_document_unsaved_and_saves() {
    let mut tab = inspecting(fixture());
    select_settings(&mut tab);
    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);

    tab.click(button);
    let item = tab.name_item("Head");
    tab.click(item);
    tab.f.frame();
    tab.f.frame();

    assert!(!tab.name_menu_open());
    let settings = settings_entity(&mut tab.f, tab.document);
    let shown: DatabaseSettings = component(&tab.f, tab.document, settings);
    assert_eq!(shown.simulation.facing_joint, "Head".into());
    assert!(tab.f.unsaved(tab.document));
    assert_eq!(text_of(&tab, label).text, "Head");

    save(&mut tab.f, tab.document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.simulation.facing_joint, "Head".into());
}

#[test]
fn picking_a_bone_for_a_list_element_writes_that_element() {
    let mut tab = inspecting(fixture());
    let channel = channels(&mut tab.f, tab.document)[0];
    select(&mut tab, channel);
    let label = label_of(&mut tab, "bones.1");
    let button = button_of(&tab, label);

    tab.click(button);
    let item = tab.name_item("Head");
    tab.click(item);
    tab.f.frame();
    tab.f.frame();

    let bones = component::<BonePositions>(&tab.f, tab.document, channel).bones;
    assert_eq!(bones[1], JointName::from("Head"));
    assert_eq!(bones[0], JointName::from("LeftFoot"));
}

#[test]
fn a_name_the_skeleton_lacks_shows_in_the_warning_colour() {
    let mut tab = inspecting(fixture());
    select_settings(&mut tab);
    let theme = UITheme::default();

    let lacking = label_of(&mut tab, "simulation.position_joint");
    let known = label_of(&mut tab, "simulation.facing_joint");

    assert_eq!(text_of(&tab, lacking).text, "Spine2");
    assert_eq!(text_of(&tab, lacking).color, theme.warning);
    assert_eq!(text_of(&tab, known).color, theme.text);
}

#[test]
fn with_no_skeleton_the_row_is_a_disabled_button_reading_the_stored_name() {
    let mut f = fixture();
    let asset = write_definition(
        &mut f,
        "content/plain.gasset",
        &MotionDatabaseDefinition::default(),
    );
    let mut tab = inspecting(f);
    crate::tests::open(&mut tab.f, &asset);
    tab.f.frame();
    select_settings(&mut tab);

    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);

    assert_eq!(bones_in_preview(&tab), None);
    assert_eq!(text_of(&tab, label).text, "Hips");
    assert_eq!(text_of(&tab, label).color, UITheme::default().text_muted);
    assert!(
        tab.f
            .world
            .get_component_for_entity::<UIDisabled>(button)
            .is_some()
    );

    tab.click(button);
    assert!(!tab.name_menu_open());
}

#[test]
fn an_empty_name_reads_as_none() {
    let mut tab = inspecting(fixture());
    let channel = channels(&mut tab.f, tab.document)[0];
    tab.f.signal(SetValue {
        entity: channel,
        component: TypeId::of::<BonePositions>(),
        path: PropertyPath::new([PathSegment::Field("bones"), PathSegment::Index(0)]),
        value: json!(""),
    });
    select(&mut tab, channel);

    let label = label_of(&mut tab, "bones.0");

    assert_eq!(text_of(&tab, label).text, "None");
}

#[test]
fn the_bones_of_a_picked_skeleton_reach_the_preview() {
    let mut f = fixture();
    let rig = write_asset(
        &mut f,
        "content/rig.gasset",
        &skeleton_of(&["Root", "Tail"]),
    );
    let mut tab = inspecting(f);
    assert_eq!(bones_in_preview(&tab).unwrap(), ["Hips", "Spine", "Head"]);
    tab.f
        .world
        .get_component_for_entity_mut::<crate::DefinitionDocument>(tab.document)
        .unwrap()
        .skeleton = AssetHandle::weak(rig.id);
    tab.f.frame();
    tab.f.frame();

    assert_eq!(bones_in_preview(&tab).unwrap(), ["Root", "Tail"]);
}

#[test]
fn a_skeleton_that_cannot_be_read_leaves_the_preview_with_no_bones() {
    let mut tab = inspecting(fixture());
    assert!(bones_in_preview(&tab).is_some());
    std::fs::write(
        tab.f.project_root.join(&tab.skeleton.address),
        b"not a skeleton",
    )
    .unwrap();
    let other = write_asset(&mut tab.f, "content/rig.gasset", &skeleton_of(&["Root"]));
    std::fs::write(tab.f.project_root.join(&other.address), b"not a skeleton").unwrap();
    tab.f
        .world
        .get_component_for_entity_mut::<crate::DefinitionDocument>(tab.document)
        .unwrap()
        .skeleton = AssetHandle::weak(other.id);
    tab.f.frame();
    tab.f.frame();

    assert_eq!(bones_in_preview(&tab), None);
}

#[test]
fn the_menu_closes_when_the_tab_stops_being_the_edit_target() {
    let mut tab = inspecting(fixture());
    select_settings(&mut tab);
    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);
    tab.click(button);
    assert!(tab.name_menu_open());

    tab.f.open();

    assert!(!tab.name_menu_open());
}

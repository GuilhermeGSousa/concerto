use concerto_ecs::{Entity, entity::hierarchy::ChildOf};
use concerto_editor::{
    hosted::EditTarget,
    inspector::{PropertyCommits, PropertyRow},
};
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_mesh::JointName;
use concerto_motion_matching::{BonePositions, MotionDatabase};
use concerto_ui::{interaction::UIDisabled, text::UIText, theme::UITheme};

use super::*;
use crate::{
    DefinitionDocument,
    panels::tests::{Tab, skeleton_of, tab_with},
    read_definition,
    tests::{channel_at, fixture, row_of, save, show, state, write_asset, write_definition},
};

fn inspecting() -> Tab {
    let mut tab = tab_with(fixture());
    tab.until_read();
    tab
}

fn show_settings(tab: &mut Tab) {
    let document = tab.document;
    show(&mut tab.f, document, "Settings");
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
                .is_some_and(|row| row.address.path.to_string() == path)
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

fn write_joint(tab: &mut Tab, name: &str, path: &str, joint: &str) {
    let document = tab.document;
    let row = row_of(&mut tab.f, document, name, path);
    let commits = tab.f.world.get_resource_mut::<PropertyCommits>().unwrap();
    commits
        .push::<JointName, JointNameEditor>(&row, joint.into())
        .unwrap();
    tab.f.frame();
    tab.f.frame();
}

fn pick_skeleton(tab: &mut Tab, id: AssetId) {
    tab.f
        .world
        .get_component_for_entity_mut::<DefinitionDocument>(tab.document)
        .unwrap()
        .skeleton = AssetHandle::weak(id);
    tab.f.frame();
    tab.f.frame();
}

#[test]
fn a_joint_name_has_a_row_of_its_own_on_a_field_and_in_a_list() {
    let mut tab = inspecting();
    show_settings(&mut tab);

    label_of(&mut tab, "simulation.position_joint");
    label_of(&mut tab, "simulation.facing_joint");

    let document = tab.document;
    show(&mut tab.f, document, "BonePositions");

    label_of(&mut tab, "bones.0");
}

#[test]
fn a_row_reads_the_stored_name() {
    let mut tab = inspecting();
    show_settings(&mut tab);

    let label = label_of(&mut tab, "simulation.facing_joint");

    assert_eq!(text_of(&tab, label).text, "Hips");
}

#[test]
fn the_row_follows_an_edit_made_elsewhere() {
    let mut tab = inspecting();

    write_joint(&mut tab, "Settings", "simulation.facing_joint", "Spine");

    let label = label_of(&mut tab, "simulation.facing_joint");
    assert_eq!(text_of(&tab, label).text, "Spine");
}

#[test]
fn the_menu_lists_the_skeletons_bones_with_no_edit_target() {
    let mut tab = inspecting();
    show_settings(&mut tab);
    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);
    assert!(
        tab.f
            .world
            .query::<&EditTarget, ()>()
            .iter(&mut tab.f.world)
            .next()
            .is_none()
    );

    tab.click(button);

    assert!(tab.name_menu_open());
    assert_eq!(tab.menu_names(), ["Hips", "Spine", "Head"]);

    tab.click(button);

    assert!(!tab.name_menu_open());
}

#[test]
fn picking_a_bone_writes_it_marks_the_document_unsaved_and_saves() {
    let mut tab = inspecting();
    show_settings(&mut tab);
    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);

    tab.click(button);
    let item = tab.name_item("Head");
    tab.click(item);
    tab.f.frame();
    tab.f.frame();

    assert!(!tab.name_menu_open());
    let settings = tab.state().settings.as_ref().unwrap();
    assert_eq!(settings.simulation.facing_joint, "Head".into());
    assert!(tab.f.unsaved(tab.document));
    assert_eq!(text_of(&tab, label).text, "Head");

    save(&mut tab.f, tab.document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.simulation.facing_joint, "Head".into());
}

#[test]
fn picking_the_bone_already_set_leaves_the_document_saved() {
    let mut tab = inspecting();
    show_settings(&mut tab);
    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);
    let current = text_of(&tab, label).text.clone();

    tab.click(button);
    let item = tab.name_item(&current);
    tab.click(item);
    tab.f.frame();
    tab.f.frame();

    assert!(!tab.name_menu_open());
    assert!(!tab.f.unsaved(tab.document));
}

#[test]
fn picking_a_bone_for_a_list_element_writes_that_element() {
    let mut tab = inspecting();
    let document = tab.document;
    show(&mut tab.f, document, "BonePositions");
    let label = label_of(&mut tab, "bones.1");
    let button = button_of(&tab, label);

    tab.click(button);
    let item = tab.name_item("Head");
    tab.click(item);
    tab.f.frame();
    tab.f.frame();

    let bones = &channel_at::<BonePositions>(&tab.f, document, 0).bones;
    assert_eq!(bones[1], JointName::from("Head"));
    assert_eq!(bones[0], JointName::from("LeftFoot"));
}

#[test]
fn a_name_the_skeleton_lacks_shows_in_the_warning_colour() {
    let mut tab = inspecting();
    show_settings(&mut tab);
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
    let asset = write_definition(&mut f, "content/plain.gasset", &MotionDatabase::default());
    let mut tab = tab_with(f);
    tab.until_read();
    crate::tests::open(&mut tab.f, &asset);
    tab.f.frame();
    show_settings(&mut tab);

    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);

    assert_eq!(state(&tab.f, tab.document).bones, None);
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
    let mut tab = inspecting();

    write_joint(&mut tab, "BonePositions", "bones.0", "");

    let label = label_of(&mut tab, "bones.0");
    assert_eq!(text_of(&tab, label).text, "None");
}

#[test]
fn a_picked_skeleton_gives_the_document_its_bones() {
    let mut f = fixture();
    let rig = write_asset(
        &mut f,
        "content/rig.gasset",
        &skeleton_of(&["Root", "Tail"]),
    );
    let mut tab = tab_with(f);
    tab.until_read();
    assert_eq!(
        tab.state().bones.as_deref().unwrap(),
        ["Hips", "Spine", "Head"]
    );

    pick_skeleton(&mut tab, rig.id);

    assert_eq!(tab.state().bones.as_deref().unwrap(), ["Root", "Tail"]);
}

#[test]
fn a_skeleton_that_cannot_be_read_leaves_the_document_with_no_bones() {
    let mut tab = inspecting();
    assert!(tab.state().bones.is_some());
    let other = write_asset(&mut tab.f, "content/rig.gasset", &skeleton_of(&["Root"]));
    std::fs::write(tab.f.project_root.join(&other.address), b"not a skeleton").unwrap();

    pick_skeleton(&mut tab, other.id);

    assert_eq!(tab.state().bones, None);
}

#[test]
fn the_menu_closes_when_another_tab_comes_to_the_front() {
    let mut tab = inspecting();
    show_settings(&mut tab);
    let label = label_of(&mut tab, "simulation.facing_joint");
    let button = button_of(&tab, label);
    tab.click(button);
    assert!(tab.name_menu_open());

    tab.f.open();

    assert!(!tab.name_menu_open());
}

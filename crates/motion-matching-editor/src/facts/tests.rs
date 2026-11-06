use concerto_editor::{
    inspector::PropertyCommits,
    joint_picker::{DocumentBones, JointNameEditor},
};
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_mesh::JointName;
use concerto_motion_matching::MotionDatabase;

use crate::{
    panels::tests::{Tab, skeleton_of, tab_with},
    read_definition,
    tests::{fixture, open, row_of, save, write_asset, write_definition},
};

fn reading() -> Tab {
    let mut tab = tab_with(fixture());
    tab.until_read();
    tab
}

fn bones_of(tab: &Tab) -> Option<Vec<String>> {
    tab.f
        .world
        .get_component_for_entity::<DocumentBones>(tab.document)
        .expect("the definition's document carries its bones")
        .0
        .as_ref()
        .map(|bones| bones.iter().map(|bone| bone.as_str().to_owned()).collect())
}

fn pick_skeleton(tab: &mut Tab, id: AssetId) {
    tab.state_mut().skeleton = AssetHandle::weak(id);
    tab.f.frame();
    tab.f.frame();
}

#[test]
fn a_definition_gives_its_document_the_bones_of_its_skeleton() {
    let tab = reading();

    assert_eq!(bones_of(&tab).unwrap(), ["Hips", "Spine", "Head"]);
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

    pick_skeleton(&mut tab, rig.id);

    assert_eq!(bones_of(&tab).unwrap(), ["Root", "Tail"]);
}

#[test]
fn a_skeleton_that_cannot_be_read_leaves_the_document_with_no_bones() {
    let mut tab = reading();
    let other = write_asset(&mut tab.f, "content/rig.gasset", &skeleton_of(&["Root"]));
    std::fs::write(tab.f.project_root.join(&other.address), b"not a skeleton").unwrap();

    pick_skeleton(&mut tab, other.id);

    assert_eq!(bones_of(&tab), None);
}

#[test]
fn a_definition_with_no_skeleton_has_no_bones() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/plain.gasset", &MotionDatabase::default());
    let mut tab = tab_with(f);
    tab.until_read();

    tab.document = open(&mut tab.f, &asset);
    tab.f.frame();

    assert_eq!(bones_of(&tab), None);
}

#[test]
fn a_joint_picked_for_the_settings_is_written_and_saved() {
    let mut tab = reading();
    let document = tab.document;
    let row = row_of(&mut tab.f, document, "Settings", "simulation.facing_joint");

    tab.f
        .world
        .get_resource_mut::<PropertyCommits>()
        .unwrap()
        .push::<JointName, JointNameEditor>(&row, "Head".into())
        .unwrap();
    tab.f.frame();
    tab.f.frame();

    let settings = tab.state().settings.as_ref().unwrap();
    assert_eq!(settings.simulation.facing_joint, "Head".into());
    assert!(tab.f.unsaved(document));

    save(&mut tab.f, document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.simulation.facing_joint, "Head".into());
}

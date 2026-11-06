use concerto_ecs::{
    Entity, IntoSystem, With, World,
    entity::hierarchy::{ChildOf, Children},
};
use concerto_foundation::assets::{content::decode_content_asset, handle::AssetHandle};
use concerto_mesh::JointName;
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::{UIClick, UIDisabled},
    text::UIText,
    theme::UITheme,
};
use concerto_window::input::MouseButton;

use super::*;
use crate::{
    hosted::{EditTarget, testing::SceneFixture},
    inspected_asset::{
        InspectedDocument,
        tests::{
            OpenRig, RIG_ADDRESS, Rig, open_rig, press_save, request_asset, rig_fixture, rig_on,
            skeleton_of, write_asset,
        },
    },
    inspector::{PropertyCommits, PropertyRow},
    name_menu::{NameMenu, close_name_menu},
};

struct Picker {
    f: SceneFixture,
    document: Entity,
}

fn picking(mut rig: OpenRig) -> Picker {
    rig.f.add_delivery(close_name_menu.into_boxed_system());
    rig.f.add_delivery(refresh_joint_rows.into_boxed_system());
    rig.f.frame();
    Picker {
        f: rig.f,
        document: rig.document,
    }
}

fn picker() -> Picker {
    picking(open_rig())
}

impl Picker {
    fn row(&mut self, path: &str) -> Option<PropertyRow> {
        let world = &mut self.f.world;
        world
            .query::<&PropertyRow, ()>()
            .iter(world)
            .find(|row| {
                row.address.target.name() == "TestRig" && row.address.path.to_string() == path
            })
            .cloned()
    }

    fn label(&mut self, path: &str) -> Entity {
        self.f
            .frames_until("the inspector never showed the joint's row", |f| {
                find_label(&mut f.world, path).is_some()
            });
        find_label(&mut self.f.world, path).unwrap()
    }

    fn text(&self, label: Entity) -> &UIText {
        self.f
            .world
            .get_component_for_entity::<UIText>(label)
            .unwrap()
    }

    fn button(&self, label: Entity) -> Entity {
        self.parent(label)
    }

    fn parent(&self, entity: Entity) -> Entity {
        self.f
            .world
            .get_component_for_entity::<ChildOf>(entity)
            .unwrap()
            .parent()
    }

    fn click(&mut self, entity: Entity) {
        self.f.world.trigger_on(
            entity,
            UIClick {
                position: Default::default(),
                button: MouseButton::Left,
            },
        );
    }

    fn menu_labels(&mut self) -> Vec<MenuLabel> {
        let world = &mut self.f.world;
        let menus: Vec<Entity> = world
            .query::<Entity, With<NameMenu>>()
            .iter(world)
            .collect();
        let mut labels = Vec::new();
        let mut pending = menus;
        while let Some(entity) = pending.pop() {
            if let Some(text) = self.f.world.get_component_for_entity::<UIText>(entity) {
                labels.push(MenuLabel {
                    entity,
                    name: text.text.clone(),
                });
            }
            if let Some(children) = self.f.world.get_component_for_entity::<Children>(entity) {
                pending.extend(children.iter().rev());
            }
        }
        labels
    }

    fn menu_names(&mut self) -> Vec<String> {
        self.menu_labels()
            .into_iter()
            .map(|label| label.name)
            .collect()
    }

    fn name_item(&mut self, name: &str) -> Entity {
        let label = self
            .menu_labels()
            .into_iter()
            .find(|label| label.name == name)
            .expect("the menu lists the name")
            .entity;
        self.parent(label)
    }

    fn name_menu_open(&mut self) -> bool {
        self.f
            .world
            .query::<&UIAnchoredPanel, With<NameMenu>>()
            .iter(&mut self.f.world)
            .any(|panel| panel.open)
    }

    fn write_joint(&mut self, path: &str, joint: &str) {
        self.f
            .frames_until("the inspector never showed the row", |f| {
                let world = &mut f.world;
                world
                    .query::<&PropertyRow, ()>()
                    .iter(world)
                    .any(|row| row.address.path.to_string() == path)
            });
        let row = self.row(path).unwrap();
        let commits = self.f.world.get_resource_mut::<PropertyCommits>().unwrap();
        commits
            .push::<JointName, JointNameEditor>(&row, joint.into())
            .unwrap();
        self.f.frame();
        self.f.frame();
    }

    fn rig(&self) -> &Rig {
        self.f
            .world
            .get_component_for_entity::<InspectedDocument>(self.document)
            .unwrap()
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<Rig>())
            .expect("the document holds a rig")
    }
}

fn find_label(world: &mut World, path: &str) -> Option<Entity> {
    let labels: Vec<JointLabelAt> = world
        .query::<(Entity, &JointNameLabel), ()>()
        .iter(world)
        .map(|(entity, label)| JointLabelAt {
            label: entity,
            row: label.row,
        })
        .collect();
    labels
        .into_iter()
        .find(|at| {
            world
                .get_component_for_entity::<PropertyRow>(at.row)
                .is_some_and(|row| row.address.path.to_string() == path)
        })
        .map(|at| at.label)
}

struct JointLabelAt {
    label: Entity,
    row: Entity,
}

struct MenuLabel {
    entity: Entity,
    name: String,
}

#[test]
fn a_joint_name_has_a_row_of_its_own_on_a_field_and_in_a_list() {
    let mut picker = picker();

    picker.label("joint");
    picker.label("joints.0");
    picker.label("joints.1");
}

#[test]
fn a_row_reads_the_stored_name() {
    let mut picker = picker();

    let label = picker.label("joint");

    assert_eq!(picker.text(label).text, "Hips");
}

#[test]
fn the_row_follows_an_edit_made_elsewhere() {
    let mut picker = picker();

    picker.write_joint("joint", "Spine");

    let label = picker.label("joint");
    assert_eq!(picker.text(label).text, "Spine");
}

#[test]
fn the_menu_lists_the_skeletons_bones_with_no_edit_target() {
    let mut picker = picker();
    let label = picker.label("joint");
    let button = picker.button(label);
    assert!(
        picker
            .f
            .world
            .query::<&EditTarget, ()>()
            .iter(&mut picker.f.world)
            .next()
            .is_none()
    );

    picker.click(button);

    assert!(picker.name_menu_open());
    assert_eq!(picker.menu_names(), ["Hips", "Spine", "Head"]);

    picker.click(button);

    assert!(!picker.name_menu_open());
}

#[test]
fn picking_a_bone_writes_it_marks_the_document_unsaved_and_saves() {
    let mut picker = picker();
    let label = picker.label("joint");
    let button = picker.button(label);

    picker.click(button);
    let item = picker.name_item("Head");
    picker.click(item);
    picker.f.frame();
    picker.f.frame();

    assert!(!picker.name_menu_open());
    assert_eq!(picker.rig().joint, "Head".into());
    assert!(picker.f.unsaved(picker.document));
    assert_eq!(picker.text(label).text, "Head");

    press_save(&mut picker.f);

    let bytes = std::fs::read(picker.f.project_root.join(RIG_ADDRESS)).unwrap();
    let saved = decode_content_asset::<Rig>(&bytes).unwrap().1;
    assert_eq!(saved.joint, "Head".into());
}

#[test]
fn picking_the_bone_already_set_leaves_the_document_saved() {
    let mut picker = picker();
    let label = picker.label("joint");
    let button = picker.button(label);
    let current = picker.text(label).text.clone();

    picker.click(button);
    let item = picker.name_item(&current);
    picker.click(item);
    picker.f.frame();
    picker.f.frame();

    assert!(!picker.name_menu_open());
    assert!(!picker.f.unsaved(picker.document));
}

#[test]
fn picking_a_bone_for_a_list_element_writes_that_element() {
    let mut picker = picker();
    let label = picker.label("joints.1");
    let button = picker.button(label);

    picker.click(button);
    let item = picker.name_item("Head");
    picker.click(item);
    picker.f.frame();
    picker.f.frame();

    let joints = &picker.rig().joints;
    assert_eq!(joints[1], JointName::from("Head"));
    assert_eq!(joints[0], JointName::from("LeftFoot"));
}

#[test]
fn a_name_the_skeleton_lacks_shows_in_the_warning_colour() {
    let mut picker = picker();
    let theme = UITheme::default();

    let lacking = picker.label("joints.0");
    let known = picker.label("joint");

    assert_eq!(picker.text(lacking).text, "LeftFoot");
    assert_eq!(picker.text(lacking).color, theme.warning);
    assert_eq!(picker.text(known).color, theme.text);
}

#[test]
fn with_no_skeleton_the_row_is_a_disabled_button_reading_the_stored_name() {
    let mut f = rig_fixture();
    let skeleton = write_asset(
        &mut f,
        "content/rigs/skeleton.gasset",
        &skeleton_of(&["Hips"]),
    );
    let rig = Rig {
        skeleton: AssetHandle::default(),
        ..rig_on(&skeleton)
    };
    let asset = write_asset(&mut f, RIG_ADDRESS, &rig);
    let document = request_asset(&mut f, &asset);
    let mut picker = picking(OpenRig {
        f,
        document,
        skeleton,
    });

    let label = picker.label("joint");
    let button = picker.button(label);

    assert_eq!(
        picker
            .f
            .world
            .get_component_for_entity::<DocumentBones>(document)
            .unwrap(),
        &DocumentBones(None)
    );
    assert_eq!(picker.text(label).text, "Hips");
    assert_eq!(picker.text(label).color, UITheme::default().text_muted);
    assert!(
        picker
            .f
            .world
            .get_component_for_entity::<UIDisabled>(button)
            .is_some()
    );

    picker.click(button);
    assert!(!picker.name_menu_open());
}

#[test]
fn an_empty_name_reads_as_none() {
    let mut picker = picker();

    picker.write_joint("joints.0", "");

    let label = picker.label("joints.0");
    assert_eq!(picker.text(label).text, "None");
}

#[test]
fn the_menu_closes_when_another_tab_comes_to_the_front() {
    let mut picker = picker();
    let label = picker.label("joint");
    let button = picker.button(label);
    picker.click(button);
    assert!(picker.name_menu_open());

    picker.f.open();

    assert!(!picker.name_menu_open());
}

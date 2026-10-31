use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_ecs::{
    Changed, Entity, IntoSystem, Query, ResMut, Resource, With, World,
    entity::hierarchy::{ChildOf, Children},
};
use concerto_editor::{
    asset_editor::{EditorDocument, EditorHosts},
    asset_picker::AssetPickerMenu,
    clip_editor::ClipFacts,
    hosted::testing::SceneFixture,
    timeline::{Timeline, TimelineSelected, time_of_frame},
    tools::selection::{Select, Selection},
};
use concerto_foundation::assets::{
    AssetId,
    content::save_content_asset_into,
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mcp::FrameDemand;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::{
    BonePositions, BoneVelocities, MotionDatabaseDefinition, MotionEntry, TimeRange,
};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::{UIClick, UIDisabled, UIInteractionStyle},
    node::{UIBox, UILayout, UINode},
    scroll::UIScrollArea,
    text::UIText,
    theme::{ButtonVariant, UITheme},
};
use concerto_window::input::MouseButton;
use glam::{Mat4, Vec2};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::{
    ChannelKind, ChannelOrder, DefinitionDocument, EntryFacts,
    menu::NameMenu,
    read_definition,
    tests::{
        broken_recipe, channels, component, definition, fixture, fixture_with, open, request, save,
        set, settings_entity, state, write_asset, write_definition,
    },
};

pub(crate) struct Tab {
    pub(crate) f: SceneFixture,
    pub(crate) document: Entity,
    pub(crate) walk: AssetEntry,
    pub(crate) run: AssetEntry,
    pub(crate) skeleton: AssetEntry,
    pub(crate) asset: AssetEntry,
}

fn bone_id(bone: usize) -> Uuid {
    Uuid::from_u128(bone as u128 + 1)
}

pub(crate) fn skeleton_of(names: &[&str]) -> Skeleton {
    let bones = names.len();
    Skeleton::new(
        vec![Mat4::IDENTITY; bones],
        (0..bones)
            .map(|bone| bone.checked_sub(1).map(|parent| parent as u16))
            .collect(),
        names.iter().map(ToString::to_string).collect(),
        vec![JointPose::IDENTITY; bones],
        (0..bones).map(bone_id).collect(),
    )
    .unwrap()
}

fn keys(seconds: usize) -> AnimationChannel {
    let times: Vec<f32> = (0..=seconds * 30).map(|key| key as f32 / 30.0).collect();
    let translations = vec![[0.0; 3]; times.len()];
    AnimationChannel::new(
        times,
        AnimationChanelOutput::from_translation(translations.into_iter()),
    )
}

pub(crate) fn clip_of(seconds: usize, bones: impl IntoIterator<Item = usize>) -> AnimationClip {
    let mut clip = AnimationClip::default();
    for bone in bones {
        clip.add_channel(bone_id(bone), keys(seconds));
    }
    clip
}

fn entry_of(clip: &AssetEntry, range: Option<TimeRange>) -> MotionEntry {
    MotionEntry {
        clip: AssetHandle::weak(clip.id),
        range,
    }
}

fn hosts(f: &mut SceneFixture, document: Entity) {
    let [left, inspector, bottom] = [(); 3].map(|_| f.world.spawn(UINode::default()));
    f.world.insert(
        EditorHosts {
            canvas: None,
            left: Some(left),
            inspector: Some(inspector),
            bottom: Some(bottom),
        },
        document,
    );
    f.frame();
}

pub(crate) fn tab_with(mut f: SceneFixture) -> Tab {
    let skeleton = write_asset(
        &mut f,
        "content/hero/skeleton.gasset",
        &skeleton_of(&["Hips", "Spine", "Head"]),
    );
    let walk = write_asset(&mut f, "content/hero/walk.gasset", &clip_of(10, 0..3));
    let run = write_asset(&mut f, "content/hero/run.gasset", &clip_of(2, 0..2));
    let opened = MotionDatabaseDefinition {
        skeleton: AssetHandle::weak(skeleton.id),
        entries: vec![
            entry_of(
                &walk,
                Some(TimeRange {
                    start: 1.0,
                    end: 2.5,
                }),
            ),
            entry_of(&run, None),
        ],
        ..Default::default()
    };
    let asset = write_definition(&mut f, "content/hero/database.gasset", &opened);
    let document = open(&mut f, &asset);
    hosts(&mut f, document);
    Tab {
        f,
        document,
        walk,
        run,
        skeleton,
        asset,
    }
}

pub(crate) fn tab() -> Tab {
    let mut tab = tab_with(fixture());
    tab.until_read();
    tab
}

impl Tab {
    pub(crate) fn panels(&self) -> &DefinitionPanels {
        self.f
            .world
            .get_component_for_entity::<DefinitionPanels>(self.document)
            .expect("the panels were built")
    }

    pub(crate) fn state(&self) -> &DefinitionDocument {
        state(&self.f, self.document)
    }

    fn state_mut(&mut self) -> &mut DefinitionDocument {
        self.f
            .world
            .get_component_for_entity_mut::<DefinitionDocument>(self.document)
            .unwrap()
    }

    fn world(&self) -> &World {
        &self.f.world
    }

    fn text(&self, entity: Entity) -> &UIText {
        self.world()
            .get_component_for_entity::<UIText>(entity)
            .unwrap()
    }

    fn timeline(&self) -> &Timeline {
        self.world()
            .get_component_for_entity::<Timeline>(self.panels().timeline)
            .unwrap()
    }

    fn shown(&self, entity: Entity) -> bool {
        self.world()
            .get_component_for_entity::<UINode>(entity)
            .unwrap()
            .visible
    }

    fn disabled(&self, entity: Entity) -> bool {
        self.world()
            .get_component_for_entity::<UIDisabled>(entity)
            .is_some()
    }

    fn selected(&self, entity: Entity) -> bool {
        self.world()
            .get_component_for_entity::<UIInteractionStyle>(entity)
            .unwrap()
            .normal
            == UITheme::default()
                .interaction(ButtonVariant::Ghost, true)
                .normal
    }

    pub(crate) fn click(&mut self, entity: Entity) {
        self.f.world.trigger_on(
            entity,
            UIClick {
                position: Default::default(),
                button: MouseButton::Left,
            },
        );
    }

    pub(crate) fn clicked(&mut self, entity: Entity) {
        self.click(entity);
        self.f.frame();
        self.f.frame();
    }

    fn parent(&self, entity: Entity) -> Entity {
        self.world()
            .get_component_for_entity::<ChildOf>(entity)
            .unwrap()
            .parent()
    }

    fn labels_under(&mut self, menus: Vec<Entity>) -> Vec<(Entity, String)> {
        let world = &mut self.f.world;
        let labels: Vec<(Entity, String)> = world
            .query::<(Entity, &UIText), ()>()
            .iter(world)
            .map(|(entity, text)| (entity, text.text.clone()))
            .collect();
        labels
            .into_iter()
            .filter(|(label, _)| {
                let mut ancestor = Some(*label);
                while let Some(entity) = ancestor {
                    if menus.contains(&entity) {
                        return true;
                    }
                    ancestor = self
                        .f
                        .world
                        .get_component_for_entity::<ChildOf>(entity)
                        .map(|parent| parent.parent());
                }
                false
            })
            .collect()
    }

    pub(crate) fn menu_labels(&mut self) -> Vec<(Entity, String)> {
        let world = &mut self.f.world;
        let menus = world
            .query::<(Entity, &UIAnchoredPanel), With<NameMenu>>()
            .iter(world)
            .map(|(entity, _)| entity)
            .collect();
        self.labels_under(menus)
    }

    pub(crate) fn menu_names(&mut self) -> Vec<String> {
        self.menu_labels()
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    }

    pub(crate) fn name_item(&mut self, name: &str) -> Entity {
        let label = self
            .menu_labels()
            .into_iter()
            .find(|(_, text)| text == name)
            .map(|(entity, _)| entity)
            .expect("the menu lists the name");
        self.parent(label)
    }

    pub(crate) fn name_menu_open(&mut self) -> bool {
        self.f
            .world
            .query::<&UIAnchoredPanel, With<NameMenu>>()
            .iter(&mut self.f.world)
            .any(|panel| panel.open)
    }

    fn asset_item(&mut self, name: &str) -> Entity {
        let world = &mut self.f.world;
        let menus = world
            .query::<(Entity, &UIAnchoredPanel), With<AssetPickerMenu>>()
            .iter(world)
            .map(|(entity, _)| entity)
            .collect();
        let label = self
            .labels_under(menus)
            .into_iter()
            .find(|(_, text)| text == name)
            .map(|(entity, _)| entity)
            .expect("the menu lists the asset");
        self.parent(label)
    }

    fn asset_menu_open(&mut self) -> bool {
        self.f
            .world
            .query::<&UIAnchoredPanel, With<AssetPickerMenu>>()
            .iter(&mut self.f.world)
            .any(|panel| panel.open)
    }

    fn select_frames(&mut self, range: Option<(u32, u32)>) {
        let timeline = self.panels().timeline;
        self.f
            .world
            .trigger_on(timeline, TimelineSelected { range });
        self.f.frame();
    }

    fn unsaved(&mut self) -> bool {
        self.f.unsaved(self.document)
    }

    pub(crate) fn until_read(&mut self) {
        let document = self.document;
        self.f.frames_until("the entries were never read", |f| {
            let state = state(f, document);
            state.facts_job.is_none()
                && state
                    .entries
                    .iter()
                    .all(|entry| state.facts_of(entry.clip.id()).is_some())
        });
        self.f.frame();
        self.f.frame();
    }

    fn channel_names(&self) -> Vec<String> {
        self.panels()
            .channel_rows
            .iter()
            .map(|row| row.name.clone())
            .collect()
    }

    fn channel_row(&self, index: usize) -> &ChannelWidgets {
        &self.panels().channel_rows[index]
    }

    fn channel_remove(&self, index: usize) -> Entity {
        let row = self.parent(self.channel_row(index).row);
        self.world()
            .get_component_for_entity::<Children>(row)
            .unwrap()
            .iter()
            .copied()
            .last()
            .unwrap()
    }

    fn selection(&self) -> Option<Entity> {
        self.f
            .scene_world(self.document)
            .get_resource::<Selection>()
            .unwrap()
            .entity()
    }

    fn set_status(&mut self, status: &str) {
        let document = self.document;
        self.f
            .world
            .get_component_for_entity_mut::<EditorDocument>(document)
            .unwrap()
            .status = status.into();
    }
}

#[test]
fn the_skeleton_picker_names_the_definitions_skeleton() {
    let tab = tab();

    assert_eq!(tab.text(tab.panels().skeleton).text, "skeleton");
    assert!(!tab.shown(tab.panels().skeleton_error));
}

#[test]
fn a_definition_with_no_skeleton_reads_none() {
    let mut f = fixture();
    let asset = write_definition(
        &mut f,
        "content/database.gasset",
        &MotionDatabaseDefinition::default(),
    );
    let document = open(&mut f, &asset);
    hosts(&mut f, document);
    f.frame();

    let panels = f
        .world
        .get_component_for_entity::<DefinitionPanels>(document)
        .unwrap();
    assert_eq!(
        f.world
            .get_component_for_entity::<UIText>(panels.skeleton)
            .unwrap()
            .text,
        "None"
    );
}

#[test]
fn picking_a_skeleton_changes_the_document_and_marks_it_unsaved() {
    let mut f = fixture();
    let other = write_asset(
        &mut f,
        "content/other/rig.gasset",
        &skeleton_of(&["Hips", "Tail"]),
    );
    let mut tab = tab_with(f);
    tab.until_read();
    let button = tab.panels().skeleton_button;

    tab.click(button);
    assert!(tab.asset_menu_open());
    let item = tab.asset_item("rig");
    tab.click(item);
    tab.f.frame();

    assert!(!tab.asset_menu_open());
    assert_eq!(tab.state().skeleton.id(), other.id);
    assert!(tab.unsaved());
    assert_eq!(tab.text(tab.panels().skeleton).text, "rig");
}

#[test]
fn a_saved_skeleton_pick_is_in_the_file() {
    let mut f = fixture();
    let other = write_asset(&mut f, "content/other/rig.gasset", &skeleton_of(&["Hips"]));
    let mut tab = tab_with(f);
    let button = tab.panels().skeleton_button;
    tab.click(button);
    let item = tab.asset_item("rig");
    tab.click(item);

    save(&mut tab.f, tab.document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.skeleton.id(), other.id);
    assert!(!tab.unsaved());
}

#[test]
fn a_skeleton_that_cannot_be_read_shows_under_the_picker() {
    let mut f = fixture();
    let skeleton = write_asset(&mut f, "content/rig.gasset", &skeleton_of(&["Hips"]));
    std::fs::write(f.project_root.join(&skeleton.address), b"not a skeleton").unwrap();
    let asset = write_definition(
        &mut f,
        "content/database.gasset",
        &MotionDatabaseDefinition {
            skeleton: AssetHandle::weak(skeleton.id),
            ..Default::default()
        },
    );
    let document = open(&mut f, &asset);
    hosts(&mut f, document);
    f.frame();

    let error = f
        .world
        .get_component_for_entity::<DefinitionPanels>(document)
        .unwrap()
        .skeleton_error;
    let line = f.world.get_component_for_entity::<UIText>(error).unwrap();
    assert!(line.text.contains("content asset"), "{}", line.text);
}

#[test]
fn an_entry_row_shows_its_clip_its_range_and_the_bones_it_drives() {
    let tab = tab();

    let rows = &tab.panels().entry_rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(tab.text(rows[0].name).text, "walk");
    assert_eq!(tab.text(rows[0].range).text, "1.00 to 2.50 s");
    assert_eq!(tab.text(rows[0].driven).text, "3 of 3");
    assert_eq!(tab.text(rows[1].name).text, "run");
    assert_eq!(tab.text(rows[1].range).text, "Whole clip");
    assert_eq!(tab.text(rows[1].driven).text, "2 of 3");
}

#[test]
fn driven_warns_when_it_differs_from_the_bones_of_the_clip() {
    let mut f = fixture();
    let stranger = write_asset(&mut f, "content/hero/stranger.gasset", &clip_of(1, 0..5));
    let mut tab = tab_with(f);
    let theme = UITheme::default();
    tab.state_mut().entries.push(entry_of(&stranger, None));
    tab.until_read();

    let rows = &tab.panels().entry_rows;
    assert_eq!(tab.text(rows[0].driven).color, theme.text);
    assert_eq!(tab.text(rows[1].driven).color, theme.text);
    assert_eq!(tab.text(rows[2].driven).text, "3 of 3");
    assert_eq!(tab.text(rows[2].driven).color, theme.warning);
}

#[test]
fn an_entry_says_what_it_knows_about_the_bones_it_drives() {
    let theme = UITheme::default();
    let facts = |read| EntryFacts {
        clip: AssetId::new(),
        skeleton: AssetId::new(),
        read,
    };
    let of = |driven, skeleton_bones, clip_bones| {
        Ok(ClipFacts {
            driven,
            skeleton_bones,
            clip_bones,
            ..Default::default()
        })
    };

    assert_eq!(
        driven_line(None, true, &theme),
        ("Reading…".into(), theme.text_muted)
    );
    assert_eq!(
        driven_line(Some(&facts(Err("gone".into()))), true, &theme),
        ("Can't read".into(), theme.warning)
    );
    assert_eq!(
        driven_line(Some(&facts(of(0, 0, 3))), false, &theme),
        ("No skeleton".into(), theme.text_muted)
    );
    assert_eq!(
        driven_line(Some(&facts(of(60, 65, 60))), true, &theme),
        ("60 of 65".into(), theme.text)
    );
    assert_eq!(
        driven_line(Some(&facts(of(60, 65, 61))), true, &theme),
        ("60 of 65".into(), theme.warning)
    );
}

#[test]
fn a_clip_that_cannot_be_read_says_so_on_its_row() {
    let mut f = fixture();
    let broken = write_asset(&mut f, "content/hero/broken.gasset", &clip_of(1, 0..1));
    std::fs::write(f.project_root.join(&broken.address), b"not a clip").unwrap();
    let mut tab = tab_with(f);
    tab.state_mut().entries.push(entry_of(&broken, None));

    tab.until_read();

    let row = &tab.panels().entry_rows[2];
    assert_eq!(tab.text(row.driven).text, "Can't read");
    assert_eq!(tab.text(row.driven).color, UITheme::default().warning);
    assert_eq!(tab.text(row.name).text, "broken");
    let facts = tab.state().facts_of(broken.id).unwrap();
    assert!(facts.read.as_ref().unwrap_err().contains("content asset"));
}

#[test]
fn a_clip_gone_from_the_project_reads_as_missing() {
    let mut tab = tab();
    let gone = AssetHandle::weak(AssetId::new());
    tab.state_mut().entries.push(MotionEntry {
        clip: gone,
        range: None,
    });

    tab.until_read();

    let row = &tab.panels().entry_rows[2];
    assert_eq!(tab.text(row.name).text, "Missing");
    assert_eq!(tab.text(row.driven).text, "Can't read");
}

#[test]
fn picking_a_skeleton_reads_the_entries_again_against_it() {
    let mut f = fixture();
    write_asset(
        &mut f,
        "content/other/rig.gasset",
        &skeleton_of(&["Hips", "Tail", "Ear", "Paw"]),
    );
    let mut tab = tab_with(f);
    tab.until_read();
    let button = tab.panels().skeleton_button;

    tab.click(button);
    let item = tab.asset_item("rig");
    tab.click(item);
    tab.f.frame();
    tab.until_read();

    let rows = &tab.panels().entry_rows;
    assert_eq!(tab.text(rows[0].driven).text, "3 of 4");
    assert_eq!(tab.text(rows[1].driven).text, "2 of 4");
}

#[test]
fn add_opens_the_clip_picker_and_a_pick_appends_a_whole_clip_entry_and_selects_it() {
    let mut f = fixture();
    let jog = write_asset(&mut f, "content/hero/jog.gasset", &clip_of(3, 0..3));
    let mut tab = tab_with(f);
    tab.until_read();
    let add = tab.panels().add_entry;

    tab.click(add);
    assert!(tab.asset_menu_open());
    let item = tab.asset_item("jog");
    tab.click(item);
    tab.f.frame();
    tab.until_read();

    assert!(!tab.asset_menu_open());
    assert_eq!(
        tab.state().entries[2],
        MotionEntry {
            clip: AssetHandle::weak(jog.id),
            range: None,
        }
    );
    assert_eq!(tab.state().selected_entry, Some(2));
    assert!(tab.unsaved());
    let rows = &tab.panels().entry_rows;
    assert_eq!(rows.len(), 3);
    assert_eq!(tab.text(rows[2].name).text, "jog");
    assert_eq!(tab.text(rows[2].range).text, "Whole clip");
    assert_eq!(tab.text(rows[2].driven).text, "3 of 3");
    assert!(tab.selected(rows[2].row));
    assert!(!tab.selected(rows[0].row));
    assert_eq!(tab.timeline().duration, 3.0);

    save(&mut tab.f, tab.document);
    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.entries.len(), 3);
    assert_eq!(saved.entries[2].clip.id(), jog.id);
}

#[test]
fn the_same_clip_can_be_added_twice() {
    let mut tab = tab();
    let add = tab.panels().add_entry;

    tab.click(add);
    let item = tab.asset_item("walk");
    tab.click(item);
    tab.f.frame();
    tab.f.frame();

    let rows = &tab.panels().entry_rows;
    assert_eq!(rows.len(), 3);
    assert_eq!(tab.text(rows[2].name).text, "walk");
    assert_eq!(tab.text(rows[2].driven).text, "3 of 3");
}

#[test]
fn remove_drops_the_entry_and_the_selection_moves_to_the_next_one() {
    let mut tab = tab();
    let select = tab.panels().entry_rows[0].row;
    tab.clicked(select);
    let remove = tab.panels().entry_rows[0].remove;

    tab.clicked(remove);

    assert_eq!(tab.state().entries.len(), 1);
    assert_eq!(tab.state().entries[0].clip.id(), tab.run.id);
    assert_eq!(tab.state().selected_entry, Some(0));
    assert!(tab.unsaved());
    assert_eq!(tab.panels().entry_rows.len(), 1);
    assert_eq!(tab.text(tab.panels().entry_rows[0].name).text, "run");
}

#[test]
fn removing_an_entry_above_the_selected_one_keeps_it_selected() {
    let mut tab = tab();
    let select = tab.panels().entry_rows[1].row;
    tab.clicked(select);
    let remove = tab.panels().entry_rows[0].remove;

    tab.clicked(remove);

    assert_eq!(tab.state().selected_entry, Some(0));
    assert_eq!(tab.state().entries[0].clip.id(), tab.run.id);
}

#[test]
fn removing_the_last_entry_while_it_is_selected_selects_the_one_before() {
    let mut tab = tab();
    let select = tab.panels().entry_rows[1].row;
    tab.clicked(select);
    let remove = tab.panels().entry_rows[1].remove;

    tab.clicked(remove);

    assert_eq!(tab.state().selected_entry, Some(0));
}

#[test]
fn removing_the_only_entry_selects_nothing_and_empties_the_timeline() {
    let mut tab = tab();
    for _ in 0..2 {
        let remove = tab.panels().entry_rows[0].remove;
        tab.clicked(remove);
    }

    assert_eq!(tab.state().selected_entry, None);
    assert!(tab.panels().entry_rows.is_empty());
    assert_eq!(tab.timeline().duration, 0.0);
    assert_eq!(tab.timeline().rate, None);
}

#[test]
fn clicking_an_entry_selects_it_and_the_timeline_follows() {
    let mut tab = tab();
    let rows = &tab.panels().entry_rows;
    let (first, second) = (rows[0].row, rows[1].row);
    assert!(tab.selected(first));
    assert_eq!(tab.timeline().duration, 10.0);
    assert_eq!(tab.timeline().rate, Some(30.0));
    assert_eq!(tab.timeline().selection, Some((30, 75)));

    tab.clicked(second);

    assert_eq!(tab.state().selected_entry, Some(1));
    assert!(tab.selected(second));
    assert!(!tab.selected(first));
    assert_eq!(tab.timeline().duration, 2.0);
    assert_eq!(tab.timeline().selection, None);
    assert!(!tab.unsaved());
}

#[test]
fn a_selection_on_the_timeline_writes_the_entrys_range() {
    let mut tab = tab();

    tab.select_frames(Some((3, 9)));

    assert_eq!(
        tab.state().entries[0].range,
        Some(TimeRange {
            start: time_of_frame(3, 30.0),
            end: time_of_frame(9, 30.0),
        })
    );
    assert_eq!(tab.timeline().selection, Some((3, 9)));
    assert!(tab.unsaved());
    let name = tab.panels().entry_rows[0].range;
    tab.f.frame();
    assert_eq!(tab.text(name).text, "0.10 to 0.30 s");
}

#[test]
fn a_selection_never_ends_after_the_clip() {
    let mut f = fixture();
    let mut times: Vec<f32> = (0..=30).map(|frame| frame as f32 / 30.0).collect();
    times.push(1.02);
    let outputs = vec![glam::Vec3::ZERO; times.len()];
    let mut clip = AnimationClip::default();
    clip.add_channel(
        bone_id(0),
        AnimationChannel::new(times, AnimationChanelOutput::Translation(outputs)),
    );
    let short = write_asset(&mut f, "content/hero/short.gasset", &clip);
    let mut tab = tab_with(f);
    tab.state_mut().entries = vec![entry_of(&short, None)];
    tab.until_read();
    let duration = tab.timeline().duration;
    let last = 31;
    assert!(time_of_frame(last, 30.0) > duration);

    tab.select_frames(Some((30, last)));

    let range = tab.state().entries[0].range.unwrap();
    assert_eq!(range.start, time_of_frame(30, 30.0));
    assert_eq!(range.end, duration);

    tab.select_frames(Some((last, last)));

    let range = tab.state().entries[0].range.unwrap();
    assert!(range.start <= range.end);
    assert!(range.end <= duration);
}

#[test]
fn a_click_on_the_timeline_clears_the_range() {
    let mut tab = tab();

    tab.select_frames(None);

    assert_eq!(tab.state().entries[0].range, None);
    assert_eq!(tab.timeline().selection, None);
    assert!(tab.unsaved());
}

#[test]
fn clearing_a_range_that_is_not_there_leaves_the_document_saved() {
    let mut tab = tab();
    let second = tab.panels().entry_rows[1].row;
    tab.clicked(second);

    tab.select_frames(None);

    assert!(!tab.unsaved());
}

#[test]
fn a_saved_range_is_in_the_file() {
    let mut tab = tab();
    tab.select_frames(Some((3, 9)));

    save(&mut tab.f, tab.document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.entries, tab.state().entries);
    assert!(!tab.unsaved());
}

#[test]
fn a_clip_with_no_detected_rate_has_no_frames_to_select() {
    let mut f = fixture();
    let mut clip = AnimationClip::default();
    clip.add_channel(
        bone_id(0),
        AnimationChannel::new(
            vec![2.0],
            AnimationChanelOutput::from_translation([[0.0; 3]].into_iter()),
        ),
    );
    let still = write_asset(&mut f, "content/hero/still.gasset", &clip);
    let mut tab = tab_with(f);
    tab.state_mut().entries = vec![entry_of(
        &still,
        Some(TimeRange {
            start: 0.0,
            end: 1.0,
        }),
    )];

    tab.until_read();

    assert_eq!(tab.timeline().duration, 2.0);
    assert_eq!(tab.timeline().rate, None);
    assert_eq!(tab.timeline().selection, None);
}

#[test]
fn the_settings_row_selects_the_settings_entity() {
    let mut tab = tab();
    let settings = settings_entity(&mut tab.f, tab.document);
    let row = tab.panels().settings;
    assert!(!tab.selected(row));

    tab.clicked(row);

    assert_eq!(tab.selection(), Some(settings));
    assert!(tab.selected(row));
    assert!(!tab.unsaved());
}

#[test]
fn a_channel_row_names_its_type_and_selects_its_entity() {
    let mut tab = tab();
    let entities = channels(&mut tab.f, tab.document);

    assert_eq!(
        tab.channel_names(),
        [
            "BonePositions",
            "BoneVelocities",
            "BoneVelocities",
            "TrajectoryPositions",
            "TrajectoryDirections",
        ]
    );

    let row = tab.channel_row(2).row;
    tab.clicked(row);

    assert_eq!(tab.selection(), Some(entities[2]));
    assert!(tab.selected(row));
    assert!(!tab.selected(tab.channel_row(1).row));
    assert!(!tab.selected(tab.panels().settings));
    assert!(!tab.unsaved());
}

#[test]
fn the_add_menu_lists_the_channels_the_game_registers() {
    let mut tab = tab();
    let add = tab.panels().add_channel;

    tab.click(add);

    assert!(tab.name_menu_open());
    assert_eq!(
        tab.menu_names(),
        [
            "BonePositions",
            "BoneVelocities",
            "TrajectoryDirections",
            "TrajectoryPositions",
        ]
    );

    tab.click(add);

    assert!(!tab.name_menu_open());
}

#[test]
fn a_channel_added_from_the_menu_goes_last_at_its_default_and_marks_the_document_unsaved() {
    let mut tab = tab();
    let add = tab.panels().add_channel;
    tab.click(add);
    let item = tab.name_item("BonePositions");

    tab.clicked(item);

    assert!(!tab.name_menu_open());
    let entities = channels(&mut tab.f, tab.document);
    assert_eq!(entities.len(), 6);
    assert_eq!(
        component::<BonePositions>(&tab.f, tab.document, entities[5]),
        BonePositions::default()
    );
    assert_eq!(tab.channel_names()[5], "BonePositions");
    assert!(tab.f.scene_world(tab.document).entity_is_valid(entities[5]));
    assert!(tab.unsaved());

    save(&mut tab.f, tab.document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.channels.len(), 6);
    assert_eq!(saved.channels[5].type_name, "BonePositions");
    assert!(!tab.unsaved());
}

#[test]
fn a_channel_added_after_a_removal_still_goes_last() {
    let mut tab = tab();
    let remove = tab.channel_remove(0);
    tab.clicked(remove);
    let add = tab.panels().add_channel;
    tab.click(add);
    let item = tab.name_item("BonePositions");
    tab.clicked(item);

    let names = tab.channel_names();
    assert_eq!(names.len(), 5);
    assert_eq!(names[4], "BonePositions");
}

#[test]
fn removing_a_channel_despawns_it_and_keeps_the_others_in_order() {
    let mut tab = tab();
    let entities = channels(&mut tab.f, tab.document);
    let remove = tab.channel_remove(1);

    tab.clicked(remove);

    assert_eq!(
        tab.channel_names(),
        [
            "BonePositions",
            "BoneVelocities",
            "TrajectoryPositions",
            "TrajectoryDirections",
        ]
    );
    assert!(!tab.f.scene_world(tab.document).entity_is_valid(entities[1]));
    assert!(tab.unsaved());

    save(&mut tab.f, tab.document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    let names: Vec<&str> = saved
        .channels
        .iter()
        .map(|channel| channel.type_name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "BonePositions",
            "BoneVelocities",
            "TrajectoryPositions",
            "TrajectoryDirections",
        ]
    );
}

#[test]
fn removing_the_selected_channel_clears_the_selection() {
    let mut tab = tab();
    let (select, remove) = (tab.channel_row(1).row, tab.channel_remove(1));
    tab.clicked(select);
    assert!(tab.selection().is_some());

    tab.clicked(remove);

    assert_eq!(tab.selection(), None);
}

#[test]
fn removing_another_channel_keeps_the_selection() {
    let mut tab = tab();
    let select = tab.channel_row(1).row;
    let remove = tab.channel_remove(3);
    tab.clicked(select);
    let selected = tab.selection();

    tab.clicked(remove);

    assert_eq!(tab.selection(), selected);
    assert!(selected.is_some());
}

#[test]
fn a_status_shows_on_a_line_of_its_own_and_hides_when_empty() {
    let mut tab = tab();
    let status = tab.panels().status;
    assert!(!tab.shown(status));

    tab.set_status("Saved content/database.gasset");
    tab.f.frame();

    assert!(tab.shown(status));
    assert_eq!(tab.text(status).text, "Saved content/database.gasset");

    tab.set_status("");
    tab.f.frame();

    assert!(!tab.shown(status));
}

#[test]
fn a_failed_preview_shows_what_was_read_and_changes_nothing() {
    let mut f = fixture_with(broken_recipe);
    let skeleton = write_asset(&mut f, "content/rig.gasset", &skeleton_of(&["Hips"]));
    let walk = write_asset(&mut f, "content/walk.gasset", &clip_of(10, 0..1));
    let asset = write_definition(
        &mut f,
        "content/database.gasset",
        &MotionDatabaseDefinition {
            skeleton: AssetHandle::weak(skeleton.id),
            entries: vec![entry_of(&walk, None)],
            ..Default::default()
        },
    );
    let document = request(&mut f, &asset);
    hosts(&mut f, document);
    let mut tab = Tab {
        f,
        document,
        run: walk.clone(),
        walk,
        skeleton,
        asset,
    };
    tab.until_read();

    let panels = tab.panels();
    assert_eq!(tab.text(panels.entry_rows[0].name).text, "walk");
    assert_eq!(tab.channel_names().len(), 5);
    assert!(tab.channel_row(0).entity.is_none());
    for button in [
        panels.skeleton_button,
        panels.add_entry,
        panels.add_channel,
        panels.settings,
        panels.entry_rows[0].remove,
    ] {
        assert!(tab.disabled(button));
    }
    assert!(
        tab.world()
            .get_component_for_entity::<UIInteractionStyle>(tab.channel_row(0).row)
            .is_none()
    );
    assert!(!tab.disabled(panels.entry_rows[0].row));

    tab.select_frames(Some((3, 9)));

    assert_eq!(tab.state().entries[0].range, None);
    assert!(!tab.unsaved());
    assert_eq!(tab.timeline().selection, None);
    assert!(
        tab.text(tab.panels().status)
            .text
            .starts_with("Preview failed")
    );
}

#[test]
fn nothing_can_be_edited_before_the_definition_opens() {
    let mut f = fixture();
    let asset = write_definition(&mut f, "content/database.gasset", &definition());
    let document = f.request_kind(MotionDatabaseDefinition::name());
    hosts(&mut f, document);
    let tab = Tab {
        f,
        document,
        walk: asset.clone(),
        run: asset.clone(),
        skeleton: asset.clone(),
        asset,
    };

    let panels = tab.panels();
    for button in [panels.skeleton_button, panels.add_entry, panels.add_channel] {
        assert!(tab.disabled(button));
    }
}

#[test]
fn a_second_definition_replaces_the_rows_and_the_timeline() {
    let mut tab = tab();
    let other = MotionDatabaseDefinition {
        entries: vec![entry_of(&tab.run, None)],
        channels: vec![],
        ..Default::default()
    };
    let other = write_definition(&mut tab.f, "content/other.gasset", &other);

    open(&mut tab.f, &other);
    tab.until_read();

    assert_eq!(tab.panels().entry_rows.len(), 1);
    assert_eq!(tab.text(tab.panels().entry_rows[0].name).text, "run");
    assert_eq!(tab.state().selected_entry, Some(0));
    assert_eq!(tab.timeline().duration, 2.0);
    assert!(tab.channel_names().is_empty());
    assert_eq!(tab.text(tab.panels().skeleton).text, "None");
}

#[test]
fn a_channel_row_for_a_definition_opened_again_is_not_stale() {
    let mut tab = tab();
    let first: Vec<Entity> = tab
        .panels()
        .channel_rows
        .iter()
        .filter_map(|row| row.entity)
        .collect();
    let other = write_definition(&mut tab.f, "content/other.gasset", &definition());
    let asset = tab.asset.clone();

    open(&mut tab.f, &other);
    open(&mut tab.f, &asset);
    tab.f.frame();

    let now: Vec<Entity> = tab
        .panels()
        .channel_rows
        .iter()
        .filter_map(|row| row.entity)
        .collect();
    assert_eq!(now.len(), first.len());
    assert_ne!(now, first);
    assert_eq!(now, channels(&mut tab.f, tab.document));
}

#[test]
fn a_background_tab_keeps_its_rows() {
    let mut tab = tab();
    let names = tab.channel_names();

    let front = tab.f.open();
    assert_ne!(front, tab.document);
    tab.f.frame();

    assert_eq!(tab.channel_names(), names);
}

#[test]
fn an_edit_in_the_inspector_leaves_the_channel_rows_alone() {
    let mut tab = tab();
    let rows: Vec<Entity> = tab
        .panels()
        .channel_rows
        .iter()
        .map(|row| row.row)
        .collect();
    let channel = channels(&mut tab.f, tab.document)[1];

    set::<BoneVelocities>(&mut tab.f, channel, ["weight"], json!(2.5));
    tab.f.frame();

    let now: Vec<Entity> = tab
        .panels()
        .channel_rows
        .iter()
        .map(|row| row.row)
        .collect();
    assert_eq!(now, rows);
}

#[test]
fn the_left_panel_scrolls_over_what_it_holds() {
    let mut tab = tab();
    let (view, content) = (tab.panels().view, tab.panels().content);
    let layout = |height: f32| {
        let rect = UIBox {
            min: Vec2::ZERO,
            size: Vec2::new(200.0, height),
        };
        UILayout {
            rect,
            content_rect: rect,
            clip_rect: rect,
            paint_order: 0,
        }
    };
    tab.f.world.insert(layout(100.0), view);
    tab.f.world.insert(layout(400.0), content);
    tab.f
        .world
        .get_component_for_entity_mut::<UIScrollArea>(view)
        .unwrap()
        .offset = 900.0;
    tab.f.add_delivery(sync_left_scroll.into_boxed_system());

    tab.f.frame();

    let area = tab
        .f
        .world
        .get_component_for_entity::<UIScrollArea>(view)
        .unwrap();
    assert_eq!(area.content_extent, 400.0);
    assert_eq!(area.offset, 300.0);
}

#[derive(Resource, Default)]
struct Writes(usize);

fn count_writes(
    texts: Query<&UIText, Changed<UIText>>,
    nodes: Query<&UINode, Changed<UINode>>,
    timelines: Query<&Timeline, Changed<Timeline>>,
    documents: Query<&DefinitionDocument, Changed<DefinitionDocument>>,
    mut writes: ResMut<Writes>,
) {
    writes.0 = texts.iter().count()
        + nodes.iter().count()
        + timelines.iter().count()
        + documents.iter().count();
}

#[test]
fn a_settled_panel_writes_nothing() {
    let mut f = fixture();
    f.world.insert_resource(Writes::default());
    f.add_delivery(count_writes.into_boxed_system());
    let mut tab = tab_with(f);
    tab.until_read();
    tab.f.frame();
    tab.f.frame();
    tab.f.frame();

    assert_eq!(tab.world().get_resource::<Writes>().unwrap().0, 0);
}

#[test]
fn a_read_in_progress_asks_an_agents_host_for_frames() {
    let mut f = fixture();
    f.world.insert_resource(FrameDemand::default());
    let mut tab = tab_with(f);
    tab.until_read();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    tab.state_mut().facts_job = Some(std::thread::spawn(move || {
        let _ = wait.recv();
        Vec::new()
    }));
    tab.f.world.insert_resource(FrameDemand::default());

    tab.f.frame();

    assert!(
        tab.world()
            .get_resource::<FrameDemand>()
            .unwrap()
            .is_requested()
    );

    release.send(()).unwrap();
    tab.until_read();
    tab.f.world.insert_resource(FrameDemand::default());
    tab.f.frame();

    assert!(
        !tab.world()
            .get_resource::<FrameDemand>()
            .unwrap()
            .is_requested()
    );
}

#[test]
fn entry_facts_hold_what_the_clip_holds() {
    let tab = tab();

    let facts = tab.state().facts_of(tab.walk.id).unwrap();

    assert_eq!(facts.skeleton, tab.skeleton.id);
    let read = facts.read.as_ref().unwrap();
    assert_eq!(read.duration, 10.0);
    assert_eq!(read.rate, Some(30.0));
    assert_eq!(read.clip_bones, 3);
    assert_eq!(read.skeleton_bones, 3);
    assert_eq!(read.driven, 3);
}

#[test]
fn opening_a_definition_again_reads_its_clips_again() {
    let mut tab = tab();
    let walk = tab.walk.clone();
    let shorter = clip_of(4, 0..3);
    let root = tab.f.project_root.clone();
    let assets = tab.f.world.get_resource_mut::<AssetRegistry>().unwrap();
    save_content_asset_into(&shorter, &root, &walk.address, assets).unwrap();
    let other = write_definition(
        &mut tab.f,
        "content/other.gasset",
        &MotionDatabaseDefinition::default(),
    );
    let asset = tab.asset.clone();

    open(&mut tab.f, &other);
    open(&mut tab.f, &asset);
    tab.until_read();

    let facts: &EntryFacts = tab.state().facts_of(walk.id).unwrap();
    assert_eq!(facts.read.as_ref().unwrap().duration, 4.0);
}

#[test]
fn a_channel_entity_carries_the_name_its_row_shows() {
    let mut tab = tab();
    let world = tab.f.scene_world_mut(tab.document);
    let kinds: Vec<(ChannelOrder, &'static str)> = world
        .query::<(&ChannelOrder, &ChannelKind), ()>()
        .iter(world)
        .map(|(order, kind)| (*order, kind.0))
        .collect();

    assert_eq!(kinds.len(), 5);
    assert!(kinds.contains(&(ChannelOrder(0), "BonePositions")));
}

#[test]
fn a_select_from_elsewhere_lights_its_row() {
    let mut tab = tab();
    let channel = channels(&mut tab.f, tab.document)[3];

    tab.f.signal(Select(Some(channel)));
    tab.f.frame();

    assert!(tab.selected(tab.channel_row(3).row));
}

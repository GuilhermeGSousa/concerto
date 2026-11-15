use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_ecs::{
    Component, Entity, With, World,
    entity::hierarchy::{ChildOf, Children},
};
use concerto_editor::{
    asset_editor::EditorHosts,
    asset_picker::AssetPickerMenu,
    hosted::testing::SceneFixture,
    name_menu::NameMenu,
    timeline::{TimelineSelected, time_of_frame},
};
use concerto_foundation::assets::{handle::AssetHandle, registry::AssetEntry};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::{MotionDatabase, MotionEntry, TimeRange};
use concerto_ui::{anchor::UIAnchoredPanel, interaction::UIClick, node::UINode, text::UIText};
use concerto_window::input::MouseButton;
use glam::Mat4;
use uuid::Uuid;

use super::*;
use crate::{
    DefinitionDocument, read_definition,
    tests::{fixture, open, save, state, write_asset, write_definition},
};

pub(crate) struct Tab {
    pub(crate) f: SceneFixture,
    pub(crate) document: Entity,
    pub(crate) run: AssetEntry,
    pub(crate) asset: AssetEntry,
}

pub(crate) fn bone_id(bone: usize) -> Uuid {
    Uuid::from_u128(bone as u128 + 1)
}

pub(crate) fn skeleton_of(names: &[&str]) -> Skeleton {
    let bones = names.len();
    Skeleton::new(
        vec![Mat4::IDENTITY; bones],
        (0..bones)
            .map(|bone| bone.checked_sub(1).map(|parent| parent as u16))
            .collect(),
        names.iter().map(|name| (*name).into()).collect(),
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
        mirrored: false,
    }
}

pub(crate) fn hosts(f: &mut SceneFixture, document: Entity) {
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

pub(crate) fn tab_with(f: SceneFixture) -> Tab {
    tab_of(f, |_| {})
}

pub(crate) fn tab_of(mut f: SceneFixture, change: impl FnOnce(&mut MotionDatabase)) -> Tab {
    let skeleton = write_asset(
        &mut f,
        "content/hero/skeleton.gasset",
        &skeleton_of(&["Hips", "Spine", "Head"]),
    );
    let walk = write_asset(&mut f, "content/hero/walk.gasset", &clip_of(10, 0..3));
    let run = write_asset(&mut f, "content/hero/run.gasset", &clip_of(2, 0..2));
    let mut opened = MotionDatabase {
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
    change(&mut opened);
    let asset = write_definition(&mut f, "content/hero/database.gasset", &opened);
    let document = open(&mut f, &asset);
    hosts(&mut f, document);
    Tab {
        f,
        document,
        run,
        asset,
    }
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

    pub(crate) fn state_mut(&mut self) -> &mut DefinitionDocument {
        self.f
            .world
            .get_component_for_entity_mut::<DefinitionDocument>(self.document)
            .unwrap()
    }

    pub(crate) fn world(&self) -> &World {
        &self.f.world
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

    fn under(&self, entity: Entity, menus: &[Entity]) -> bool {
        let mut ancestor = Some(entity);
        while let Some(entity) = ancestor {
            if menus.contains(&entity) {
                return true;
            }
            ancestor = self
                .world()
                .get_component_for_entity::<ChildOf>(entity)
                .map(ChildOf::parent);
        }
        false
    }

    fn item_in<M: Component>(&mut self, name: &str) -> Entity {
        let world = &mut self.f.world;
        let menus: Vec<Entity> = world
            .query::<Entity, (With<UIAnchoredPanel>, With<M>)>()
            .iter(world)
            .collect();
        let labels: Vec<Entity> = world
            .query::<(Entity, &UIText), ()>()
            .iter(world)
            .filter(|(_, text)| text.text == name)
            .map(|(entity, _)| entity)
            .collect();
        let label = labels
            .into_iter()
            .find(|label| self.under(*label, &menus))
            .expect("the menu lists the name");
        self.parent(label)
    }

    pub(crate) fn unsaved(&mut self) -> bool {
        self.f.unsaved(self.document)
    }

    pub(crate) fn until_settled(&mut self) {
        let document = self.document;
        self.f.frames_until("the workers never settled", |f| {
            let state = state(f, document);
            state.derive_job.is_none() && !state.derive_wanted && state.clip_job.is_none()
        });
        self.f.frame();
        self.f.frame();
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
}

#[test]
fn the_left_panels_edits_of_the_skeleton_entries_ranges_and_channels_are_saved() {
    let mut f = fixture();
    let rig = write_asset(
        &mut f,
        "content/other/rig.gasset",
        &skeleton_of(&["Hips", "Spine", "Head"]),
    );
    let jog = write_asset(&mut f, "content/hero/jog.gasset", &clip_of(3, 0..3));
    let mut tab = tab_with(f);
    tab.until_read();

    let picker = tab.panels().skeleton_button;
    tab.click(picker);
    let item = tab.item_in::<AssetPickerMenu>("rig");
    tab.click(item);
    tab.f.frame();
    tab.until_read();
    let add = tab.panels().add_entry;
    tab.click(add);
    let item = tab.item_in::<AssetPickerMenu>("jog");
    tab.click(item);
    tab.f.frame();
    tab.until_read();
    let remove = tab.panels().entry_rows[0].remove;
    tab.clicked(remove);
    let mirrored = tab.panels().entry_rows[0].mirrored;
    tab.clicked(mirrored);
    let timeline = tab.panels().timeline;
    tab.f.world.trigger_on(
        timeline,
        TimelineSelected {
            range: Some((3, 9)),
        },
    );
    tab.f.frame();
    let add = tab.panels().add_channel;
    tab.click(add);
    let item = tab.item_in::<NameMenu>("BonePositions");
    tab.clicked(item);
    let remove = tab.channel_remove(1);
    tab.clicked(remove);
    assert!(tab.unsaved());

    save(&mut tab.f, tab.document);

    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.skeleton.id(), rig.id);
    assert_eq!(
        saved.entries,
        [
            MotionEntry {
                clip: AssetHandle::weak(tab.run.id),
                range: None,
                mirrored: true,
            },
            MotionEntry {
                clip: AssetHandle::weak(jog.id),
                range: Some(TimeRange {
                    start: time_of_frame(3, 30.0),
                    end: time_of_frame(9, 30.0),
                }),
                mirrored: false,
            },
        ]
    );
    let channels: Vec<&str> = saved
        .channels
        .iter()
        .map(|channel| channel.type_name())
        .collect();
    assert_eq!(
        channels,
        [
            "BonePositions",
            "BoneVelocities",
            "TrajectoryPositions",
            "TrajectoryDirections",
            "BonePositions",
        ]
    );
    assert!(!tab.unsaved());
}

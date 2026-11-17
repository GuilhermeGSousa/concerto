use super::*;
use crate::{
    preview::{ClipPlayback, ClipPreviewProgress},
    transport::{TransportRow, spawn_transport},
};
use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel};
use concerto_ecs::{
    IntoSystem, System, component::name::Name as EntityName, events::event_channel::EventChannel,
};
use concerto_editor::{
    asset_editor::{ActiveEditor, AssetEditorRegistry, SaveRequested},
    guard::DiscardEdits,
    hosted::{
        Hosted, HostedApp,
        testing::{SceneFixture, forwarding},
    },
    inspected_asset::{ProjectTypes, inspected_asset_systems, install_inspected_listeners},
    inspector::{InspectedSnapshots, InspectorRegistry, WriteValue},
};
use concerto_foundation::{
    assets::{
        asset_server::AssetServer, asset_store::AssetStore, content::save_content_asset_into,
    },
    transform::Transform,
};
use concerto_mesh::skeleton::JointPose;
use concerto_ui::{interaction::UIClick, node::UINode, theme::UITheme};
use concerto_window::input::MouseButton;
use glam::{Mat4, Quat, Vec3};
use serde_json::{Value, json};
use uuid::Uuid;

pub(super) const NAMES: [&str; 4] = ["root", "pelvis", "spine", "head"];
pub(super) const PELVIS: usize = 1;

pub(super) fn skeleton(first_id: u128, heights: [f32; 4]) -> Skeleton {
    let at = |y| JointPose {
        translation: Vec3::new(0.0, y, 0.0),
        ..JointPose::IDENTITY
    };
    Skeleton::new(
        vec![Mat4::IDENTITY; 4],
        vec![None, Some(0), Some(1), Some(2)],
        NAMES.map(Into::into).to_vec(),
        heights.map(at).to_vec(),
        (first_id..first_id + 4).map(Uuid::from_u128).collect(),
    )
    .unwrap()
}

pub(super) fn source_skeleton() -> Skeleton {
    skeleton(10, [0.0, 1.0, 0.3, 0.4])
}

pub(super) fn target_skeleton() -> Skeleton {
    skeleton(20, [0.0, 0.9, 0.25, 0.35])
}

pub(super) fn rig_on(skeleton: &AssetEntry) -> RetargetRig {
    RetargetRig::new()
        .with_skeleton(AssetHandle::weak(skeleton.id))
        .with_root("pelvis")
        .with_chain(RetargetChain::new("spine", "spine", "head"))
}

pub(super) fn walk_on(skeleton: &Skeleton) -> AnimationClip {
    let times: Vec<f32> = (0..=30).map(|frame| frame as f32 / 30.0).collect();
    let translations: Vec<[f32; 3]> = times.iter().map(|time| [*time, 1.0, 0.0]).collect();
    let rotations: Vec<[f32; 4]> = times
        .iter()
        .map(|time| Quat::from_rotation_x(*time).to_array())
        .collect();
    let mut clip = AnimationClip::default();
    clip.add_channel(
        skeleton.bone_ids[PELVIS],
        AnimationChannel::new(
            times.clone(),
            AnimationChanelOutput::from_translation(translations.into_iter()),
        ),
    );
    clip.add_channel(
        skeleton.bone_ids[2],
        AnimationChannel::new(
            times,
            AnimationChanelOutput::from_rotation(rotations.into_iter()),
        ),
    );
    clip
}

pub(super) fn rig_fixture() -> SceneFixture {
    let mut f = SceneFixture::new();
    f.world
        .get_resource_mut::<AssetEditorRegistry>()
        .unwrap()
        .register_creatable::<RetargetRig, RetargetRigEditor>()
        .unwrap();
    let types = AppTypeRegistry::default();
    types.write().register::<RetargetRig>();
    f.world.insert_resource(ProjectTypes {
        registry: Some(types),
    });
    f.world.insert_resource(RigPreviewTypes(preview_types()));
    f.world.insert_resource(InspectorRegistry::default());
    f.world.insert_resource(InspectedSnapshots::default());
    let rigs = AssetStore::<RetargetRig>::new();
    let clips = AssetStore::<AnimationClip>::new();
    let server = f.world.get_resource_mut::<AssetServer>().unwrap();
    server.register_asset(&rigs);
    server.register_derived_asset(&clips);
    f.world.insert_resource(rigs);
    f.world.insert_resource(clips);
    f.world
        .insert_resource(EventChannel::<Hosted<ClipPreviewProgress>>::new());
    f.world
        .insert_resource(EventChannel::<AssetsWritten>::new());
    install_inspected_listeners(&mut f.world);
    f.add_delivery(forwarding::<ClipPreviewProgress>());
    f.add_delivery(follow_transport_progress.into_boxed_system());
    for system in inspected_asset_systems() {
        f.add_delivery(system);
    }
    f.add_delivery(sync_rig_transport.into_boxed_system());
    f.add_delivery(follow_rig.into_boxed_system());
    f
}

pub(super) fn write<A: Asset>(f: &mut SceneFixture, address: &str, asset: &A) -> AssetEntry {
    let root = f.project_root.clone();
    let assets = f.world.get_resource_mut::<AssetRegistry>().unwrap();
    let id = save_content_asset_into(asset, None, &root, address, assets).unwrap();
    assets.get(id).unwrap().clone()
}

pub(super) struct RigTab {
    pub(super) f: SceneFixture,
    pub(super) document: Entity,
    pub(super) open: AssetEntry,
    pub(super) open_skeleton: AssetEntry,
    pub(super) other: AssetEntry,
    pub(super) walk: AssetEntry,
}

const OPEN_ADDRESS: &str = "content/hero/rig.gasset";
const OTHER_ADDRESS: &str = "content/other/rig.gasset";

pub(super) fn rig_tab() -> RigTab {
    let mut f = rig_fixture();
    let open_skeleton = write(&mut f, "content/hero/skeleton.gasset", &target_skeleton());
    let other_skeleton = write(&mut f, "content/other/skeleton.gasset", &source_skeleton());
    let open = write(&mut f, OPEN_ADDRESS, &rig_on(&open_skeleton));
    let other = write(&mut f, OTHER_ADDRESS, &rig_on(&other_skeleton));
    let walk = write(
        &mut f,
        "content/other/walk.gasset",
        &walk_on(&source_skeleton()),
    );
    f.request(&open);
    let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
    f.frames_until("the rig never finished opening", |f| {
        f.document(document).pending.is_none()
            && f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_some()
    });
    RigTab {
        f,
        document,
        open,
        open_skeleton,
        other,
        walk,
    }
}

pub(super) fn hex(entry: &AssetEntry) -> Value {
    json!(entry.id.simple_hex())
}

impl RigTab {
    pub(super) fn state(&self) -> &RigDocument {
        self.f
            .world
            .get_component_for_entity::<RigDocument>(self.document)
            .unwrap()
    }

    pub(super) fn rig(&self) -> &RetargetRig {
        self.f
            .world
            .get_component_for_entity::<InspectedDocument>(self.document)
            .unwrap()
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<RetargetRig>())
            .unwrap()
    }

    pub(super) fn values(&self) -> Vec<Arc<dyn PropertyTarget>> {
        self.f
            .world
            .get_component_for_entity::<InspectedValues>(self.document)
            .unwrap()
            .0
            .clone()
    }

    pub(super) fn write_value(&mut self, value: usize, path: &str, json: Value) {
        let target = self.values()[value].clone();
        self.f.world.trigger(WriteValue {
            target,
            path: path.into(),
            json,
        });
        self.f.frame();
    }

    pub(super) fn write_rig(&mut self, path: &str, json: Value) {
        self.write_value(0, path, json);
    }

    pub(super) fn write_preview(&mut self, path: &str, json: Value) {
        self.write_value(1, path, json);
    }

    fn transport(&self) -> &ClipTransport {
        self.f
            .world
            .get_component_for_entity::<ClipTransport>(self.document)
            .unwrap()
    }

    fn playback(&self) -> &ClipPlayback {
        self.f
            .scene_world(self.document)
            .get_resource::<ClipPlayback>()
            .unwrap()
    }

    fn until(&mut self, what: &str, done: impl Fn(&RigTab) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            self.f.frame();
            if done(self) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn playing_the_walk(mut self) -> Self {
        let (other, walk) = (hex(&self.other), hex(&self.walk));
        self.write_preview("other", other);
        self.write_preview("clip", walk);
        self.write_preview("mode", json!("Clip"));
        self.until("the walk never played", |tab| {
            tab.playback().clip.is_some() && tab.transport().time > 0.0
        });
        self
    }

    fn play_button(&mut self) -> Entity {
        let document = self.document;
        let host = self.f.world.spawn(UINode::default());
        let mut spawn = (move |theme: Res<UITheme>, mut cmd: CommandQueue| {
            spawn_transport(&mut cmd.entity(host), document, &theme);
        })
        .into_system();
        spawn.initialize(&mut self.f.world);
        spawn.run_and_apply((), &mut self.f.world);
        let world = &mut self.f.world;
        world
            .query::<&TransportRow, ()>()
            .iter(world)
            .next()
            .unwrap()
            .play
    }

    fn edit_every_frame(&mut self, edit: usize) {
        if edit.is_multiple_of(2) {
            let root = if edit.is_multiple_of(4) {
                "spine"
            } else {
                "pelvis"
            };
            self.write_rig("root", json!(root));
        } else {
            self.write_preview("bones", json!(edit % 4 == 1));
        }
    }

    fn spine_rotations(&mut self) -> Vec<Quat> {
        let world = self.f.scene_world_mut(self.document);
        let spines: Vec<Entity> = world
            .query::<(Entity, &EntityName), ()>()
            .iter(world)
            .filter(|(_, name)| name.as_str() == "spine")
            .map(|(entity, _)| entity)
            .collect();
        spines
            .into_iter()
            .map(|spine| {
                world
                    .get_component_for_entity::<Transform>(spine)
                    .unwrap()
                    .rotation
            })
            .collect()
    }
}

pub(super) fn left_click() -> UIClick {
    UIClick {
        position: Default::default(),
        button: MouseButton::Left,
    }
}

#[test]
fn a_rig_edit_is_unsaved_saved_and_discarded_and_a_preview_edit_leaves_the_tab_clean() {
    let mut tab = rig_tab();
    let names: Vec<String> = tab.values().iter().map(|value| value.name()).collect();
    assert_eq!(names, ["RetargetRig", "Preview"]);
    assert_eq!(*tab.rig(), rig_on(&tab.open_skeleton));

    tab.write_preview("bones", json!(false));
    assert!(!tab.state().preview.bones);
    assert!(!tab.f.unsaved(tab.document));

    tab.write_rig("root", json!("spine"));
    assert!(tab.f.unsaved(tab.document));
    tab.f.world.insert(SaveRequested, tab.document);
    tab.f.frame();

    let bytes = std::fs::read(tab.f.project_root.join(OPEN_ADDRESS)).unwrap();
    let (header, saved) = decode_content_asset::<RetargetRig>(&bytes).unwrap();
    assert_eq!(header.asset_id, tab.open.id);
    assert_eq!(saved.root, "spine".into());
    assert!(!tab.f.unsaved(tab.document));

    tab.write_rig("root", json!("head"));
    tab.f.world.trigger(DiscardEdits(tab.document));
    tab.f.frame();

    assert_eq!(tab.rig().root, "spine".into());
}

#[test]
fn the_preview_value_plays_the_clip_through_the_retarget_and_pauses_where_told() {
    let mut tab = rig_tab();
    let (other, walk) = (hex(&tab.other), hex(&tab.walk));
    tab.write_preview("other", other);
    tab.write_preview("clip", walk);
    tab.write_preview("mode", json!("Clip"));

    let document = tab.document;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        tab.f.frame();
        let spines = tab.spine_rotations();
        if spines.len() == 2
            && spines
                .iter()
                .all(|rotation| !rotation.abs_diff_eq(Quat::IDENTITY, 1e-3))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "both spines never turned: {spines:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(tab.f.status(document), "");

    tab.write_preview("paused_at", json!({"Some": {"0": 0.5}}));

    tab.f.frames_until("the preview never paused at 0.5", |f| {
        let playback = f
            .scene_world(document)
            .get_resource::<ClipPlayback>()
            .unwrap();
        !playback.playing && playback.time == 0.5
    });
    for _ in 0..5 {
        tab.f.frame();
    }
    let transport = tab
        .f
        .world
        .get_component_for_entity::<ClipTransport>(document)
        .unwrap();
    assert!(!transport.playing);
    assert_eq!(transport.time, 0.5);
    assert_eq!(tab.state().preview.paused_at, Some(0.5));
}

#[test]
fn a_pause_from_the_transport_holds_while_the_rig_is_edited_every_frame_and_play_clears_it() {
    let mut tab = rig_tab().playing_the_walk();
    let play = tab.play_button();
    for edit in 0..5 {
        tab.write_rig(
            "root",
            json!(if edit % 2 == 0 { "spine" } else { "pelvis" }),
        );
    }

    tab.f.world.trigger_on(play, left_click());
    for edit in 5..15 {
        tab.write_rig(
            "root",
            json!(if edit % 2 == 0 { "spine" } else { "pelvis" }),
        );
    }

    assert!(tab.state().preview.paused_at.is_some());
    assert_eq!(tab.state().preview.paused_at, Some(tab.transport().time));
    assert!(tab.transport().time > 0.0);
    assert_eq!(tab.playback().time, tab.transport().time);
    assert!(!tab.transport().playing);
    assert!(!tab.playback().playing);

    tab.f.world.trigger_on(play, left_click());
    tab.until("play never cleared paused_at", |tab| {
        tab.state().preview.paused_at.is_none()
    });
    assert!(tab.transport().playing);
}

#[test]
fn a_clip_that_does_not_loop_stops_at_its_end_while_the_rig_and_the_preview_are_edited_every_frame()
{
    let mut tab = rig_tab().playing_the_walk();
    tab.write_preview("looping", json!(false));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut edit = 0;
    while tab.state().preview.paused_at.is_none() {
        assert!(std::time::Instant::now() < deadline, "the clip never ended");
        tab.edit_every_frame(edit);
        edit += 1;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    for _ in 0..6 {
        tab.edit_every_frame(edit);
        edit += 1;
    }

    assert_eq!(tab.state().preview.paused_at, Some(1.0));
    assert!(!tab.state().preview.looping);
    assert!(!tab.transport().playing);
    assert!(!tab.playback().playing);
    assert_eq!(tab.playback().time, 1.0);
}

#[test]
fn a_clip_that_does_not_loop_ending_leaves_the_saved_status_alone() {
    let mut tab = rig_tab().playing_the_walk();
    tab.write_preview("speed", json!(0.25));
    tab.write_preview("looping", json!(false));
    tab.write_rig("root", json!("spine"));
    tab.f.world.insert(SaveRequested, tab.document);
    tab.f.frame();
    tab.f.frame();
    let saved = tab.f.status(tab.document);
    assert!(!saved.is_empty());
    assert!(!tab.f.unsaved(tab.document));
    assert_eq!(tab.state().preview.paused_at, None);

    tab.until("the clip never ended", |tab| {
        tab.state().preview.paused_at.is_some()
    });
    for _ in 0..5 {
        tab.f.frame();
    }

    assert_eq!(tab.f.status(tab.document), saved);
    assert!(!tab.transport().playing);
}

use super::*;
use crate::{
    preview::{ClipPlayback, ClipPreviewProgress},
    rig::preview::{RigPosing, RigSides},
    transport::{TransportRow, spawn_transport},
};
use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel};
use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
};
use concerto_ecs::{Changed, IntoSystem, ResMut, System, events::event_channel::EventChannel};
use concerto_editor::{
    EditorPlugin,
    asset_editor::{ActiveEditor, AssetEditorRegistry, SaveRequested},
    guard::DiscardEdits,
    hosted::{
        Hosted, HostedApp,
        testing::{SceneFixture, forwarding, plain_recipe},
    },
    inspected_asset::{ProjectTypes, inspected_asset_systems, install_inspected_listeners},
    inspector::{InspectedSnapshots, InspectorRegistry, WriteValue},
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{
    asset_server::AssetServer, asset_store::AssetStore, content::save_content_asset_into,
};
use concerto_mesh::skeleton::JointPose;
use concerto_ui::{interaction::UIClick, node::UINode, theme::UITheme};
use concerto_window::{input::MouseButton, plugin::WindowPlugin};
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
    let id = save_content_asset_into(asset, &root, address, assets).unwrap();
    assets.get(id).unwrap().clone()
}

pub(super) struct RigTab {
    pub(super) f: SceneFixture,
    pub(super) document: Entity,
    pub(super) open: AssetEntry,
    pub(super) open_skeleton: AssetEntry,
    pub(super) other: AssetEntry,
    pub(super) other_skeleton: AssetEntry,
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
        other_skeleton,
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

    pub(super) fn sent(&self) -> &SentStages {
        self.f
            .world
            .get_component_for_entity::<SentStages>(self.document)
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

    pub(super) fn transport(&self) -> &ClipTransport {
        self.f
            .world
            .get_component_for_entity::<ClipTransport>(self.document)
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

    pub(super) fn status(&self) -> String {
        self.f.status(self.document)
    }

    pub(super) fn sides(&self) -> &RigSides {
        self.f
            .scene_world(self.document)
            .get_resource::<RigSides>()
            .unwrap()
    }

    pub(super) fn posing(&self) -> &RigPosing {
        self.f
            .scene_world(self.document)
            .get_resource::<RigPosing>()
            .unwrap()
    }

    pub(super) fn playback(&self) -> &ClipPlayback {
        self.f
            .scene_world(self.document)
            .get_resource::<ClipPlayback>()
            .unwrap()
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

    pub(super) fn until(&mut self, what: &str, done: impl Fn(&RigTab) -> bool) {
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

    pub(super) fn with_other(mut self) -> Self {
        let other = hex(&self.other);
        self.write_preview("other", other);
        let (source, target) = (self.other_skeleton.id, self.open_skeleton.id);
        self.until("the preview never showed both rigs", |tab| {
            tab.sides().skeleton_of(RigSide::Source) == Some(source)
                && tab.sides().skeleton_of(RigSide::Target) == Some(target)
                && tab.posing().retargets()
        });
        self
    }

    pub(super) fn playing_the_walk(mut self) -> Self {
        let walk = hex(&self.walk);
        self.write_preview("clip", walk);
        self.write_preview("mode", json!("Clip"));
        self.until("the walk never played", |tab| {
            tab.posing().mode() == RigPreviewMode::Clip
                && tab.playback().clip.is_some()
                && tab.transport().time > 0.0
        });
        self
    }

    pub(super) fn committed(&self, target: &str) -> Vec<PropertyCommitted> {
        self.f
            .written::<Hosted<PropertyCommitted>>()
            .into_iter()
            .map(|hosted| hosted.event)
            .filter(|event| event.address.target.name() == target)
            .collect()
    }
}

#[test]
fn opening_a_rig_fills_the_rig_value_and_the_preview_value() {
    let mut tab = rig_tab();

    let names: Vec<String> = tab.values().iter().map(|value| value.name()).collect();
    assert_eq!(names, ["RetargetRig", "Preview"]);
    assert_eq!(*tab.rig(), rig_on(&tab.open_skeleton));
    assert_eq!(tab.state().preview, RigPreview::default());
    assert_eq!(tab.status(), "Pick the other rig");
    assert!(!tab.f.unsaved(tab.document));
}

#[test]
fn a_rig_edit_marks_the_tab_unsaved_and_a_preview_edit_does_not() {
    let mut tab = rig_tab();

    tab.write_preview("bones", json!(false));
    assert!(!tab.state().preview.bones);
    assert_eq!(tab.committed("Preview").len(), 1);
    assert!(tab.committed("Preview")[0].result.is_ok());
    assert!(!tab.f.unsaved(tab.document));

    tab.write_rig("root", json!("spine"));
    assert_eq!(tab.rig().root, "spine".into());
    assert!(tab.f.unsaved(tab.document));
}

#[test]
fn save_writes_the_rig() {
    let mut tab = rig_tab();
    tab.write_rig("root", json!("spine"));

    tab.f.world.insert(SaveRequested, tab.document);
    tab.f.frame();

    let bytes = std::fs::read(tab.f.project_root.join(OPEN_ADDRESS)).unwrap();
    let (header, saved) = decode_content_asset::<RetargetRig>(&bytes).unwrap();
    assert_eq!(header.asset_id, tab.open.id);
    assert_eq!(saved.root, "spine".into());
    assert!(!tab.f.unsaved(tab.document));
}

#[test]
fn discard_restores_the_rig() {
    let mut tab = rig_tab();
    tab.write_rig("root", json!("spine"));

    tab.f.world.trigger(DiscardEdits(tab.document));
    tab.f.frame();

    assert_eq!(*tab.rig(), rig_on(&tab.open_skeleton));
}

#[test]
fn the_other_rig_and_the_clip_can_be_picked_with_the_rig_tab_alone() {
    let mut tab = rig_tab();

    let (other, walk) = (hex(&tab.other), hex(&tab.walk));
    tab.write_preview("other", other);
    tab.write_preview("clip", walk);

    assert!(
        tab.committed("Preview")
            .iter()
            .all(|event| event.result.is_ok())
    );
    assert_eq!(tab.state().preview.other.id(), tab.other.id);
    assert_eq!(tab.state().preview.clip.id(), tab.walk.id);
}

#[test]
fn the_rig_plugin_registers_its_editor_and_lets_the_editor_load_rigs_their_skeletons_and_clips() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(EditorPlugin {
            project: None,
            decorated: false,
            recipe: plain_recipe,
        });
    let server = app.get_resource::<AssetServer>().unwrap();
    assert!(!server.is_registered::<RetargetRig>());
    assert!(!server.is_registered::<AnimationClip>());
    assert!(!server.is_registered::<Skeleton>());

    app.register_plugin(crate::transport::TransportPlugin)
        .register_plugin(RetargetRigEditorPlugin);

    let server = app.get_resource::<AssetServer>().unwrap();
    assert!(server.is_registered::<RetargetRig>());
    assert!(server.is_registered::<AnimationClip>());
    assert!(server.is_registered::<Skeleton>());
    let editors = app.get_resource::<AssetEditorRegistry>().unwrap();
    let kind = RetargetRig::name();
    assert!(editors.preview(kind).is_some());
    assert!(editors.bottom_host(kind));
    assert!(editors.creatable().any(|(creatable, _)| creatable == kind));
}

#[test]
fn any_finite_speed_above_zero_is_accepted_and_zero_or_below_is_refused() {
    let mut tab = rig_tab();

    tab.write_preview("speed", json!(1.5));
    assert_eq!(tab.state().preview.speed, 1.5);

    for refused in [0.0, -1.0] {
        tab.write_preview("speed", json!(refused));
        assert_eq!(tab.state().preview.speed, 1.5);
        assert!(tab.status().contains("Preview.speed"), "{}", tab.status());
    }
}

#[test]
fn two_resolving_rigs_give_the_preview_two_stages_two_sides_offsets_and_a_retarget() {
    let tab = rig_tab().with_other();

    let sides = tab.sides();
    let offsets = tab.sent().offsets.unwrap();
    assert!(offsets.source < 0.0 && offsets.target > 0.0);
    assert_eq!(offsets.source, -offsets.target);
    assert_eq!(sides.offset_of(RigSide::Source), offsets.source);
    assert_eq!(sides.offset_of(RigSide::Target), offsets.target);
    assert_eq!(sides.root_bone_of(RigSide::Source), Some(PELVIS));
    assert_eq!(sides.root_bone_of(RigSide::Target), Some(PELVIS));
    let source = sides.colors_of(RigSide::Source);
    let target = sides.colors_of(RigSide::Target);
    assert_eq!(source.len(), 4);
    assert_eq!(source[2], source[3]);
    assert_eq!(source[2], target[2]);
    assert_ne!(source[2], source[0]);
    assert_ne!(source[PELVIS], source[2]);
    assert_eq!(tab.status(), "");
    let facts = tab.state().facts;
    assert_eq!(facts.chains, Some(Share { count: 1, of: 1 }));
    assert!((facts.scale.unwrap() - 0.9).abs() < 1e-5);
    assert_eq!(tab.state().skeleton.as_ref().unwrap().names[2], "spine");
}

#[test]
fn a_second_run_with_no_stage_change_sends_no_stage() {
    let mut tab = rig_tab().with_other();
    let roots = [RigSide::Source, RigSide::Target].map(|side| tab.sides().root_of(side));

    tab.write_preview("mode", json!("Clip"));
    tab.until("the mode never reached the preview", |tab| {
        tab.posing().mode() == RigPreviewMode::Clip
    });

    let kept = [RigSide::Source, RigSide::Target].map(|side| tab.sides().root_of(side));
    assert_eq!(kept, roots);
}

#[test]
fn a_broken_chain_on_the_open_rig_reports_the_target_rig_and_drops_the_retarget() {
    let mut tab = rig_tab().with_other();

    tab.write_rig("chains.0.end", json!("tail"));

    assert!(tab.f.unsaved(tab.document));
    let status = tab.status();
    assert!(status.starts_with("Target rig: "), "{status}");
    assert!(status.contains("tail"), "{status}");
    tab.until("the retarget stayed", |tab| !tab.posing().retargets());
    assert_eq!(tab.state().facts.scale, None);
}

#[test]
fn swap_makes_the_open_rig_the_source() {
    let mut tab = rig_tab().with_other();

    tab.write_preview("open_rig_is", json!("Source"));

    let (open, other) = (tab.open_skeleton.id, tab.other_skeleton.id);
    tab.until("the sides never swapped", |tab| {
        tab.sides().skeleton_of(RigSide::Source) == Some(open)
            && tab.sides().skeleton_of(RigSide::Target) == Some(other)
            && tab.posing().retargets()
    });
}

#[test]
fn saved_survives_a_quiet_resolve_and_a_preview_edit_with_an_error_replaces_it() {
    let mut tab = rig_tab().with_other();
    tab.write_rig("root", json!("spine"));

    tab.f.world.insert(SaveRequested, tab.document);
    tab.f.frame();
    tab.f.frame();

    let saved = format!("Saved {OPEN_ADDRESS}");
    assert_eq!(tab.status(), saved);

    tab.write_preview("mode", json!("Clip"));

    assert_eq!(tab.status(), "Pick a clip");
}

#[test]
fn assets_written_on_the_other_rig_reads_it_again() {
    let mut tab = rig_tab();
    let broken = rig_on(&tab.other_skeleton).with_chain(RetargetChain::new("arm", "spine", "tail"));
    write(&mut tab.f, OTHER_ADDRESS, &broken);
    let other = hex(&tab.other);
    tab.write_preview("other", other);
    assert!(tab.status().starts_with("Source rig: "), "{}", tab.status());

    write(&mut tab.f, OTHER_ADDRESS, &rig_on(&tab.other_skeleton));
    let id = tab.other.id;
    tab.f
        .world
        .get_resource_mut::<EventChannel<AssetsWritten>>()
        .unwrap()
        .push_event(AssetsWritten { ids: vec![id] });
    tab.until("the other rig was never read again", |tab| {
        tab.posing().retargets()
    });

    assert_eq!(tab.status(), "");
}

pub(super) fn transport_row(tab: &mut RigTab) -> TransportRow {
    let document = tab.document;
    let host = tab.f.world.spawn(UINode::default());
    let mut spawn = (move |theme: Res<UITheme>, mut cmd: CommandQueue| {
        spawn_transport(&mut cmd.entity(host), document, &theme);
    })
    .into_system();
    spawn.initialize(&mut tab.f.world);
    spawn.run_and_apply((), &mut tab.f.world);
    let world = &mut tab.f.world;
    *world
        .query::<&TransportRow, ()>()
        .iter(world)
        .next()
        .unwrap()
}

pub(super) fn left_click() -> UIClick {
    UIClick {
        position: Default::default(),
        button: MouseButton::Left,
    }
}

#[test]
fn a_pause_from_the_widget_sets_paused_at_and_play_clears_it() {
    let mut tab = rig_tab().with_other().playing_the_walk();
    let play = transport_row(&mut tab).play;

    tab.f.world.trigger_on(play, left_click());
    tab.until("the pause never reached the preview value", |tab| {
        tab.state().preview.paused_at.is_some()
    });
    assert_eq!(tab.state().preview.paused_at, Some(tab.transport().time));

    tab.f.world.trigger_on(play, left_click());
    tab.until("play never cleared paused_at", |tab| {
        tab.state().preview.paused_at.is_none()
    });
    assert!(tab.transport().playing);
}

#[test]
fn setting_paused_at_pauses_and_seeks_the_preview() {
    let mut tab = rig_tab().with_other().playing_the_walk();

    tab.write_preview("paused_at", json!({"Some": {"0": 0.5}}));

    tab.until("the preview never paused at 0.5", |tab| {
        !tab.playback().playing && tab.playback().time == 0.5
    });
    for _ in 0..5 {
        tab.f.frame();
    }
    assert_eq!(tab.state().preview.paused_at, Some(0.5));
    assert!(!tab.transport().playing);
    assert_eq!(tab.transport().time, 0.5);
}

#[derive(Resource, Default)]
struct Writes(usize);

fn count_writes(changed: Query<Entity, Changed<RigDocument>>, mut writes: ResMut<Writes>) {
    writes.0 += changed.iter().count();
}

#[test]
fn nothing_writes_the_preview_value_while_the_clip_plays() {
    let mut tab = rig_tab().with_other().playing_the_walk();
    tab.f.world.insert_resource(Writes::default());
    tab.f.add_delivery(count_writes.into_boxed_system());
    tab.f.frame();
    tab.f.frame();
    tab.f.world.get_resource_mut::<Writes>().unwrap().0 = 0;
    let from = tab.transport().time;

    tab.until("the clip stopped moving", |tab| {
        tab.transport().time > from + 0.1 || tab.transport().time < from
    });

    assert_eq!(tab.f.world.get_resource::<Writes>().unwrap().0, 0);
    assert_eq!(tab.state().preview.paused_at, None);
}

#[test]
fn a_clip_that_does_not_loop_ends_paused_at_its_duration() {
    let mut tab = rig_tab().with_other().playing_the_walk();

    tab.write_preview("looping", json!(false));

    tab.until("the clip never ended", |tab| {
        tab.state().preview.paused_at.is_some()
    });
    assert_eq!(tab.state().preview.paused_at, Some(1.0));
    assert!(!tab.state().preview.looping);
}

fn edit_the_rig(tab: &mut RigTab, edit: usize) {
    let root = if edit % 2 == 0 { "spine" } else { "pelvis" };
    tab.write_rig("root", json!(root));
}

#[test]
fn a_pause_from_the_widget_holds_while_the_rig_is_edited_every_frame() {
    let mut tab = rig_tab().with_other().playing_the_walk();
    let play = transport_row(&mut tab).play;
    for edit in 0..5 {
        edit_the_rig(&mut tab, edit);
    }

    tab.f.world.trigger_on(play, left_click());
    for edit in 5..15 {
        edit_the_rig(&mut tab, edit);
    }

    assert!(tab.state().preview.paused_at.is_some());
    assert_eq!(tab.state().preview.paused_at, Some(tab.transport().time));
    assert!(!tab.transport().playing);
    assert!(!tab.playback().playing);
}

#[test]
fn a_clip_that_does_not_loop_stays_at_its_end_while_the_rig_is_edited_every_frame() {
    let mut tab = rig_tab().with_other().playing_the_walk();
    tab.write_preview("looping", json!(false));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut edit = 0;
    while tab.state().preview.paused_at.is_none() {
        assert!(std::time::Instant::now() < deadline, "the clip never ended");
        edit_the_rig(&mut tab, edit);
        edit += 1;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    for _ in 0..5 {
        edit_the_rig(&mut tab, edit);
        edit += 1;
    }

    assert_eq!(tab.state().preview.paused_at, Some(1.0));
    assert!(!tab.transport().playing);
    assert!(!tab.playback().playing);
    assert_eq!(tab.playback().time, 1.0);
}

#[test]
fn a_clip_that_does_not_loop_ending_keeps_saved() {
    let mut tab = rig_tab().with_other().playing_the_walk();
    tab.write_preview("speed", json!(0.25));
    tab.write_preview("looping", json!(false));
    tab.write_rig("root", json!("spine"));
    tab.f.world.insert(SaveRequested, tab.document);
    tab.f.frame();
    tab.f.frame();
    let saved = format!("Saved {OPEN_ADDRESS}");
    assert_eq!(tab.status(), saved);
    assert_eq!(tab.state().preview.paused_at, None);

    tab.until("the clip never ended", |tab| {
        tab.state().preview.paused_at.is_some()
    });
    for _ in 0..5 {
        tab.f.frame();
    }

    assert_eq!(tab.status(), saved);
    assert!(!tab.transport().playing);
}

#[test]
fn reopening_another_rig_keeps_the_preview_and_goes_back_to_the_start() {
    let mut tab = rig_tab().with_other().playing_the_walk();
    tab.write_preview("paused_at", json!({"Some": {"0": 0.5}}));
    tab.until("the preview never paused at 0.5", |tab| {
        !tab.playback().playing && tab.playback().time == 0.5
    });
    let kept = tab.state().preview.clone();
    let second = write(
        &mut tab.f,
        "content/hero/second.gasset",
        &rig_on(&tab.open_skeleton),
    );

    tab.f.request(&second);

    tab.until("the second rig never opened", |tab| {
        tab.f
            .document(tab.document)
            .current
            .as_ref()
            .is_some_and(|current| current.id == second.id)
    });
    tab.until("the preview never went back to the start", |tab| {
        tab.playback().time == 0.0 && tab.transport().time == 0.0
    });
    for _ in 0..5 {
        tab.f.frame();
    }
    assert_eq!(
        tab.state().preview,
        RigPreview {
            paused_at: Some(0.0),
            ..kept
        }
    );
    assert!(!tab.playback().playing);
    assert_eq!(tab.playback().time, 0.0);
}

#[test]
fn set_property_on_the_preview_mode_and_other_reaches_follow_rig_and_inspect_lists_both_values() {
    let mut tab = rig_tab();

    let other = hex(&tab.other);
    tab.write_preview("other", other);
    tab.write_preview("mode", json!("Clip"));

    assert_eq!(tab.status(), "Pick a clip");
    assert!(
        tab.f
            .world
            .get_component_for_entity::<TransportDisabled>(tab.document)
            .is_some()
    );
    assert_eq!(
        tab.sent().source.map(|stage| stage.skeleton),
        Some(tab.other_skeleton.id)
    );
    tab.f.world.trigger(RefreshInspector);
    let snapshots = tab.f.world.get_resource::<InspectedSnapshots>().unwrap();
    let names: Vec<&str> = snapshots
        .values
        .iter()
        .map(|value| value.name.as_str())
        .collect();
    assert_eq!(names, ["RetargetRig", "Preview"]);
    assert!(
        snapshots
            .values
            .iter()
            .all(|value| !value.properties.is_empty())
    );
}

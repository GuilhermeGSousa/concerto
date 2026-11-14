//! The editor of a retarget rig: the rig's inspector, a second rig and a clip picked beside it, and a preview of both characters side by side.
mod panels;
pub mod preview;

use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Context;
use concerto_animation::{
    clip::AnimationClip,
    retarget::{PoseRetarget, ReferencePose, ResolvedRig, RetargetChain, RetargetRig},
};
use concerto_app::{App, InspectedAsset, plugins::Plugin, schedule_groups::Update};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Ref, Res, Resource, World,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    query::change_detection::DetectChanges,
    reflect::AppTypeRegistry,
};
use concerto_editor::{
    asset_editor::{AssetEditor, AssetEditorAppExt, EditorDocument, PreviewSpec},
    fonts::{AssetIconsAppExt, glyph},
    hosted::Hosted,
    inspected_asset::{InspectedAssetEditor, InspectedAssetSystems, InspectedDocument},
    inspector::{DocumentValue, InspectedValues, PropertyTarget, RefreshInspector, TargetWorld},
    preview::PreviewSignals,
    project::{AssetsWritten, ProjectState},
    scene::load_scene,
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{decode_content_asset, read_asset_record},
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::skeleton::Skeleton;
use concerto_name::Name;
use concerto_reflect::{PropertyPath, Reflect, ReflectDefault};
use concerto_scene::scene::Scene;

use panels::{build_rig_panels, sync_rig_panels};
pub use preview::RigPreviewMode;
use preview::{
    PlaceRigSides, RigSide, RigStage, SetPoseRetarget, SetRigPreview, SetRigSide, ShowRigStage,
    retarget_preview,
};

use crate::{
    ClipFacts, default_scene, measure_transport,
    preview::{ClearClip, SeekClip, SetClipTransport, ShowClip},
    transport::{
        ClipTransport, TransportDisabled, at_the_end, follow_transport_progress,
        sync_transport_rows, sync_transport_timelines,
    },
};

/// What a rig tab shows beside the open rig; never saved.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(Default)]
pub struct RigPreview {
    pub other: AssetHandle<RetargetRig>,
    pub clip: AssetHandle<AnimationClip>,
    pub open_rig_is: OpenRigSide,
    pub mode: RigPreviewMode,
    pub root_motion: bool,
    pub bones: bool,
    pub looping: bool,
    pub speed: f32,
    pub paused_at: Option<f32>,
}

impl Default for RigPreview {
    fn default() -> Self {
        Self {
            other: AssetHandle::default(),
            clip: AssetHandle::default(),
            open_rig_is: OpenRigSide::default(),
            mode: RigPreviewMode::default(),
            root_motion: false,
            bones: true,
            looping: true,
            speed: 1.0,
            paused_at: None,
        }
    }
}

/// Which side of the retarget the open rig plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum OpenRigSide {
    #[default]
    Target,
    Source,
}

impl OpenRigSide {
    fn side(self) -> RigSide {
        match self {
            OpenRigSide::Target => RigSide::Target,
            OpenRigSide::Source => RigSide::Source,
        }
    }
}

/// The registry of a rig tab's `Preview` value, apart from the game's.
#[derive(Resource, Clone)]
pub(crate) struct RigPreviewTypes(pub(crate) AppTypeRegistry);

pub(crate) fn preview_types() -> AppTypeRegistry {
    let types = AppTypeRegistry::default();
    {
        let mut registry = types.write();
        registry.register::<RigPreview>();
        registry.register::<OpenRigSide>();
        registry.register::<RigPreviewMode>();
        registry.register::<AssetHandle<RetargetRig>>();
        registry.register::<AssetHandle<AnimationClip>>();
        registry.register::<Option<f32>>();
        registry.register::<f32>();
        registry.register::<bool>();
    }
    types
}

fn playable_speed(speed: f32) -> bool {
    speed.is_finite() && speed > 0.0
}

/// A rig tab's [`RigPreview`], shown as the `Preview` value; an edit never marks the tab unsaved, and a speed of 0 or below, or not finite, is refused.
pub struct RigPreviewValue {
    pub document: Entity,
}

impl PropertyTarget for RigPreviewValue {
    fn name(&self) -> String {
        "Preview".to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity::<RigDocument>(self.document) else {
            return false;
        };
        read(&held.preview);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity::<RigDocument>(self.document) else {
            return false;
        };
        let mut preview = held.preview.clone();
        edit(&mut preview);
        if !playable_speed(preview.speed) {
            return false;
        }
        let changed = preview != held.preview;
        if changed
            && let Some(held) = world.get_component_for_entity_mut::<RigDocument>(self.document)
        {
            held.preview = preview;
        }
        true
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        world
            .get_resource::<RigPreviewTypes>()
            .map(|types| types.0.clone())
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }

    fn saved(&self) -> bool {
        false
    }
}

/// What a rig tab has read, by asset id: each read or why it failed; a read goes when `AssetsWritten` names it.
#[derive(Component, Default)]
pub(crate) struct RigReads {
    skeletons: HashMap<AssetId, Result<Arc<Skeleton>, String>>,
    clips: HashMap<AssetId, Result<Arc<AnimationClip>, String>>,
    rigs: HashMap<AssetId, Result<Arc<RetargetRig>, String>>,
    scenes: HashMap<AssetId, Result<Arc<Scene>, String>>,
}

struct ReadFrom<'a> {
    root: &'a Path,
    assets: &'a AssetRegistry,
}

fn read_asset<A: Asset>(from: &ReadFrom, id: AssetId, what: &str) -> anyhow::Result<A> {
    let entry = from
        .assets
        .get(id)
        .with_context(|| format!("the {what} is not in the project"))?;
    let bytes = read_asset_record(from.root, entry)
        .with_context(|| format!("cannot read {}", entry.location()))?;
    let (header, value) = decode_content_asset::<A>(&bytes)?;
    anyhow::ensure!(
        header.asset_id == id,
        "{} holds another asset's id",
        entry.address
    );
    Ok(value)
}

fn cached<T: Clone>(
    reads: &mut HashMap<AssetId, Result<T, String>>,
    id: AssetId,
    read: impl FnOnce() -> anyhow::Result<T>,
) -> Result<T, String> {
    reads
        .entry(id)
        .or_insert_with(|| read().map_err(|error| format!("{error:#}")))
        .clone()
}

impl RigReads {
    fn holds_any(&self, ids: &[AssetId]) -> bool {
        ids.iter().any(|id| {
            self.skeletons.contains_key(id)
                || self.clips.contains_key(id)
                || self.rigs.contains_key(id)
                || self.scenes.contains_key(id)
        })
    }

    fn forget(&mut self, ids: &[AssetId]) {
        for id in ids {
            self.skeletons.remove(id);
            self.clips.remove(id);
            self.rigs.remove(id);
            self.scenes.remove(id);
        }
    }

    fn skeleton(&mut self, id: AssetId, from: &ReadFrom) -> Result<Arc<Skeleton>, String> {
        if id.is_nil() {
            return Err("the rig names no skeleton".to_owned());
        }
        cached(&mut self.skeletons, id, || {
            read_asset::<Skeleton>(from, id, "skeleton").map(Arc::new)
        })
    }

    fn clip(&mut self, id: AssetId, from: &ReadFrom) -> Result<Arc<AnimationClip>, String> {
        cached(&mut self.clips, id, || {
            read_asset::<AnimationClip>(from, id, "clip").map(Arc::new)
        })
    }

    fn rig(&mut self, id: AssetId, from: &ReadFrom) -> Result<Arc<RetargetRig>, String> {
        cached(&mut self.rigs, id, || {
            read_asset::<RetargetRig>(from, id, "rig").map(Arc::new)
        })
    }

    fn scene(&mut self, entry: &AssetEntry, from: &ReadFrom) -> Result<Arc<Scene>, String> {
        cached(&mut self.scenes, entry.id, || {
            load_scene(from.root.to_owned(), entry.address.clone(), entry.id).map(Arc::new)
        })
    }
}

/// The skeleton and scene a preview side was last given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SentStage {
    pub(crate) skeleton: AssetId,
    pub(crate) scene: Option<AssetId>,
}

/// Where along X each side of the preview stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SideOffsets {
    pub(crate) source: f32,
    pub(crate) target: f32,
}

/// What a rig tab last sent its preview: each side's stage, the offsets, and the clip.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub(crate) struct SentStages {
    pub(crate) source: Option<SentStage>,
    pub(crate) target: Option<SentStage>,
    pub(crate) offsets: Option<SideOffsets>,
    pub(crate) clip: Option<AssetId>,
}

impl SentStages {
    fn side_mut(&mut self, side: RigSide) -> &mut Option<SentStage> {
        match side {
            RigSide::Source => &mut self.source,
            RigSide::Target => &mut self.target,
        }
    }

    fn forget(&mut self, ids: &[AssetId]) {
        for side in [&mut self.source, &mut self.target] {
            let named = side.is_some_and(|sent| {
                ids.contains(&sent.skeleton) || sent.scene.is_some_and(|scene| ids.contains(&scene))
            });
            if named {
                *side = None;
            }
        }
        if self.clip.is_some_and(|clip| ids.contains(&clip)) {
            self.clip = None;
        }
    }
}

/// `count` of `of`, as a rig tab's facts read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Share {
    pub(crate) count: usize,
    pub(crate) of: usize,
}

/// What a rig tab reports about the retarget: the scale from source to target, the target chains with a source match, and the source bones the clip drives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RigFacts {
    pub(crate) scale: Option<f32>,
    pub(crate) chains: Option<Share>,
    pub(crate) driven: Option<Share>,
}

/// What a rig tab holds beside the rig: the preview's settings, the open rig's skeleton or why it can't be read, the rig it last followed, the status line it last wrote, and the facts.
#[derive(Component, Default)]
pub(crate) struct RigDocument {
    pub(crate) preview: RigPreview,
    pub(crate) skeleton: Option<Arc<Skeleton>>,
    pub(crate) skeleton_error: Option<String>,
    pub(crate) opened: Option<AssetId>,
    pub(crate) status_line: Option<String>,
    pub(crate) facts: RigFacts,
}

/// Opens a `RetargetRig` in a tab that inspects it beside a second rig and a clip, with both characters side by side in a preview.
#[derive(Default)]
pub struct RetargetRigEditor;

impl AssetEditor for RetargetRigEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let record = InspectedAsset::of::<RetargetRig>();
        InspectedAssetEditor { record }.build(editor);
        let document = editor.entity();
        let rig = DocumentValue::<InspectedDocument> {
            name: record.kind.to_owned(),
            document,
            read: Box::new(|held| held.value.as_deref()),
            edit: Box::new(|held| held.value.as_deref_mut()),
        };
        editor.insert((
            InspectedValues(vec![Arc::new(rig), Arc::new(RigPreviewValue { document })]),
            RigDocument::default(),
            RigReads::default(),
            SentStages::default(),
            ClipTransport::default(),
            TransportDisabled,
        ));
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: retarget_preview,
        })
    }

    fn bottom_host(&self) -> bool {
        true
    }
}

const BONE_GREY: Color = Color::srgba(0.6, 0.6, 0.6, 1.0);
const ROOT_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 1.0);
const MOTION_ROOT_COLOR: Color = Color::srgba(0.85, 0.95, 0.55, 1.0);
const PALETTE: [Color; 8] = [
    Color::srgba(0.90, 0.30, 0.30, 1.0),
    Color::srgba(0.30, 0.70, 0.95, 1.0),
    Color::srgba(0.40, 0.85, 0.40, 1.0),
    Color::srgba(0.95, 0.75, 0.25, 1.0),
    Color::srgba(0.75, 0.45, 0.95, 1.0),
    Color::srgba(0.25, 0.85, 0.80, 1.0),
    Color::srgba(0.95, 0.50, 0.20, 1.0),
    Color::srgba(0.95, 0.45, 0.75, 1.0),
];

struct ChainColor {
    name: Name,
    color: Color,
}

fn chain_colors(open: &RetargetRig, other: Option<&RetargetRig>) -> Vec<ChainColor> {
    let mut colors: Vec<ChainColor> = Vec::new();
    let others = other.into_iter().flat_map(|other| other.chains.iter());
    for chain in open.chains.iter().chain(others) {
        if colors.iter().all(|known| known.name != chain.name) {
            colors.push(ChainColor {
                name: chain.name,
                color: PALETTE[colors.len() % PALETTE.len()],
            });
        }
    }
    colors
}

fn chain_bones(skeleton: &Skeleton, chain: &RetargetChain) -> Vec<usize> {
    let Some(start) = skeleton.bone_index(chain.start.0) else {
        return Vec::new();
    };
    let Some(end) = skeleton.bone_index(chain.end.0) else {
        return Vec::new();
    };
    let mut bones = vec![end];
    let mut bone = end;
    while bone != start {
        let Some(parent) = skeleton.parents[bone] else {
            return Vec::new();
        };
        bone = usize::from(parent);
        bones.push(bone);
    }
    bones
}

fn bone_colors(rig: &RetargetRig, skeleton: &Skeleton, chains: &[ChainColor]) -> Vec<Color> {
    let mut colors = vec![BONE_GREY; skeleton.bone_count()];
    for chain in &rig.chains {
        let Some(color) = chains
            .iter()
            .find(|known| known.name == chain.name)
            .map(|known| known.color)
        else {
            continue;
        };
        for bone in chain_bones(skeleton, chain) {
            colors[bone] = color;
        }
    }
    if let Some(root) = skeleton.bone_index(rig.root.0) {
        colors[root] = ROOT_COLOR;
    }
    if let Some(motion_root) = rig
        .motion_root
        .and_then(|motion_root| skeleton.bone_index(motion_root.0))
    {
        colors[motion_root] = MOTION_ROOT_COLOR;
    }
    colors
}

struct ShownScene {
    id: AssetId,
    scene: Arc<Scene>,
}

struct SideView {
    rig: Arc<RetargetRig>,
    skeleton_id: AssetId,
    skeleton: Arc<Skeleton>,
    resolved: Result<ResolvedRig, String>,
    scene: Option<ShownScene>,
}

fn read_side(
    rig: Arc<RetargetRig>,
    reads: &mut RigReads,
    from: &ReadFrom,
) -> Result<SideView, String> {
    let skeleton_id = rig.skeleton.id();
    let skeleton = reads.skeleton(skeleton_id, from)?;
    let reference = match &rig.reference {
        ReferencePose::Rest => Ok(None),
        ReferencePose::ClipFrame { clip, .. } => reads.clip(clip.id(), from).map(Some),
    };
    let resolved = reference.and_then(|clip| {
        rig.resolve(&skeleton, clip.as_deref())
            .map_err(|error| error.to_string())
    });
    let scene = from
        .assets
        .get(skeleton_id)
        .and_then(|entry| default_scene(entry, from.assets))
        .and_then(|entry| {
            let scene = reads.scene(entry, from).ok()?;
            Some(ShownScene {
                id: entry.id,
                scene,
            })
        });
    Ok(SideView {
        rig,
        skeleton_id,
        skeleton,
        resolved,
        scene,
    })
}

fn side_label(side: RigSide) -> &'static str {
    match side {
        RigSide::Source => "Source rig",
        RigSide::Target => "Target rig",
    }
}

fn opposite(side: RigSide) -> RigSide {
    match side {
        RigSide::Source => RigSide::Target,
        RigSide::Target => RigSide::Source,
    }
}

struct ShownClip {
    id: AssetId,
    clip: Arc<AnimationClip>,
}

#[derive(Default)]
struct Sides {
    source: Option<SideView>,
    target: Option<SideView>,
}

impl Sides {
    fn side(&self, side: RigSide) -> Option<&SideView> {
        match side {
            RigSide::Source => self.source.as_ref(),
            RigSide::Target => self.target.as_ref(),
        }
    }

    fn side_mut(&mut self, side: RigSide) -> &mut Option<SideView> {
        match side {
            RigSide::Source => &mut self.source,
            RigSide::Target => &mut self.target,
        }
    }
}

#[derive(Default)]
struct SideParts {
    source: Option<String>,
    target: Option<String>,
}

impl SideParts {
    fn side_mut(&mut self, side: RigSide) -> &mut Option<String> {
        match side {
            RigSide::Source => &mut self.source,
            RigSide::Target => &mut self.target,
        }
    }
}

struct Followed {
    open_side: RigSide,
    skeleton_error: Option<String>,
    sides: Sides,
    retarget: Option<PoseRetarget>,
    clip: Option<ShownClip>,
    status: String,
    facts: RigFacts,
}

fn view_part(side: RigSide, view: &Result<SideView, String>) -> Option<String> {
    let error = match view {
        Ok(view) => view.resolved.as_ref().err()?,
        Err(error) => error,
    };
    Some(format!("{}: {error}", side_label(side)))
}

fn built_retarget(sides: &Sides) -> Option<Result<PoseRetarget, String>> {
    let source = sides.source.as_ref()?;
    let target = sides.target.as_ref()?;
    let source_rig = source.resolved.as_ref().ok()?;
    let target_rig = target.resolved.as_ref().ok()?;
    Some(
        PoseRetarget::new(source_rig, &source.skeleton, target_rig, &target.skeleton)
            .map_err(|error| format!("Retarget: {error}")),
    )
}

fn follow(
    open: &RetargetRig,
    preview: &RigPreview,
    reads: &mut RigReads,
    from: &ReadFrom,
) -> Followed {
    let open_side = preview.open_rig_is.side();
    let other_side = opposite(open_side);
    let mut parts = SideParts::default();
    let mut sides = Sides::default();

    let open_view = read_side(Arc::new(open.clone()), reads, from);
    *parts.side_mut(open_side) = view_part(open_side, &open_view);
    let skeleton_error = open_view
        .as_ref()
        .err()
        .filter(|_| !open.skeleton.id().is_nil())
        .cloned();
    *sides.side_mut(open_side) = open_view.ok();

    let other_id = preview.other.id();
    if other_id.is_nil() {
        *parts.side_mut(other_side) = Some("Pick the other rig".to_owned());
    } else {
        let other_view = reads
            .rig(other_id, from)
            .and_then(|other| read_side(other, reads, from));
        *parts.side_mut(other_side) = view_part(other_side, &other_view);
        *sides.side_mut(other_side) = other_view.ok();
    }

    let mut status: Vec<String> = [parts.source, parts.target].into_iter().flatten().collect();
    let retarget = match built_retarget(&sides) {
        Some(Ok(retarget)) => Some(retarget),
        Some(Err(error)) => {
            status.push(error);
            None
        }
        None => None,
    };

    let clip = if preview.mode == RigPreviewMode::Clip {
        let id = preview.clip.id();
        if id.is_nil() {
            status.push("Pick a clip".to_owned());
            None
        } else {
            match reads.clip(id, from) {
                Ok(clip) => Some(ShownClip { id, clip }),
                Err(error) => {
                    status.push(format!("Clip: {error}"));
                    None
                }
            }
        }
    } else {
        None
    };

    let driven = sides.source.as_ref().and_then(|source| {
        let shown = clip.as_ref()?;
        Some(Share {
            count: source
                .skeleton
                .bone_ids
                .iter()
                .filter(|&id| shown.clip.get_channels(id).is_some())
                .count(),
            of: source.skeleton.bone_count(),
        })
    });
    if driven.is_some_and(|driven| driven.count == 0) {
        status.push("The clip drives none of the source skeleton's bones".to_owned());
    }

    let chains = sides.source.as_ref().and_then(|source| {
        let target = sides.target.as_ref()?;
        Some(Share {
            count: target
                .rig
                .chains
                .iter()
                .filter(|chain| {
                    source
                        .rig
                        .chains
                        .iter()
                        .any(|known| known.name == chain.name)
                })
                .count(),
            of: target.rig.chains.len(),
        })
    });

    Followed {
        open_side,
        skeleton_error,
        facts: RigFacts {
            scale: retarget.as_ref().map(PoseRetarget::scale),
            chains,
            driven,
        },
        sides,
        retarget,
        clip,
        status: status.join(" · "),
    }
}

struct Footprint {
    width: f32,
    height: f32,
}

fn footprint(view: Option<&SideView>) -> Footprint {
    let Some(view) = view else {
        return Footprint {
            width: 0.0,
            height: 0.0,
        };
    };
    let pose = match &view.resolved {
        Ok(resolved) => &resolved.reference_local[..],
        Err(_) => &view.skeleton.rest_pose[..],
    };
    let xs: Vec<f32> = view
        .skeleton
        .forward_kinematics(pose)
        .map(|model| model.iter().map(|bone| bone.w_axis.x).collect())
        .unwrap_or_default();
    let min = xs.iter().copied().reduce(f32::min).unwrap_or(0.0);
    let max = xs.iter().copied().reduce(f32::max).unwrap_or(0.0);
    Footprint {
        width: max - min,
        height: view
            .resolved
            .as_ref()
            .map_or(0.0, |resolved| resolved.root_height),
    }
}

fn side_offsets(sides: &Sides) -> SideOffsets {
    let source = footprint(sides.source.as_ref());
    let target = footprint(sides.target.as_ref());
    let gap = (source.width + target.width) / 2.0 + 0.25 * source.height.max(target.height);
    SideOffsets {
        source: -gap / 2.0,
        target: gap / 2.0,
    }
}

struct Steered {
    transport: ClipTransport,
    send: bool,
    seek: Option<f32>,
}

fn steer_transport(
    transport: &ClipTransport,
    preview: &RigPreview,
    clip: Option<&ShownClip>,
    restarted: bool,
) -> Steered {
    let mut wanted = transport.clone();
    let facts = clip.map_or_else(ClipFacts::default, |shown| ClipFacts::of(&shown.clip));
    measure_transport(&mut wanted, &facts);
    if restarted {
        wanted.time = 0.0;
    }
    let mut send = false;
    let mut seek = None;
    if wanted.looping != preview.looping {
        wanted.looping = preview.looping;
        send = true;
    }
    if wanted.speed != preview.speed {
        wanted.speed = preview.speed;
        send = true;
    }
    match preview.paused_at {
        Some(time) => {
            if wanted.playing {
                wanted.playing = false;
                send = true;
            }
            if wanted.time != time {
                wanted.time = time;
                seek = Some(time);
            }
        }
        None if !wanted.playing => {
            if !wanted.looping && at_the_end(&wanted) {
                wanted.time = 0.0;
                seek = Some(0.0);
            }
            wanted.playing = true;
            send = true;
        }
        None => {}
    }
    Steered {
        transport: wanted,
        send,
        seek,
    }
}

fn send_sides(
    document: Entity,
    open: &RetargetRig,
    followed: &Followed,
    sent: &mut SentStages,
    previews: &mut PreviewSignals,
) {
    let offsets = side_offsets(&followed.sides);
    if sent.offsets != Some(offsets) {
        sent.offsets = Some(offsets);
        previews.trigger(
            document,
            PlaceRigSides {
                source: offsets.source,
                target: offsets.target,
            },
        );
    }
    let other = followed
        .sides
        .side(opposite(followed.open_side))
        .map(|view| view.rig.as_ref());
    let chains = chain_colors(open, other);
    for side in [RigSide::Source, RigSide::Target] {
        let view = followed.sides.side(side);
        let wanted = view.map(|view| SentStage {
            skeleton: view.skeleton_id,
            scene: view.scene.as_ref().map(|shown| shown.id),
        });
        let held = sent.side_mut(side);
        if *held != wanted {
            *held = wanted;
            let stage = view.map(|view| {
                let stage = RigStage::new(Arc::clone(&view.skeleton), view.skeleton_id);
                match &view.scene {
                    Some(shown) => stage.with_scene(Scene::clone(&shown.scene)),
                    None => stage,
                }
            });
            previews.trigger(document, ShowRigStage { side, stage });
        }
        let Some(view) = view else {
            continue;
        };
        let resolved = view.resolved.as_ref().ok();
        previews.trigger(
            document,
            SetRigSide {
                side,
                reference: resolved.map(|resolved| resolved.reference_local.clone()),
                root: resolved.map(|resolved| resolved.root),
                colors: bone_colors(&view.rig, &view.skeleton, &chains),
            },
        );
    }
}

fn same_skeleton(held: Option<&Arc<Skeleton>>, read: Option<&Arc<Skeleton>>) -> bool {
    match held {
        Some(held) => read.is_some_and(|read| Arc::ptr_eq(held, read)),
        None => read.is_none(),
    }
}

struct PreviewEdit {
    document: Entity,
    path: PropertyPath,
}

impl PreviewEdit {
    fn names(&self, field: &str) -> bool {
        self.path
            .get_depth(0)
            .is_none_or(|segment| segment == field)
    }
}

fn preview_edits(committed: &mut EventReader<Hosted<PropertyCommitted>>) -> Vec<PreviewEdit> {
    committed
        .read()
        .filter(|hosted| hosted.event.result.is_ok() && !hosted.event.address.target.saved())
        .map(|hosted| PreviewEdit {
            document: hosted.document,
            path: hosted.event.address.path.clone(),
        })
        .collect()
}

pub(crate) fn follow_rig(
    documents: Query<(
        Entity,
        &mut EditorDocument,
        Ref<InspectedDocument>,
        &mut RigDocument,
        &mut RigReads,
        &mut SentStages,
        &mut ClipTransport,
        Option<&TransportDisabled>,
    )>,
    mut written: EventReader<AssetsWritten>,
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
    mut cmd: CommandQueue,
) {
    let written: Vec<AssetId> = written
        .read()
        .flat_map(|written| written.ids.iter().copied())
        .collect();
    let previewed = preview_edits(&mut committed);
    let Some(root) = project
        .project
        .as_ref()
        .map(|project| project.root.as_path())
    else {
        return;
    };
    let from = ReadFrom {
        root,
        assets: &assets,
    };
    for (
        document,
        mut editor,
        inspected,
        mut state,
        mut reads,
        mut sent,
        mut transport,
        disabled,
    ) in documents.iter()
    {
        let edited = previewed.iter().any(|edit| edit.document == document);
        let forgot = reads.holds_any(&written);
        if !(edited || forgot || state.is_changed() || inspected.is_changed()) {
            continue;
        }
        let Some(open) = inspected
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<RetargetRig>())
        else {
            continue;
        };
        let reads = &mut **reads;
        let sent = &mut **sent;
        let current = editor.current.as_ref().map(|asset| asset.id);
        let reopened = state.opened != current;
        if reopened {
            state.opened = current;
            if state.preview.paused_at.is_some_and(|time| time != 0.0) {
                state.preview.paused_at = Some(0.0);
                cmd.trigger(RefreshInspector);
            }
        }
        if forgot {
            reads.forget(&written);
            sent.forget(&written);
        }
        let followed = follow(open, &state.preview, reads, &from);

        let open_skeleton = followed
            .sides
            .side(followed.open_side)
            .map(|view| Arc::clone(&view.skeleton));
        if !same_skeleton(state.skeleton.as_ref(), open_skeleton.as_ref()) {
            state.skeleton = open_skeleton;
        }
        if state.skeleton_error != followed.skeleton_error {
            state.skeleton_error = followed.skeleton_error.clone();
        }

        send_sides(document, open, &followed, sent, &mut previews);
        previews.trigger(document, SetPoseRetarget(followed.retarget.clone()));
        previews.trigger(
            document,
            SetRigPreview {
                mode: state.preview.mode,
                root_motion: state.preview.root_motion,
                bones: state.preview.bones,
            },
        );

        let shown = followed.clip.as_ref().map(|shown| shown.id);
        let restarted = sent.clip != shown;
        if restarted {
            sent.clip = shown;
            match &followed.clip {
                Some(shown) => previews.trigger(document, ShowClip(Arc::clone(&shown.clip))),
                None => previews.trigger(document, ClearClip),
            }
        }
        let mut steered = steer_transport(
            &transport,
            &state.preview,
            followed.clip.as_ref(),
            restarted || reopened,
        );
        if reopened && steered.seek.is_none() {
            steered.seek = Some(0.0);
        }
        if steered.send {
            previews.trigger(
                document,
                SetClipTransport {
                    playing: steered.transport.playing,
                    looping: steered.transport.looping,
                    rate: steered.transport.speed,
                },
            );
        }
        if let Some(time) = steered.seek {
            previews.trigger(document, SeekClip(time));
        }
        if **transport != steered.transport {
            **transport = steered.transport;
        }

        let off = state.preview.mode == RigPreviewMode::Reference || followed.clip.is_none();
        match (off, disabled.is_some()) {
            (true, false) => cmd.insert(TransportDisabled, document),
            (false, true) => cmd.remove::<TransportDisabled>(document),
            _ => {}
        }

        if state.facts != followed.facts {
            state.facts = followed.facts;
        }
        let ours = editor.status.is_empty()
            || state.status_line.as_deref() == Some(editor.status.as_str());
        if ours || edited {
            if editor.status != followed.status {
                editor.status = followed.status.clone();
            }
            if state.status_line.as_deref() != Some(followed.status.as_str()) {
                state.status_line = Some(followed.status);
            }
        }
    }
}

pub(crate) fn sync_rig_transport(
    documents: Query<(Entity, &mut RigDocument, &ClipTransport)>,
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    mut cmd: CommandQueue,
) {
    let previewed = preview_edits(&mut committed);
    let mut wrote = false;
    for (document, mut state, transport) in documents.iter() {
        let edited = |field: &str| {
            previewed
                .iter()
                .any(|edit| edit.document == document && edit.names(field))
        };
        let paused_at = (!transport.playing).then_some(transport.time);
        if !edited("paused_at") && state.preview.paused_at != paused_at {
            state.preview.paused_at = paused_at;
            wrote = true;
        }
        if !edited("looping") && state.preview.looping != transport.looping {
            state.preview.looping = transport.looping;
            wrote = true;
        }
        if !edited("speed") && state.preview.speed != transport.speed {
            state.preview.speed = transport.speed;
            wrote = true;
        }
    }
    if wrote {
        cmd.trigger(RefreshInspector);
    }
}

/// Registers [`RetargetRigEditor`], lets the editor pick rigs and clips and load what a rig names, and runs the rig tab's systems; needs [`crate::transport::TransportPlugin`].
pub struct RetargetRigEditorPlugin;

impl Plugin for RetargetRigEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_icon(RetargetRig::name(), glyph::PERSON_ARMS_SPREAD);
        app.register_creatable_asset_editor::<RetargetRig, RetargetRigEditor>()
            .expect("the editor's App holds the asset-editor registry")
            .register_asset::<RetargetRig>()
            .register_asset::<Skeleton>()
            .register_derived_asset::<AnimationClip>()
            .insert_resource(RigPreviewTypes(preview_types()))
            .add_system(
                Update,
                (
                    sync_rig_transport,
                    follow_rig,
                    build_rig_panels,
                    sync_rig_panels,
                )
                    .chain()
                    .after(InspectedAssetSystems)
                    .after(follow_transport_progress)
                    .before(sync_transport_rows)
                    .before(sync_transport_timelines),
            );
    }
}

#[cfg(test)]
mod tests;

//! The tab of a `Ragdoll` asset: its parts as a tree, the selected part's form, buttons that act on the preview, and the preview beside them.
use std::{path::Path, sync::Arc};

use concerto_animation::clip::AnimationClip;
use concerto_app::{App, InspectedAsset, plugins::Plugin, schedule_groups::Update};
use concerto_color::Color;
use concerto_ecs::{
    Added, Component, Entity, IntoSystemConfig, Query, Ref, Res, Resource, With, World,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    reflect::AppTypeRegistry,
    signal::{
        On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_editor::{
        AssetEditor, AssetEditorAppExt, EditorDocument, EditorHosts, PreviewSpec, Unsaved,
    },
    fonts::{AssetIconsAppExt, glyph},
    inspected_asset::{InspectedAssetEditor, InspectedAssetSystems, InspectedDocument},
    inspector::{DocumentValue, InspectedValues, PropertyTarget, RefreshInspector, TargetWorld},
    name_menu::{NameMenu, close_menu, open_name_menu},
    preview::PreviewSignals,
    project::{AssetsWritten, ProjectState},
};
use concerto_foundation::assets::{Asset, AssetId, handle::AssetHandle, registry::AssetRegistry};
use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_ragdoll::Ragdoll;
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_scene::scene::Scene;
use concerto_ui::elements::{Label, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::UIClick,
    items::{UIItemSelected, UISelected},
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
    tree::{UITree, UITreeElement},
};
use concerto_window::input::MouseButton;
use glam::Vec3;
use taffy::FlexWrap;

use crate::{
    create::read_asset,
    parts::{
        PartEdit, PartRow, TrialAction, apply_edit, bones_without_part, part_rows, trial_report,
    },
    preview::{
        DropRagdoll, PlayRagdoll, PokeRagdoll, ResetRagdoll, SetRagdoll, ShowRagdollStage,
        ragdoll_preview,
    },
};

const DEFAULT_IMPULSE: Vec3 = Vec3::new(0.0, 0.0, 30.0);

/// What a ragdoll tab does to its preview and how long it tries the body for; never saved.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(Default)]
pub struct RagdollPreview {
    /// How high the lowest bone starts above the floor when the body is dropped, in metres.
    pub drop_height: f32,
    /// The bone of the part a poke hits.
    pub poke_bone: JointName,
    /// A poke's impulse in N·s.
    pub poke_impulse: Vec3,
    /// How long a drop or a poke is tried for, in seconds.
    pub seconds: f32,
    /// The clip Play plays on the character, whose ragdoll follows it.
    pub clip: AssetHandle<AnimationClip>,
}

impl Default for RagdollPreview {
    fn default() -> Self {
        Self {
            drop_height: 0.5,
            poke_bone: JointName::default(),
            poke_impulse: DEFAULT_IMPULSE,
            seconds: 5.0,
            clip: AssetHandle::default(),
        }
    }
}

#[derive(Resource, Clone)]
pub(crate) struct RagdollPreviewTypes(AppTypeRegistry);

pub(crate) fn preview_types() -> AppTypeRegistry {
    let types = AppTypeRegistry::default();
    {
        let mut registry = types.write();
        registry.register::<RagdollPreview>();
        registry.register::<JointName>();
        registry.register::<Vec3>();
        registry.register::<AssetHandle<AnimationClip>>();
        registry.register::<f32>();
    }
    types
}

struct RagdollPreviewValue {
    document: Entity,
}

impl PropertyTarget for RagdollPreviewValue {
    fn name(&self) -> String {
        "Preview".to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity::<RagdollDocument>(self.document) else {
            return false;
        };
        read(&held.preview);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        let Some(held) = world.get_component_for_entity_mut::<RagdollDocument>(self.document)
        else {
            return false;
        };
        let mut preview = held.preview.clone();
        edit(&mut preview);
        if preview != held.preview {
            held.preview = preview;
        }
        true
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        world
            .get_resource::<RagdollPreviewTypes>()
            .map(|types| types.0.clone())
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }

    fn saved(&self) -> bool {
        false
    }
}

struct SelectedPartValue {
    document: Entity,
    types: DocumentValue<InspectedDocument>,
}

impl SelectedPartValue {
    fn new(document: Entity) -> Self {
        Self {
            document,
            types: DocumentValue {
                name: "Part".to_owned(),
                document,
                read: Box::new(|held| held.value.as_deref()),
                edit: Box::new(|held| held.value.as_deref_mut()),
            },
        }
    }
}

impl PropertyTarget for SelectedPartValue {
    fn name(&self) -> String {
        "Part".to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let part = world
            .get_component_for_entity::<RagdollDocument>(self.document)
            .and_then(|state| state.selected)
            .and_then(|bone| {
                world
                    .get_component_for_entity::<InspectedDocument>(self.document)?
                    .value
                    .as_deref()?
                    .downcast_ref::<Ragdoll>()?
                    .part(bone.as_str())
            });
        let Some(part) = part else {
            return false;
        };
        read(part);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        let Some(bone) = world
            .get_component_for_entity::<RagdollDocument>(self.document)
            .and_then(|state| state.selected)
        else {
            return false;
        };
        let part = world
            .get_component_for_entity_mut::<InspectedDocument>(self.document)
            .and_then(|held| {
                held.value
                    .as_deref_mut()?
                    .downcast_mut::<Ragdoll>()?
                    .part_mut(bone.as_str())
            });
        let Some(part) = part else {
            return false;
        };
        edit(part);
        true
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        self.types.types(world)
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }
}

#[derive(Default)]
struct Sent {
    stage: Option<(AssetId, Option<AssetId>)>,
    ragdoll: Option<(AssetId, Ragdoll)>,
}

#[derive(Default)]
struct Reads {
    skeleton: Option<(AssetId, Result<Arc<Skeleton>, String>)>,
    scene: Option<(AssetId, Result<Option<(AssetId, Scene)>, String>)>,
}

/// What a ragdoll tab holds beside the open ragdoll: the selected part, the preview's settings, what the tab has read and sent to its preview, the last report, and what is wrong with the ragdoll.
#[derive(Component, Default)]
pub(crate) struct RagdollDocument {
    pub(crate) selected: Option<Name>,
    pub(crate) preview: RagdollPreview,
    reads: Reads,
    sent: Sent,
    skeleton: Option<Arc<Skeleton>>,
    report: String,
    problem: Option<String>,
    opened: Option<AssetId>,
}

/// Replaces the Ragdoll a tab holds and marks the tab unsaved; the part buttons and the `set_ragdoll_part` tool both send it.
pub(crate) struct StoreRagdoll {
    pub(crate) document: Entity,
    pub(crate) ragdoll: Ragdoll,
}
impl Signal for StoreRagdoll {}

pub(crate) fn store_ragdoll(
    on: On<StoreRagdoll>,
    mut documents: Query<&mut InspectedDocument>,
    mut cmd: CommandQueue,
) {
    let StoreRagdoll { document, ragdoll } = on.signal();
    let Some(mut held) = documents.get_entity_mut(*document) else {
        return;
    };
    held.value = Some(Box::new(ragdoll.clone()));
    cmd.insert(Unsaved, *document);
    cmd.trigger(RefreshInspector);
}

/// Edits the part on `bone` of the ragdoll in a tab as the Add part and Remove part buttons do.
pub(crate) struct EditRagdollPart {
    pub(crate) document: Entity,
    pub(crate) bone: String,
    pub(crate) edit: PartEdit,
}
impl Signal for EditRagdollPart {}

fn edit_ragdoll_part(
    on: On<EditRagdollPart>,
    mut documents: Query<(&InspectedDocument, &mut RagdollDocument)>,
    mut cmd: CommandQueue,
) {
    let signal = on.signal();
    let Some((inspected, mut state)) = documents.get_entity_mut(signal.document) else {
        return;
    };
    let (Some(mut ragdoll), Some(skeleton)) = (
        inspected
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<Ragdoll>())
            .cloned(),
        state.skeleton.clone(),
    ) else {
        return;
    };
    match apply_edit(&mut ragdoll, &skeleton, &signal.bone, &signal.edit) {
        Ok(done) => {
            if signal.edit.remove
                && state
                    .selected
                    .is_some_and(|bone| bone.as_str() == signal.bone)
            {
                state.selected = None;
            } else if !signal.edit.remove {
                state.selected = Some(Name::new(&signal.bone));
            }
            state.report = done;
            cmd.trigger(StoreRagdoll {
                document: signal.document,
                ragdoll,
            });
        }
        Err(message) => state.report = message,
    }
}

/// Selects the part on `bone` in a tab, which the form shows.
struct SelectRagdollPart {
    document: Entity,
    bone: Name,
}
impl Signal for SelectRagdollPart {}

fn select_ragdoll_part(
    on: On<SelectRagdollPart>,
    mut documents: Query<&mut RagdollDocument>,
    mut cmd: CommandQueue,
) {
    let signal = on.signal();
    if let Some(mut state) = documents.get_entity_mut(signal.document)
        && state.selected != Some(signal.bone)
    {
        state.selected = Some(signal.bone);
        cmd.trigger(RefreshInspector);
    }
}

/// What a button or tool does to a ragdoll tab's preview.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PreviewRequest {
    Drop {
        height: f32,
        seconds: f32,
    },
    Poke {
        bone: String,
        impulse: Vec3,
        seconds: f32,
    },
    Play {
        clip: AssetId,
    },
    Reset,
}

pub(crate) struct PreviewSubject<'a> {
    pub(crate) ragdoll: &'a Ragdoll,
    pub(crate) skeleton: &'a Skeleton,
    pub(crate) root: &'a Path,
    pub(crate) assets: &'a AssetRegistry,
}

pub(crate) fn perform(
    document: Entity,
    request: &PreviewRequest,
    subject: &PreviewSubject,
    previews: &mut PreviewSignals,
) -> Result<String, String> {
    match request {
        PreviewRequest::Drop { height, seconds } => {
            let report = trial_report(
                subject.ragdoll,
                subject.skeleton,
                &TrialAction::Drop { height: *height },
                *seconds,
            )?;
            previews.trigger(document, DropRagdoll { height: *height });
            Ok(report)
        }
        PreviewRequest::Poke {
            bone,
            impulse,
            seconds,
        } => {
            let report = trial_report(
                subject.ragdoll,
                subject.skeleton,
                &TrialAction::Poke {
                    bone: bone.clone(),
                    impulse: *impulse,
                },
                *seconds,
            )?;
            previews.trigger(
                document,
                PokeRagdoll {
                    bone: Name::new(bone),
                    impulse: *impulse,
                },
            );
            Ok(report)
        }
        PreviewRequest::Play { clip } => {
            if clip.is_nil() {
                return Err(
                    "Pick a clip: set clip on the Preview value, with set_property or the picker."
                        .to_owned(),
                );
            }
            let entry = subject
                .assets
                .get(*clip)
                .map(|entry| entry.address.clone())
                .ok_or_else(|| "The clip is not in the project.".to_owned())?;
            let loaded = read_asset::<AnimationClip>(subject.root, subject.assets, *clip, "clip")
                .map_err(|error| format!("{error:#}"))?;
            previews.trigger(document, PlayRagdoll(Arc::new(loaded)));
            Ok(format!(
                "Playing {entry}; the ragdoll follows it kinematically. Powered playback, where the body pulls itself toward the clip, does not exist yet."
            ))
        }
        PreviewRequest::Reset => {
            previews.trigger(document, ResetRagdoll);
            Ok("Back in the rest pose, following its animation.".to_owned())
        }
    }
}

/// Runs a [`PreviewRequest`] on a ragdoll tab's preview and keeps what it answered where the tab shows it; the buttons send it.
pub(crate) struct RunPreviewAction {
    pub(crate) document: Entity,
    pub(crate) request: PreviewRequest,
}
impl Signal for RunPreviewAction {}

fn run_preview_action(
    on: On<RunPreviewAction>,
    mut documents: Query<(&InspectedDocument, &mut RagdollDocument)>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
) {
    let signal = on.signal();
    let (Some((inspected, mut state)), Some(root)) = (
        documents.get_entity_mut(signal.document),
        project.project.as_ref().map(|open| open.root.as_path()),
    ) else {
        return;
    };
    let answer = match (
        inspected
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<Ragdoll>()),
        state.skeleton.clone(),
    ) {
        (Some(ragdoll), Some(skeleton)) => perform(
            signal.document,
            &signal.request,
            &PreviewSubject {
                ragdoll,
                skeleton: &skeleton,
                root,
                assets: &assets,
            },
            &mut previews,
        ),
        _ => Err("The ragdoll or its skeleton has not loaded.".to_owned()),
    };
    state.report = answer.unwrap_or_else(|message| message);
}

fn request_from(preview: &RagdollPreview, kind: ButtonKind) -> PreviewRequest {
    match kind {
        ButtonKind::Drop => PreviewRequest::Drop {
            height: preview.drop_height,
            seconds: preview.seconds,
        },
        ButtonKind::Poke => PreviewRequest::Poke {
            bone: preview.poke_bone.0.as_str().to_owned(),
            impulse: preview.poke_impulse,
            seconds: preview.seconds,
        },
        ButtonKind::Play => PreviewRequest::Play {
            clip: preview.clip.id(),
        },
        ButtonKind::Reset => PreviewRequest::Reset,
    }
}

#[derive(Clone, Copy)]
enum ButtonKind {
    Drop,
    Poke,
    Play,
    Reset,
}

fn preview_button(document: Entity, kind: ButtonKind) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&RagdollDocument>, mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(state) = documents.get_entity(document) else {
            return;
        };
        cmd.trigger(RunPreviewAction {
            document,
            request: request_from(&state.preview, kind),
        });
    })
    .into_listener()
}

fn add_part_menu(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           documents: Query<(&InspectedDocument, &RagdollDocument)>,
           mut menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some((inspected, state)) = documents.get_entity(document) else {
            return;
        };
        let (Some(ragdoll), Some(skeleton)) = (
            inspected
                .value
                .as_deref()
                .and_then(|value| value.downcast_ref::<Ragdoll>()),
            state.skeleton.as_ref(),
        ) else {
            return;
        };
        let bones: Vec<&str> = bones_without_part(ragdoll, skeleton)
            .into_iter()
            .map(|bone| bone.as_str())
            .collect();
        open_name_menu(
            on.entity(),
            &bones,
            None,
            |bone| add_part(document, bone.to_owned()),
            &mut menus,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn add_part(document: Entity, bone: String) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           mut menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&mut menus);
        cmd.trigger(EditRagdollPart {
            document,
            bone: bone.clone(),
            edit: PartEdit::default(),
        });
    })
    .into_listener()
}

fn remove_part(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&RagdollDocument>, mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(bone) = documents
            .get_entity(document)
            .and_then(|state| state.selected)
        else {
            return;
        };
        cmd.trigger(EditRagdollPart {
            document,
            bone: bone.as_str().to_owned(),
            edit: PartEdit {
                remove: true,
                ..PartEdit::default()
            },
        });
    })
    .into_listener()
}

fn select_part(document: Entity, bone: Name) -> Listener<UIItemSelected> {
    (move |_: On<UIItemSelected>, mut cmd: CommandQueue| {
        cmd.trigger(SelectRagdollPart { document, bone });
    })
    .into_listener()
}

/// Opens a `Ragdoll` in a tab with its parts as a tree, the selected part's form, and a preview of its skeleton's character with the parts drawn over it.
#[derive(Default)]
pub struct RagdollEditor;

impl AssetEditor for RagdollEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let record = InspectedAsset::of::<Ragdoll>();
        InspectedAssetEditor { record }.build(editor);
        let document = editor.entity();
        let profiles = DocumentValue::<InspectedDocument> {
            name: "Profiles".to_owned(),
            document,
            read: Box::new(|held| {
                let ragdoll = held.value.as_deref()?.downcast_ref::<Ragdoll>()?;
                Some(&ragdoll.profiles as &dyn Reflect)
            }),
            edit: Box::new(|held| {
                let ragdoll = held.value.as_deref_mut()?.downcast_mut::<Ragdoll>()?;
                Some(&mut ragdoll.profiles as &mut dyn Reflect)
            }),
        };
        editor.insert((
            InspectedValues(vec![
                Arc::new(SelectedPartValue::new(document)),
                Arc::new(RagdollPreviewValue { document }),
                Arc::new(profiles),
            ]),
            RagdollDocument::default(),
        ));
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: ragdoll_preview,
        })
    }

    fn bottom_host(&self) -> bool {
        true
    }
}

struct ReadFrom<'a> {
    root: &'a Path,
    assets: &'a AssetRegistry,
}

fn read_skeleton(reads: &mut Reads, id: AssetId, from: &ReadFrom) -> Result<Arc<Skeleton>, String> {
    if !reads.skeleton.as_ref().is_some_and(|(held, _)| *held == id) {
        let read = read_asset::<Skeleton>(from.root, from.assets, id, "skeleton")
            .map(Arc::new)
            .map_err(|error| format!("{error:#}"));
        reads.skeleton = Some((id, read));
    }
    reads
        .skeleton
        .as_ref()
        .map_or_else(|| Err(String::new()), |(_, read)| read.clone())
}

fn read_scene(reads: &mut Reads, skeleton: AssetId, from: &ReadFrom) -> Option<(AssetId, Scene)> {
    if !reads
        .scene
        .as_ref()
        .is_some_and(|(held, _)| *held == skeleton)
    {
        let entry = from.assets.get(skeleton);
        let read = concerto_animation_editor::read_shown_scene(from.root, entry, None, from.assets)
            .map_err(|error| format!("{error:#}"));
        reads.scene = Some((skeleton, read));
    }
    reads
        .scene
        .as_ref()
        .and_then(|(_, read)| read.as_ref().ok())
        .and_then(Clone::clone)
}

fn follow_ragdoll(
    mut documents: Query<(
        Entity,
        &EditorDocument,
        Ref<InspectedDocument>,
        &mut RagdollDocument,
    )>,
    mut written: EventReader<AssetsWritten>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
    mut cmd: CommandQueue,
) {
    let written: Vec<AssetId> = written
        .read()
        .flat_map(|written| written.ids.iter().copied())
        .collect();
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
    for (document, editor, inspected, mut state) in documents.iter_mut() {
        let Some(ragdoll) = inspected
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<Ragdoll>())
        else {
            continue;
        };
        let current = editor.current.as_ref().map(|entry| entry.id);
        let reads = &mut state.reads;
        if reads
            .skeleton
            .as_ref()
            .is_some_and(|(id, _)| written.contains(id))
        {
            reads.skeleton = None;
        }
        if reads.scene.as_ref().is_some_and(|(id, read)| {
            written.contains(id)
                || read
                    .as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .is_some_and(|(scene, _)| written.contains(scene))
        }) {
            reads.scene = None;
        }
        let skeleton_id = ragdoll.skeleton.id();
        let skeleton = if skeleton_id.is_nil() {
            Err("The ragdoll names no skeleton; set its skeleton in the Ragdoll value.".to_owned())
        } else {
            read_skeleton(&mut state.reads, skeleton_id, &from)
        };
        let reopened = state.opened != current;
        if reopened {
            state.opened = current;
            state.selected = None;
            state.report.clear();
            state.sent = Sent::default();
        }
        let problem = match &skeleton {
            Err(error) => Some(error.clone()),
            Ok(skeleton) => ragdoll
                .resolve(skeleton)
                .err()
                .map(|error| format!("The ragdoll does not fit its skeleton: {error}")),
        };
        if state.problem != problem {
            state.problem = problem;
        }
        let skeleton = skeleton.ok();
        let changed_skeleton = match (&state.skeleton, &skeleton) {
            (Some(held), Some(read)) => !Arc::ptr_eq(held, read),
            (None, None) => false,
            _ => true,
        };
        if changed_skeleton {
            state.skeleton = skeleton.clone();
        }
        if let Some(skeleton) = &skeleton {
            let valid = state
                .selected
                .is_some_and(|bone| ragdoll.part(bone.as_str()).is_some());
            if !valid {
                let first = part_rows(ragdoll, skeleton).first().map(|row| row.bone);
                if state.selected != first {
                    state.selected = first;
                    cmd.trigger(RefreshInspector);
                }
            }
        }
        let (Some(skeleton), Some(current)) = (skeleton, current) else {
            continue;
        };
        let scene = read_scene(&mut state.reads, skeleton_id, &from);
        let stage = (skeleton_id, scene.as_ref().map(|(id, _)| *id));
        if state.sent.stage != Some(stage) {
            state.sent.stage = Some(stage);
            state.sent.ragdoll = None;
            previews.trigger(
                document,
                ShowRagdollStage {
                    skeleton,
                    skeleton_id,
                    scene: scene.map(|(_, scene)| scene),
                },
            );
        }
        let stale = state
            .sent
            .ragdoll
            .as_ref()
            .is_none_or(|(id, sent)| *id != current || sent != ragdoll);
        if stale {
            state.sent.ragdoll = Some((current, ragdoll.clone()));
            previews.trigger(
                document,
                SetRagdoll {
                    id: current,
                    ragdoll: ragdoll.clone(),
                },
            );
        }
    }
}

const NO_PARTS: &str = "No parts";

/// The widgets of a ragdoll tab that follow its document, kept on the document.
#[derive(Component)]
pub(crate) struct RagdollPanels {
    tree: Entity,
    empty: Entity,
    report: Entity,
    problem: Entity,
    shown: Vec<PartRow>,
    rows: Vec<Entity>,
}

/// A row of a ragdoll tab's part tree.
#[derive(Component)]
pub(crate) struct PartRowTag {
    bone: Name,
    tag: Entity,
    text: String,
}

fn title(theme: &UITheme, text: &str) -> Label {
    theme
        .label(text.to_uppercase())
        .small()
        .muted()
        .height(UIValue::Px(24.0))
        .fixed()
}

fn build_panels(
    documents: Query<(Entity, &EditorHosts), (With<RagdollDocument>, Added<EditorHosts>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, hosts) in documents.iter() {
        let (Some(left), Some(inspector), Some(bottom)) =
            (hosts.left, hosts.inspector, hosts.bottom)
        else {
            continue;
        };
        let mut left_host = cmd.entity(left);
        let mut panel = left_host.spawn_child(theme.column().gap(theme.spacing_sm).grow());
        panel.add_child(title(&theme, "Parts"));
        let empty = panel
            .spawn_child(theme.label(NO_PARTS).muted().fixed())
            .entity();
        let tree = panel
            .spawn_child((
                UINode::default()
                    .with_height(UIValue::Px(0.0))
                    .with_flex_grow(1.0),
                UITree::default(),
            ))
            .entity();

        let mut inspector_host = cmd.entity(inspector);
        let mut actions = inspector_host.spawn_child(theme.column().gap(theme.spacing_sm).fixed());
        let mut parts_row = actions.spawn_child(
            theme
                .row()
                .gap(theme.spacing_sm)
                .node(|node| node.with_flex_wrap(FlexWrap::Wrap))
                .fixed(),
        );
        parts_row.add_child(theme.button("Add part").on_click(add_part_menu(document)));
        parts_row.add_child(theme.button("Remove part").on_click(remove_part(document)));
        let problem = actions
            .spawn_child(theme.label("").color(theme.error).hidden().fixed())
            .entity();

        let mut bottom_host = cmd.entity(bottom);
        let mut preview = bottom_host.spawn_child(
            theme
                .column()
                .gap(theme.spacing_sm)
                .padding(UIRect::all(theme.spacing_sm))
                .grow(),
        );
        preview.add_child(title(&theme, "Preview"));
        let mut preview_row = preview.spawn_child(
            theme
                .row()
                .gap(theme.spacing_sm)
                .node(|node| node.with_flex_wrap(FlexWrap::Wrap))
                .fixed(),
        );
        for (label, kind) in [
            ("Drop", ButtonKind::Drop),
            ("Poke", ButtonKind::Poke),
            ("Play", ButtonKind::Play),
            ("Reset", ButtonKind::Reset),
        ] {
            preview_row.add_child(theme.button(label).on_click(preview_button(document, kind)));
        }
        let report = preview.spawn_child(theme.label("").small().mono()).entity();
        cmd.insert(
            RagdollPanels {
                tree,
                empty,
                report,
                problem,
                shown: Vec::new(),
                rows: Vec::new(),
            },
            document,
        );
    }
}

fn tag_text(ragdoll: &Ragdoll, bone: Name) -> String {
    ragdoll
        .part(bone.as_str())
        .map_or_else(String::new, |part| {
            let shape = match part.shape {
                concerto_ragdoll::RagdollShape::Capsule { .. } => "capsule",
                concerto_ragdoll::RagdollShape::Box { .. } => "box",
                concerto_ragdoll::RagdollShape::Sphere { .. } => "sphere",
            };
            format!("{shape} · {:.1} kg", part.mass)
        })
}

fn spawn_part_rows(
    cmd: &mut CommandQueue,
    theme: &UITheme,
    document: Entity,
    tree: Entity,
    rows: &[PartRow],
    ragdoll: &Ragdoll,
) -> Vec<Entity> {
    let spawned: Vec<Entity> = rows
        .iter()
        .map(|row| {
            let tag = cmd
                .spawn(
                    theme
                        .label(tag_text(ragdoll, row.bone))
                        .small()
                        .muted()
                        .no_wrap()
                        .fixed()
                        .margin(UIRect::axes(0.0, 8.0)),
                )
                .entity();
            let label = cmd
                .spawn((
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_shrink(1.0)
                        .with_padding(UIRect::axes(0.0, 8.0)),
                    theme.text(row.bone.as_str()).single_line(),
                ))
                .entity();
            let item = cmd
                .spawn((
                    theme.canvas().fill(Color::TRANSPARENT).radius_sm(),
                    UITreeElement { expanded: true },
                    PartRowTag {
                        bone: row.bone,
                        tag,
                        text: tag_text(ragdoll, row.bone),
                    },
                    select_part(document, row.bone),
                ))
                .entity();
            cmd.add_child(item, label);
            cmd.add_child(item, tag);
            item
        })
        .collect();
    for (index, row) in rows.iter().enumerate() {
        let parent = row
            .parent
            .and_then(|parent| rows.iter().position(|other| other.bone == parent))
            .map_or(tree, |parent| spawned[parent]);
        cmd.add_child(parent, spawned[index]);
    }
    spawned
}

fn sync_panels(
    mut documents: Query<(
        Entity,
        Ref<InspectedDocument>,
        &RagdollDocument,
        &mut RagdollPanels,
    )>,
    mut tags: Query<(Entity, &mut PartRowTag, Option<&UISelected>)>,
    mut texts: Query<&mut UIText>,
    mut nodes: Query<&mut UINode>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, inspected, state, mut panels) in documents.iter_mut() {
        let ragdoll = inspected
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<Ragdoll>());
        let rows = match (ragdoll, &state.skeleton) {
            (Some(ragdoll), Some(skeleton)) => part_rows(ragdoll, skeleton),
            _ => Vec::new(),
        };
        if let Some(ragdoll) = ragdoll
            && panels.shown != rows
        {
            for row in std::mem::take(&mut panels.rows) {
                cmd.despawn(row);
            }
            let roots: Vec<PartRow> = rows.clone();
            panels.rows = spawn_part_rows(&mut cmd, &theme, document, panels.tree, &roots, ragdoll);
            panels.shown = rows;
        }
        let mut set = |entity: Entity, value: &str| {
            if let Some(mut text) = texts.get_entity_mut(entity)
                && text.text != value
            {
                text.text = value.to_owned();
            }
        };
        let mut line = |entity: Entity, value: &str| {
            set(entity, value);
            if let Some(mut node) = nodes.get_entity_mut(entity)
                && node.visible == value.is_empty()
            {
                node.visible = !value.is_empty();
            }
        };
        line(
            panels.empty,
            if panels.shown.is_empty() {
                NO_PARTS
            } else {
                ""
            },
        );
        line(panels.problem, state.problem.as_deref().unwrap_or(""));
        set(panels.report, &state.report);
        for (entity, mut row, selected) in tags.iter_mut() {
            if !panels.rows.contains(&entity) {
                continue;
            }
            if let Some(ragdoll) = ragdoll {
                let text = tag_text(ragdoll, row.bone);
                if row.text != text {
                    set(row.tag, &text);
                    row.text = text;
                }
            }
            let wanted = state.selected == Some(row.bone);
            if wanted && selected.is_none() {
                cmd.insert(UISelected, entity);
            } else if !wanted && selected.is_some() {
                cmd.remove::<UISelected>(entity);
            }
        }
        let _ = &inspected;
    }
}

/// Registers [`RagdollEditor`] for the `Ragdoll` asset and runs the tab's systems; the tools it brings are added by the crate's plugin.
pub(crate) struct RagdollTabPlugin;

impl Plugin for RagdollTabPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_icon(Ragdoll::name(), glyph::BONE);
        app.register_creatable_asset_editor::<Ragdoll, RagdollEditor>()
            .expect("the editor's App holds the asset-editor registry")
            .register_asset::<Ragdoll>()
            .register_asset::<Skeleton>()
            .register_derived_asset::<AnimationClip>()
            .register_type::<Ragdoll>()
            .insert_resource(RagdollPreviewTypes(preview_types()))
            .add_listener(store_ragdoll)
            .add_listener(edit_ragdoll_part)
            .add_listener(select_ragdoll_part)
            .add_listener(run_preview_action)
            .add_system(
                Update,
                (follow_ragdoll, build_panels, sync_panels)
                    .chain()
                    .after(InspectedAssetSystems),
            );
    }
}

use std::path::Path;

use concerto_ecs::{Entity, Query, Res, command::CommandQueue};
use concerto_editor::{
    asset_editor::{ActiveEditor, EditorDocument},
    inspected_asset::InspectedDocument,
    preview::PreviewSignals,
    project::ProjectState,
};
use concerto_foundation::assets::{Asset, AssetId, registry::AssetRegistry};
use concerto_mcp::{McpTool, ToolCalls, ToolError};
use concerto_mesh::Skeleton;
use concerto_ragdoll::Ragdoll;
use glam::Vec3;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    create::read_asset,
    parts::{PartEdit, ShapeKind, TrialAction, apply_edit, parts_text, trial_report},
    tab::{PreviewRequest, PreviewSubject, RagdollDocument, StoreRagdoll, perform},
};

const DEFAULT_DROP_HEIGHT: f32 = 0.5;
const DEFAULT_SECONDS: f32 = 5.0;
const DEFAULT_IMPULSE: Vec3 = Vec3::new(0.0, 0.0, 30.0);

/// Lists the parts of a Ragdoll asset, one line each under its parent part: bone, parent, shape and sizes, mass, and the joint's limits in radians. A tab that has the asset open answers with its current, possibly unsaved, value.
#[derive(Deserialize, JsonSchema)]
pub struct RagdollParts {
    /// The Ragdoll, by address or id, as list_assets shows it; the front tab's without one.
    pub asset: Option<String>,
}

/// Edits the part on one bone of the Ragdoll open in a tab, as its form does, and leaves the tab unsaved; save writes it. A bone with no part gets one that runs to its first child bone, with a loose ball joint: 45° of swing each way, 30° of twist each way, and a little friction. Sizes are in metres, angles in radians.
#[derive(Deserialize, JsonSchema)]
pub struct SetRagdollPart {
    /// The bone, by name as ragdoll_parts shows it.
    pub bone: String,
    /// The Ragdoll, by address or id; the front tab's without one. It must be open: open_asset opens it.
    pub asset: Option<String>,
    /// Removes the part on the bone; every other field is ignored.
    pub remove: Option<bool>,
    /// capsule, box or sphere; keeps the part's shape without it.
    pub shape: Option<String>,
    /// A capsule's or sphere's radius.
    pub radius: Option<f32>,
    /// Half the length of a capsule's cylinder.
    pub half_height: Option<f32>,
    /// A box's half sizes along x, y and z.
    pub half_extents: Option<[f32; 3]>,
    /// Mass in kg.
    pub mass: Option<f32>,
    /// Where the shape's centre is, in the bone's space.
    pub offset: Option<[f32; 3]>,
    /// Half the swing range around the axis perpendicular to the twist and plane axes.
    pub swing_normal: Option<f32>,
    /// Half the swing range around the plane axis.
    pub swing_plane: Option<f32>,
    /// Lowest twist angle, from -π to 0.
    pub twist_min: Option<f32>,
    /// Highest twist angle, from 0 to π.
    pub twist_max: Option<f32>,
    /// Friction torque of the joint while it is not driven, in N·m.
    pub friction: Option<f32>,
}

/// Tries a Ragdoll in a physics world of its own, with no preview and no GPU, on the tab's current, possibly unsaved, value, and answers whether the body came to rest and after how long, its peak speed, the widest its joints opened, and which pairs of parts interpenetrate in the rest pose, by bone with the depth in metres.
#[derive(Deserialize, JsonSchema)]
pub struct TestRagdoll {
    /// The Ragdoll, by address or id; the front tab's without one.
    pub asset: Option<String>,
    /// drop, which lets the body go above the floor, or poke, which hits one part; drop without it.
    pub action: Option<String>,
    /// For a drop, how high the lowest bone starts above the floor, in metres; 0.5 without it.
    pub height: Option<f32>,
    /// For a poke, the bone of the part to hit.
    pub bone: Option<String>,
    /// For a poke, the impulse in N·s along x, y and z; [0, 0, 30] without it.
    pub impulse: Option<[f32; 3]>,
    /// How long to simulate, in seconds; 5 without it.
    pub seconds: Option<f32>,
}

/// Does in the preview of the Ragdoll open in a tab what the tab's Drop, Poke, Play and Reset buttons do, and for a drop or a poke answers the same report as test_ragdoll. The preview must be built, which it is once the tab has been in front; what it shows is seen with screenshot target=viewport. Play follows the clip kinematically; powered playback does not exist yet.
#[derive(Deserialize, JsonSchema)]
pub struct PreviewRagdoll {
    /// drop, poke, play or reset.
    pub action: String,
    /// The Ragdoll, by address or id; the front tab's without one. It must be open: open_asset opens it.
    pub asset: Option<String>,
    /// For a drop, how high the lowest bone starts above the floor, in metres; the tab's Preview value without it.
    pub height: Option<f32>,
    /// For a poke, the bone of the part to hit; the tab's Preview value without it.
    pub bone: Option<String>,
    /// For a poke, the impulse in N·s along x, y and z; the tab's Preview value without it.
    pub impulse: Option<[f32; 3]>,
    /// For a drop or a poke, how long the report simulates, in seconds; the tab's Preview value without it.
    pub seconds: Option<f32>,
    /// For play, the AnimationClip, by address or id; the tab's Preview value without it.
    pub clip: Option<String>,
}

pub(crate) const PARTS: &str = "ragdoll_parts";
pub(crate) const SET_PART: &str = "set_ragdoll_part";
pub(crate) const TEST: &str = "test_ragdoll";
pub(crate) const PREVIEW: &str = "preview_ragdoll";

pub(crate) fn parts_tool() -> McpTool {
    McpTool::read_only(PARTS).needs(concerto_mcp::Needs::Project)
}

pub(crate) fn set_part_tool() -> McpTool {
    McpTool::new(SET_PART).needs(concerto_mcp::Needs::Project)
}

pub(crate) fn preview_tool() -> McpTool {
    McpTool::new(PREVIEW).needs(concerto_mcp::Needs::Project)
}

pub(crate) fn test_tool() -> McpTool {
    McpTool::read_only(TEST).needs(concerto_mcp::Needs::Project)
}

fn refused(code: &'static str, message: impl Into<String>) -> ToolError {
    ToolError::new(code, message)
}

struct Located {
    id: AssetId,
    address: String,
    document: Option<Entity>,
}

fn locate(
    asked: Option<&str>,
    active: &ActiveEditor,
    registry: &AssetRegistry,
    documents: &Query<(Entity, &EditorDocument, &InspectedDocument)>,
) -> Result<Located, ToolError> {
    let id = match asked {
        Some(asked) => registry
            .id_for_location(asked)
            .or_else(|| AssetId::from_simple_hex(asked).ok())
            .filter(|id| registry.get(*id).is_some())
            .ok_or_else(|| {
                refused(
                    "unknown_asset",
                    format!(
                        "No asset `{asked}` in the project; list_assets kind=Ragdoll shows the ragdolls."
                    ),
                )
            })?,
        None => active
            .0
            .and_then(|front| documents.get_entity(front))
            .filter(|(_, document, _)| document.asset_type == Ragdoll::name())
            .and_then(|(_, document, _)| document.current.as_ref().map(|entry| entry.id))
            .ok_or_else(|| {
                refused(
                    "no_ragdoll",
                    "No Ragdoll tab is in front; pass `asset` (list_assets kind=Ragdoll) or open one with open_asset.",
                )
            })?,
    };
    let entry = registry
        .get(id)
        .ok_or_else(|| refused("unknown_asset", "That asset is not in the project."))?;
    if entry.kind != Ragdoll::name() {
        return Err(refused(
            "not_a_ragdoll",
            format!(
                "{} is a {}, not a Ragdoll; list_assets kind=Ragdoll shows the ragdolls.",
                entry.address, entry.kind
            ),
        ));
    }
    let document = documents
        .iter()
        .find(|(_, document, _)| {
            document
                .current
                .as_ref()
                .is_some_and(|entry| entry.id == id)
        })
        .map(|(entity, _, _)| entity);
    Ok(Located {
        id,
        address: entry.address.clone(),
        document,
    })
}

pub(crate) struct Subject {
    pub(crate) ragdoll: Ragdoll,
    pub(crate) skeleton: Skeleton,
    pub(crate) note: &'static str,
}

fn read_subject(
    located: &Located,
    documents: &Query<(Entity, &EditorDocument, &InspectedDocument)>,
    root: &Path,
    registry: &AssetRegistry,
) -> Result<Subject, ToolError> {
    let open = located
        .document
        .and_then(|document| documents.get_entity(document))
        .and_then(|(_, _, inspected)| inspected.value.as_deref())
        .and_then(|value| value.downcast_ref::<Ragdoll>())
        .cloned();
    let (ragdoll, note) = match open {
        Some(ragdoll) => (ragdoll, " (the open tab's value)"),
        None => (
            read_asset::<Ragdoll>(root, registry, located.id, "ragdoll")
                .map_err(|error| refused("read_failed", format!("{error:#}")))?,
            "",
        ),
    };
    let skeleton = read_skeleton(&ragdoll, root, registry)?;
    Ok(Subject {
        ragdoll,
        skeleton,
        note,
    })
}

pub(crate) fn read_skeleton(
    ragdoll: &Ragdoll,
    root: &Path,
    registry: &AssetRegistry,
) -> Result<Skeleton, ToolError> {
    let skeleton = ragdoll.skeleton.id();
    if skeleton.is_nil() {
        return Err(refused(
            "no_skeleton",
            "The ragdoll names no skeleton; set its skeleton property with set_property on the open tab.",
        ));
    }
    read_asset::<Skeleton>(root, registry, skeleton, "skeleton")
        .map_err(|error| refused("read_failed", format!("{error:#}")))
}

fn vec3(value: [f32; 3]) -> Vec3 {
    Vec3::from_array(value)
}

pub(crate) fn ragdoll_parts(
    mut calls: ToolCalls<RagdollParts>,
    active: Res<ActiveEditor>,
    registry: Res<AssetRegistry>,
    project: Res<ProjectState>,
    documents: Query<(Entity, &EditorDocument, &InspectedDocument)>,
) {
    for (arguments, reply) in calls.read() {
        let Some(root) = project.project.as_ref().map(|open| open.root.as_path()) else {
            reply.err(refused("no_project", "Open a project first."));
            continue;
        };
        let answered = locate(arguments.asset.as_deref(), &active, &registry, &documents).and_then(
            |located| {
                let subject = read_subject(&located, &documents, root, &registry)?;
                Ok((located, subject))
            },
        );
        match answered {
            Ok((located, subject)) => {
                let mass: f32 = subject.ragdoll.parts.iter().map(|part| part.mass).sum();
                let mut text = format!(
                    "{}{} · {} parts, {mass:.1} kg",
                    located.address,
                    subject.note,
                    subject.ragdoll.parts.len()
                );
                for line in parts_text(&subject.ragdoll, &subject.skeleton) {
                    text.push('\n');
                    text.push_str(&line);
                }
                reply.ok(text);
            }
            Err(error) => reply.err(error),
        }
    }
}

pub(crate) fn test_ragdoll(
    mut calls: ToolCalls<TestRagdoll>,
    active: Res<ActiveEditor>,
    registry: Res<AssetRegistry>,
    project: Res<ProjectState>,
    documents: Query<(Entity, &EditorDocument, &InspectedDocument)>,
) {
    for (arguments, reply) in calls.read() {
        let Some(root) = project.project.as_ref().map(|open| open.root.as_path()) else {
            reply.err(refused("no_project", "Open a project first."));
            continue;
        };
        let action = match trial_action(&arguments) {
            Ok(action) => action,
            Err(message) => {
                reply.err(refused("invalid_arguments", message));
                continue;
            }
        };
        let seconds = arguments.seconds.unwrap_or(DEFAULT_SECONDS);
        let answered = locate(arguments.asset.as_deref(), &active, &registry, &documents).and_then(
            |located| {
                let subject = read_subject(&located, &documents, root, &registry)?;
                let report = trial_report(&subject.ragdoll, &subject.skeleton, &action, seconds)
                    .map_err(|message| refused("trial_failed", message))?;
                Ok(format!("{}{}: {report}", located.address, subject.note))
            },
        );
        match answered {
            Ok(text) => reply.ok(text),
            Err(error) => reply.err(error),
        }
    }
}

fn trial_action(arguments: &TestRagdoll) -> Result<TrialAction, String> {
    match arguments
        .action
        .as_deref()
        .unwrap_or("drop")
        .to_ascii_lowercase()
        .as_str()
    {
        "drop" => Ok(TrialAction::Drop {
            height: arguments.height.unwrap_or(DEFAULT_DROP_HEIGHT),
        }),
        "poke" => Ok(TrialAction::Poke {
            bone: arguments.bone.clone().ok_or_else(|| {
                "A poke needs `bone`, the part to hit; ragdoll_parts lists them.".to_owned()
            })?,
            impulse: arguments.impulse.map_or(DEFAULT_IMPULSE, vec3),
        }),
        other => Err(format!("`{other}` is not an action; use drop or poke.")),
    }
}

fn part_edit(arguments: &SetRagdollPart) -> Result<PartEdit, String> {
    Ok(PartEdit {
        remove: arguments.remove.unwrap_or(false),
        shape: arguments
            .shape
            .as_deref()
            .map(ShapeKind::parse)
            .transpose()?,
        radius: arguments.radius,
        half_height: arguments.half_height,
        half_extents: arguments.half_extents.map(vec3),
        mass: arguments.mass,
        offset: arguments.offset.map(vec3),
        swing_normal: arguments.swing_normal,
        swing_plane: arguments.swing_plane,
        twist_min: arguments.twist_min,
        twist_max: arguments.twist_max,
        friction: arguments.friction,
    })
}

pub(crate) fn set_ragdoll_part(
    mut calls: ToolCalls<SetRagdollPart>,
    active: Res<ActiveEditor>,
    registry: Res<AssetRegistry>,
    project: Res<ProjectState>,
    documents: Query<(Entity, &EditorDocument, &InspectedDocument)>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let Some(root) = project.project.as_ref().map(|open| open.root.as_path()) else {
            reply.err(refused("no_project", "Open a project first."));
            continue;
        };
        let edit = match part_edit(&arguments) {
            Ok(edit) => edit,
            Err(message) => {
                reply.err(refused("invalid_arguments", message));
                continue;
            }
        };
        let located = match locate(arguments.asset.as_deref(), &active, &registry, &documents) {
            Ok(located) => located,
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        let Some(document) = located.document else {
            reply.err(refused(
                "not_open",
                format!(
                    "{} is not open in a tab; open_asset opens it, then set_ragdoll_part edits it and save writes it.",
                    located.address
                ),
            ));
            continue;
        };
        let Some((_, _, inspected)) = documents.get_entity(document) else {
            reply.err(refused("not_open", "The tab is gone; open_asset opens it."));
            continue;
        };
        let Some(mut ragdoll) = inspected
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<Ragdoll>())
            .cloned()
        else {
            reply.err(refused(
                "still_loading",
                "The tab has not loaded; call status until it says busy: no, then try again.",
            ));
            continue;
        };
        let skeleton = match read_skeleton(&ragdoll, root, &registry) {
            Ok(skeleton) => skeleton,
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        match apply_edit(&mut ragdoll, &skeleton, &arguments.bone, &edit) {
            Ok(text) => {
                cmd.trigger(StoreRagdoll { document, ragdoll });
                reply.ok(format!(
                    "{text} {} is unsaved; save writes it.",
                    located.address
                ));
            }
            Err(message) => reply.err(refused("refused", message)),
        }
    }
}

pub(crate) fn preview_ragdoll(
    mut calls: ToolCalls<PreviewRagdoll>,
    active: Res<ActiveEditor>,
    registry: Res<AssetRegistry>,
    project: Res<ProjectState>,
    documents: Query<(Entity, &EditorDocument, &InspectedDocument)>,
    states: Query<&RagdollDocument>,
    mut previews: PreviewSignals,
) {
    for (arguments, reply) in calls.read() {
        let Some(root) = project.project.as_ref().map(|open| open.root.as_path()) else {
            reply.err(refused("no_project", "Open a project first."));
            continue;
        };
        let answered = locate(arguments.asset.as_deref(), &active, &registry, &documents).and_then(
            |located| {
                let document = located.document.ok_or_else(|| {
                    refused(
                        "not_open",
                        format!(
                            "{} is not open in a tab; open_asset opens it.",
                            located.address
                        ),
                    )
                })?;
                let subject = read_subject(&located, &documents, root, &registry)?;
                let settings = states
                    .get_entity(document)
                    .map(|state| state.preview.clone())
                    .unwrap_or_default();
                let request = preview_request(&arguments, &settings, &registry)
                    .map_err(|message| refused("invalid_arguments", message))?;
                let report = perform(
                    document,
                    &request,
                    &PreviewSubject {
                        ragdoll: &subject.ragdoll,
                        skeleton: &subject.skeleton,
                        root,
                        assets: &registry,
                    },
                    &mut previews,
                )
                .map_err(|message| refused("preview_failed", message))?;
                Ok(format!("{}: {report}", located.address))
            },
        );
        match answered {
            Ok(text) => reply.ok(text),
            Err(error) => reply.err(error),
        }
    }
}

fn preview_request(
    arguments: &PreviewRagdoll,
    settings: &crate::tab::RagdollPreview,
    registry: &AssetRegistry,
) -> Result<PreviewRequest, String> {
    let seconds = arguments.seconds.unwrap_or(settings.seconds);
    match arguments.action.to_ascii_lowercase().as_str() {
        "drop" => Ok(PreviewRequest::Drop {
            height: arguments.height.unwrap_or(settings.drop_height),
            seconds,
        }),
        "poke" => Ok(PreviewRequest::Poke {
            bone: arguments
                .bone
                .clone()
                .unwrap_or_else(|| settings.poke_bone.0.as_str().to_owned()),
            impulse: arguments.impulse.map_or(settings.poke_impulse, vec3),
            seconds,
        }),
        "play" => {
            let clip = match arguments.clip.as_deref() {
                Some(asked) => registry
                    .id_for_location(asked)
                    .or_else(|| AssetId::from_simple_hex(asked).ok())
                    .filter(|id| registry.get(*id).is_some())
                    .ok_or_else(|| {
                        format!(
                            "No clip `{asked}` in the project; list_assets kind=AnimationClip shows the clips."
                        )
                    })?,
                None => settings.clip.id(),
            };
            Ok(PreviewRequest::Play { clip })
        }
        "reset" => Ok(PreviewRequest::Reset),
        other => Err(format!(
            "`{other}` is not an action; use drop, poke, play or reset."
        )),
    }
}

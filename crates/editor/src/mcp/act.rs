//! Tools that change what the editor has open, selected or in view. Each goes
//! through the resource a panel would write, never around it.
use std::{path::PathBuf, time::Duration};

use concerto_app::App;
use concerto_ecs::{Entity, World};
use concerto_foundation::assets::AssetId;
use concerto_mcp::{Handled, McpApp, McpTool, Pending, ToolError, ToolOutput};
use glam::Vec3;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    LOAD_TIMEOUT, WorldIndex, entity_arg, find_asset, project, res_mut, when_project_settled,
};
use crate::{
    asset_editor::{AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry, EditorDocument},
    project::{EditorCommand, EditorCommands, ProjectState},
    scene::SceneRoot,
    selection::Selection,
    viewport::{FlyCamera, ViewportCommands},
};

/// The fly camera's pitch limit; straight up or down is a singularity.
const MAX_PITCH: f32 = 1.55;

pub(super) fn register(app: &mut App) {
    app.add_mcp_tool(McpTool::new(
        "open_project",
        "Opens a project directory (one holding imported `content/`), closing \
         every open editor, and waits until its catalogue is read. Relative \
         paths resolve against the editor's working directory.",
        open_project,
    ))
    .add_mcp_tool(McpTool::new(
        "open_asset",
        "Opens an asset in its editor and waits for it to load. A Scene opens \
         in the scene editor, replacing the scene shown there. Pass an address \
         or id from `list_assets`.",
        open_asset,
    ))
    .add_mcp_tool(McpTool::new(
        "close_editor",
        "Closes an asset editor by the id `status` lists for it, discarding \
         its temporary state.",
        close_editor,
    ))
    .add_mcp_tool(McpTool::new(
        "select",
        "Sets the editor's selection to an entity or a catalogue asset, or \
         clears it when given neither. In a windowed editor the hierarchy and \
         inspector follow; `frame` acts on it.",
        select,
    ))
    .add_mcp_tool(McpTool::new(
        "frame",
        "Moves the camera to fit an entity's meshes (selecting it), or the \
         whole scene when no entity is given, keeping the current viewing \
         angle. Returns the resulting camera pose.",
        frame,
    ))
    .add_mcp_tool(McpTool::new(
        "set_camera",
        "Places the editor camera at `position`, aimed at `look_at` or along \
         `yaw`/`pitch` (radians; yaw 0 looks down -Z, positive pitch looks up).",
        set_camera,
    ));
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenProject {
    /// The project directory, e.g. `examples/render-test`.
    path: String,
}

fn open_project(world: &mut World, args: OpenProject) -> Handled {
    when_project_settled(world, move |world| open_project_now(world, args))
}

fn open_project_now(world: &mut World, args: OpenProject) -> Handled {
    let generation = world
        .get_resource::<ProjectState>()
        .expect("ProjectState")
        .generation;
    let path = PathBuf::from(&args.path);
    res_mut::<EditorCommands>(world)
        .0
        .push_back(EditorCommand::OpenProject(path));
    Pending::new(LOAD_TIMEOUT, move |world| {
        let state = world.get_resource::<ProjectState>()?;
        if state.generation != generation {
            let project = state.project.as_ref()?;
            let mut kinds: Vec<(&str, usize)> = Vec::new();
            for asset in &project.assets {
                match kinds.iter_mut().find(|(kind, _)| *kind == asset.kind) {
                    Some((_, count)) => *count += 1,
                    None => kinds.push((&asset.kind, 1)),
                }
            }
            kinds.sort();
            let kinds = kinds
                .iter()
                .map(|(kind, count)| format!("{count} {kind}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Some(Ok(ToolOutput::text(format!(
                "Opened {} with {} assets: {kinds}.",
                project.root.display(),
                project.assets.len()
            ))));
        }
        if !state.busy() {
            return Some(Err(ToolError::new("open_failed", state.status.clone())));
        }
        None
    })
    .on_timeout("The project is still opening. `status` shows its progress.")
    .into()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenAsset {
    /// The asset's address (`content/levels/a.gasset`) or id.
    asset: String,
}

fn open_asset(world: &mut World, args: OpenAsset) -> Handled {
    when_project_settled(world, move |world| open_asset_now(world, args))
}

fn open_asset_now(world: &mut World, args: OpenAsset) -> Handled {
    let project = match project(world) {
        Ok(project) => project,
        Err(error) => return error.into(),
    };
    let asset = match find_asset(project, &args.asset) {
        Ok(asset) => asset.clone(),
        Err(error) => return error.into(),
    };
    let Some(editor_kind) = world
        .get_resource::<AssetEditorRegistry>()
        .and_then(|registry| registry.editor_kind(&asset.kind))
    else {
        return ToolError::new(
            "no_editor",
            format!(
                "{} is a {}, which has no editor. Scenes do.",
                asset.address, asset.kind
            ),
        )
        .into();
    };
    res_mut::<EditorCommands>(world)
        .0
        .push_back(EditorCommand::OpenAsset(asset.id));

    let mut first_poll = true;
    Pending::new(LOAD_TIMEOUT, move |world| {
        let document = world
            .query::<(Entity, &EditorDocument), ()>()
            .iter(world)
            .find(|(_, doc)| doc.asset_type == editor_kind)
            .map(|(entity, doc)| (entity, doc.clone()));
        let Some((editor, doc)) = document else {
            // The open command is processed in the frame it is queued, so a
            // missing editor on the next frame means it was dropped.
            if std::mem::take(&mut first_poll) {
                return None;
            }
            return Some(Err(ToolError::new(
                "open_failed",
                "The editor did not open the asset. `status` may say why.",
            )));
        };
        first_poll = false;
        if doc.pending.is_some() {
            return None;
        }
        if doc
            .current
            .as_ref()
            .is_some_and(|current| current.id == asset.id)
        {
            return Some(Ok(ToolOutput::text(opened(
                world,
                editor,
                &asset.address,
                asset.id,
            ))));
        }
        Some(Err(ToolError::new(
            "open_failed",
            if doc.status.is_empty() {
                format!("{} did not load.", asset.address)
            } else {
                doc.status
            },
        )))
    })
    .on_timeout(format!(
        "{} is still loading. `status` shows its progress.",
        args.asset
    ))
    .into()
}

/// A summary of what opening an asset produced.
fn opened(world: &mut World, editor: Entity, address: &str, id: AssetId) -> String {
    let root = world
        .query::<(Entity, &SceneRoot), ()>()
        .iter(world)
        .find(|(_, root)| root.asset_id == id)
        .map(|(entity, _)| entity);
    let mut text = format!("Opened {address} in editor {}.", editor.to_id_string());
    if let Some(root) = root {
        let index = WorldIndex::new(world);
        text.push_str(&format!(
            " Scene root {} with {} entities; it is selected. `scene_tree` shows them.",
            root.to_id_string(),
            index.subtree_size(root)
        ));
    }
    text
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CloseEditor {
    /// The editor's id, as `status` lists it.
    editor: String,
}

fn close_editor(world: &mut World, args: CloseEditor) -> Handled {
    let editor = match entity_arg(world, &args.editor) {
        Ok(editor) => editor,
        Err(error) => return error.into(),
    };
    if world
        .get_component_for_entity::<EditorDocument>(editor)
        .is_none()
    {
        return ToolError::new(
            "not_an_editor",
            format!(
                "{} is not an asset editor. `status` lists them.",
                args.editor
            ),
        )
        .into();
    }
    res_mut::<AssetEditorCommands>(world)
        .0
        .push_back(AssetEditorCommand::Close(editor));
    ToolOutput::text(format!("Closed editor {}.", args.editor)).into()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Select {
    /// An entity id to select.
    #[serde(default)]
    entity: Option<String>,
    /// A catalogue asset (address or id) to select.
    #[serde(default)]
    asset: Option<String>,
}

fn select(world: &mut World, args: Select) -> Handled {
    match (args.entity, args.asset) {
        (Some(_), Some(_)) => {
            ToolError::new("invalid_arguments", "Pass `entity` or `asset`, not both.").into()
        }
        (Some(id), None) => match entity_arg(world, &id) {
            Ok(entity) => {
                res_mut::<Selection>(world).select_entity(entity);
                ToolOutput::text(format!("Selected entity {id}.")).into()
            }
            Err(error) => error.into(),
        },
        (None, Some(query)) => when_project_settled(world, move |world| {
            let found = project(world)
                .and_then(|project| find_asset(project, &query))
                .map(|asset| (asset.id, asset.address.clone()));
            match found {
                Ok((id, address)) => {
                    res_mut::<Selection>(world).select_asset(id);
                    ToolOutput::text(format!("Selected asset {address}.")).into()
                }
                Err(error) => error.into(),
            }
        }),
        (None, None) => {
            res_mut::<Selection>(world).clear();
            ToolOutput::text("Selection cleared.").into()
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Frame {
    /// The entity to frame; it becomes the selection. Omit to frame everything.
    #[serde(default)]
    entity: Option<String>,
}

fn frame(world: &mut World, args: Frame) -> Handled {
    if let Some(id) = &args.entity {
        match entity_arg(world, id) {
            Ok(entity) => res_mut::<Selection>(world).select_entity(entity),
            Err(error) => return error.into(),
        }
    }
    let before = pose(world.get_resource::<FlyCamera>().expect("FlyCamera"));
    {
        let mut commands = res_mut::<ViewportCommands>(world);
        if args.entity.is_some() {
            commands.frame_selected = true;
        } else {
            commands.frame_all = true;
        }
    }
    // Framing runs in this frame's LateUpdate; the next poll sees its result.
    Pending::new(Duration::from_secs(5), move |world| {
        let after = pose(world.get_resource::<FlyCamera>()?);
        Some(if after == before {
            Err(ToolError::new(
                "nothing_to_frame",
                "No loaded mesh to fit. Meshes load asynchronously after a scene \
                 opens; if one just opened, try again.",
            ))
        } else {
            Ok(ToolOutput::text(format!("Framed. Camera {after}.")))
        })
    })
    .into()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetCamera {
    /// `[x, y, z]` in metres.
    position: [f32; 3],
    /// A point `[x, y, z]` to aim at. Overrides `yaw` and `pitch`.
    #[serde(default)]
    look_at: Option<[f32; 3]>,
    /// Radians around world up; 0 looks down -Z. Keeps the current yaw if omitted.
    #[serde(default)]
    yaw: Option<f32>,
    /// Radians; positive looks up. Keeps the current pitch if omitted.
    #[serde(default)]
    pitch: Option<f32>,
}

fn set_camera(world: &mut World, args: SetCamera) -> Handled {
    let position = Vec3::from(args.position);
    if !position.is_finite() {
        return ToolError::new("invalid_arguments", "`position` must be finite.").into();
    }
    let mut fly = res_mut::<FlyCamera>(world);
    fly.position = position;
    if let Some(target) = args.look_at {
        let forward = Vec3::from(target) - position;
        let Some(forward) = forward.try_normalize() else {
            return ToolError::new(
                "invalid_arguments",
                "`look_at` must differ from `position`.",
            )
            .into();
        };
        // Inverse of `rotation() * -Z` for a yaw-then-pitch camera.
        fly.yaw = (-forward.x).atan2(-forward.z);
        fly.pitch = forward.y.clamp(-1.0, 1.0).asin();
    } else {
        if let Some(yaw) = args.yaw {
            fly.yaw = yaw;
        }
        if let Some(pitch) = args.pitch {
            fly.pitch = pitch;
        }
    }
    fly.pitch = fly.pitch.clamp(-MAX_PITCH, MAX_PITCH);
    ToolOutput::text(format!("Camera {}.", pose(&fly))).into()
}

fn pose(fly: &FlyCamera) -> String {
    // `+ 0.0` turns a negative zero into a positive one, so it prints `0.000`.
    let p = fly.position + glam::Vec3::ZERO;
    format!(
        "at [{:.3}, {:.3}, {:.3}], yaw {:.3}, pitch {:.3}",
        p.x,
        p.y,
        p.z,
        fly.yaw + 0.0,
        fly.pitch + 0.0
    )
}

mod act;
mod edit;
mod observe;
mod run;

use std::path::PathBuf;

use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Query, Res, ResMut, Resource,
    command::CommandQueue,
    query::filter::With,
    signal::On,
    system::input::{StaticSystemInput, SystemInputData},
};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_mcp::{AppMcpExt, FrameDemand, McpPlugin, Needs, ToolError, host::HostClosing};

use crate::{
    asset_editor::{EditorDocument, SaveRequested},
    guard::{GuardChoice, UnsavedGuard},
    hosted::{EditWorld, PendingApp, PendingPlayApp, PlayState},
    project::ProjectState,
};

/// Builds the editor `App` that `--mcp` serves: the editor for `recipe` on `project`, with [`McpPlugin`] and [`EditorMcpPlugin`].
pub fn editor(recipe: fn(&mut App), project: PathBuf) -> impl FnOnce(&mut App) {
    move |app| {
        crate::run::register_editor(app, recipe, project, false);
        app.register_plugin(McpPlugin)
            .register_plugin(EditorMcpPlugin);
    }
}

/// The editor's part of serving MCP; register it after [`EditorPlugin`](crate::EditorPlugin) and [`McpPlugin`].
pub struct EditorMcpPlugin;

impl Plugin for EditorMcpPlugin {
    fn build(&self, app: &mut App) {
        app.describe_need(
            Needs::Project,
            ToolError::new("no_project", "Open a project first with open_project."),
        )
        .describe_need(
            Needs::Scene,
            ToolError::new("no_scene", "Open a scene first with open_asset."),
        )
        .describe_need(
            Needs::Playing,
            ToolError::new("not_playing", "Start the game with play."),
        )
        .add_system(LateUpdate, demand_frames_while_busy)
        .add_system(LateUpdate, demand_frames_while_the_scene_loads)
        .add_system(Update, discard_while_closing)
        .add_listener(quit_when_host_closes);
        observe::build(app);
        act::build(app);
        edit::build(app);
        run::build(app);
    }
}

#[derive(Resource)]
struct HostIsClosing;

type BusyInputs = (
    Res<'static, ProjectState>,
    Query<'static, 'static, &'static EditorDocument>,
    Query<'static, 'static, &'static PendingApp>,
    Query<'static, 'static, &'static PendingPlayApp>,
    Query<'static, 'static, &'static EditorDocument, With<SaveRequested>>,
    Query<'static, 'static, &'static PlayState>,
);

fn is_busy(
    (project, documents, building, starting, saving, states): &SystemInputData<BusyInputs>,
) -> bool {
    project.busy()
        || documents.iter().any(|document| document.pending.is_some())
        || building.iter().next().is_some()
        || starting.iter().next().is_some()
        || saving.iter().next().is_some()
        || states.iter().any(|state| state.is_on())
}

fn demand_frames_while_busy(busy: StaticSystemInput<BusyInputs>, mut demand: ResMut<FrameDemand>) {
    if is_busy(&busy) {
        demand.request();
    }
}

fn demand_frames_while_the_scene_loads(
    assets: EditWorld<Option<Res<AssetServer>>>,
    mut demand: ResMut<FrameDemand>,
) {
    if assets.as_deref().is_some_and(AssetServer::is_loading) {
        demand.request();
    }
}

fn quit_when_host_closes(
    _: On<HostClosing>,
    mut guard: ResMut<UnsavedGuard>,
    mut cmd: CommandQueue,
) {
    guard.request_quit();
    cmd.insert_resource(HostIsClosing);
}

fn discard_while_closing(closing: Option<Res<HostIsClosing>>, mut guard: ResMut<UnsavedGuard>) {
    if closing.is_some() && guard.asking() {
        guard.choose(GuardChoice::Discard);
    }
}

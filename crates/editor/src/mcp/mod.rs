mod act;
mod edit;
mod input;
mod layout;
mod observe;
mod run;
mod value;

pub use run::PlayTimeout;

use std::path::PathBuf;

use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, Res, ResMut, Resource,
    command::CommandQueue,
    query::filter::{Changed, Or, With, Without},
    signal::On,
    system::{
        NonSendMarker,
        input::{StaticSystemInput, SystemInputData},
    },
};
use concerto_foundation::assets::{asset_server::AssetServer, asset_store::AssetStore};
use concerto_mcp::{AppMcpExt, FrameDemand, McpPlugin, Needs, ToolError, host::HostClosing};
use concerto_scene::{
    link::{SceneLink, SceneLinkState},
    scene::Scene,
};
use concerto_ui::{sets::UiSet, text::UIText, text_input::UITextInput};

use crate::{
    asset_editor::{BackgroundWork, EditorDocument, SaveRequested},
    graph_canvas::CanvasReadout,
    guard::{GuardChoice, UnsavedGuard},
    hosted::{
        EditWorld, HostRole, HostedApp, PendingApp, PendingPlayApp, PlayState, ViewportTarget,
    },
    inspector::WriteRefusal,
    project::ProjectState,
    timeline::TimelineReadout,
};

impl From<WriteRefusal> for ToolError {
    fn from(refusal: WriteRefusal) -> Self {
        Self::new(refusal.code, refusal.message)
    }
}

/// Builds the editor `App` that `--mcp` serves: the editor for `recipe` on `project`, [`McpPlugin`], then `plugins`, which can add tools, and [`EditorMcpPlugin`].
pub fn editor(
    recipe: fn(&mut App),
    project: PathBuf,
    plugins: Vec<fn(&mut App)>,
) -> impl FnOnce(&mut App) {
    move |app| {
        crate::run::register_editor(app, recipe, project, false, &[]);
        app.register_plugin(McpPlugin);
        crate::run::apply_plugins(app, &plugins);
        app.register_plugin(EditorMcpPlugin);
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
        .insert_resource(PanelTextChanged(false))
        .add_system(
            LateUpdate,
            (
                note_text_changes.after(UiSet::PostLayout),
                demand_frames_while_busy,
            )
                .chain(),
        )
        .add_system(LateUpdate, demand_frames_while_the_scene_loads)
        .add_system(LateUpdate, demand_frames_while_links_wait)
        .add_system(Update, discard_while_closing)
        .add_listener(quit_when_host_closes);
        observe::build(app);
        act::build(app);
        layout::build(app);
        edit::build(app);
        value::build(app);
        run::build(app);
        input::build(app);
    }
}

#[derive(Resource)]
struct HostIsClosing;

#[derive(Resource)]
struct PanelTextChanged(bool);

type TextChanges = Or<(Changed<UIText>, Changed<UITextInput>)>;

type BusyInputs = (
    Res<'static, ProjectState>,
    Query<'static, 'static, &'static EditorDocument>,
    Query<'static, 'static, &'static PendingApp>,
    Query<'static, 'static, &'static PendingPlayApp>,
    Query<'static, 'static, &'static EditorDocument, With<SaveRequested>>,
    Query<'static, 'static, &'static PlayState>,
    Res<'static, PanelTextChanged>,
    Query<'static, 'static, &'static HostedApp, With<ViewportTarget>>,
    Query<'static, 'static, &'static BackgroundWork>,
    NonSendMarker,
);

fn is_busy(
    (project, documents, building, starting, saving, states, text, previews, work, _): &SystemInputData<
        BusyInputs,
    >,
) -> bool {
    text.0
        || project.busy()
        || documents.iter().any(|document| document.pending.is_some())
        || building.iter().next().is_some()
        || starting.iter().next().is_some()
        || saving.iter().next().is_some()
        || work.iter().next().is_some()
        || states.iter().any(|state| state.is_on())
        || front_preview_is_loading(previews)
}

fn note_text_changes(
    changes: Query<
        Entity,
        (
            TextChanges,
            Without<TimelineReadout>,
            Without<CanvasReadout>,
        ),
    >,
    mut changed: ResMut<PanelTextChanged>,
) {
    let now = changes.iter().next().is_some();
    if changed.0 != now {
        changed.0 = now;
    }
}

fn front_preview_is_loading(previews: &Query<&HostedApp, With<ViewportTarget>>) -> bool {
    previews.iter().any(|preview| {
        preview.role() == HostRole::Preview
            && preview
                .world()
                .get_resource::<AssetServer>()
                .is_some_and(|server| server.is_loading())
    })
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

fn demand_frames_while_links_wait(
    links: EditWorld<Query<(&SceneLink, &SceneLinkState)>>,
    scenes: EditWorld<Option<Res<AssetStore<Scene>>>>,
    mut demand: ResMut<FrameDemand>,
) {
    let Some(scenes) = scenes.as_deref() else {
        return;
    };
    if links.iter().any(|(link, state)| {
        *state == SceneLinkState::Waiting && scenes.get(link.scene()).is_some()
    }) {
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

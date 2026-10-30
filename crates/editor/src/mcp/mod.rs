mod act;
mod edit;
mod input;
mod observe;
mod run;

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
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_mcp::{AppMcpExt, FrameDemand, McpPlugin, Needs, ToolError, host::HostClosing};
use concerto_ui::{sets::UiSet, text::UIText, text_input::UITextInput};

use crate::{
    asset_editor::{EditorDocument, SaveRequested},
    guard::{GuardChoice, UnsavedGuard},
    hosted::{
        EditWorld, HostRole, HostedApp, PendingApp, PendingPlayApp, PlayState, ViewportTarget,
    },
    project::ProjectState,
    timeline::TimelineReadout,
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
        .add_system(Update, discard_while_closing)
        .add_listener(quit_when_host_closes);
        observe::build(app);
        act::build(app);
        edit::build(app);
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
    NonSendMarker,
);

fn is_busy(
    (project, documents, building, starting, saving, states, text, previews, _): &SystemInputData<
        BusyInputs,
    >,
) -> bool {
    text.0
        || project.busy()
        || documents.iter().any(|document| document.pending.is_some())
        || building.iter().next().is_some()
        || starting.iter().next().is_some()
        || saving.iter().next().is_some()
        || states.iter().any(|state| state.is_on())
        || front_preview_is_loading(previews)
}

fn note_text_changes(
    changes: Query<Entity, (TextChanges, Without<TimelineReadout>)>,
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

#[cfg(test)]
mod tests {
    use concerto_ecs::{IntoSystem, System, World};

    use super::*;
    use crate::hosted::testing::{plain_recipe, preview_now};
    use concerto_foundation::assets::{Asset, AssetId, LoadableAsset, asset_store::AssetStore};
    use serde::{Deserialize, Serialize};

    #[derive(Asset, Serialize, Deserialize)]
    struct Slow;
    impl LoadableAsset for Slow {}

    fn loading_recipe(app: &mut App) {
        plain_recipe(app);
        let mut server = AssetServer::new();
        server.register_asset(&AssetStore::<Slow>::new());
        std::mem::forget(server.load::<Slow>(AssetId::new()));
        app.insert_resource(server);
    }

    #[derive(Resource, Default)]
    struct Seen(bool);

    fn sees_a_loading_preview(preview: impl FnOnce(&mut World)) -> bool {
        let mut world = World::new();
        world.insert_resource(Seen::default());
        world.register_component::<ViewportTarget>();
        preview(&mut world);
        let mut system = (|previews: Query<&HostedApp, With<ViewportTarget>>,
                           mut seen: ResMut<Seen>| {
            seen.0 = front_preview_is_loading(&previews);
        })
        .into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);
        world.get_resource::<Seen>().unwrap().0
    }

    #[test]
    fn a_preview_in_front_that_is_still_loading_counts() {
        assert!(sees_a_loading_preview(|world| {
            world.spawn((preview_now(loading_recipe), ViewportTarget));
        }));
    }

    #[test]
    fn a_preview_behind_another_tab_is_frozen_and_does_not_count() {
        assert!(!sees_a_loading_preview(|world| {
            world.spawn(preview_now(loading_recipe));
        }));
    }

    fn notes_a_change(spawn: impl FnOnce(&mut World)) -> bool {
        let mut world = World::new();
        world.insert_resource(PanelTextChanged(false));
        world.register_component::<UIText>();
        world.register_component::<UITextInput>();
        world.register_component::<TimelineReadout>();
        spawn(&mut world);
        let mut system = note_text_changes.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);
        world.get_resource::<PanelTextChanged>().unwrap().0
    }

    #[test]
    fn a_panels_text_changing_keeps_the_editor_busy() {
        assert!(notes_a_change(|world| {
            world.spawn(UIText::default());
        }));
    }

    #[test]
    fn a_timeline_readout_counting_through_playback_does_not() {
        assert!(!notes_a_change(|world| {
            world.spawn((UIText::default(), TimelineReadout));
        }));
    }
}

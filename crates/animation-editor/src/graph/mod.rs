//! The asset editor of an [`AnimationGraph`]: its tab, working copy, preview and MCP tools.
mod copy;
mod document;
mod panels;
mod param_editor;
mod params;
mod preview;
mod save;
mod stage;
mod tools;

use concerto_animation::{graph::AnimationGraph, mask::BoneMask, node::IkTarget};
use concerto_app::{App, plugins::Plugin, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;
use concerto_editor::{asset_editor::AssetEditorAppExt, hosted::HostedEventsAppExt};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_procedural_animation::IgnoredBody;
use concerto_scene::scene::Scene;
use glam::{Vec2, Vec3};

pub use copy::{
    BuiltGraph, BuiltNode, Connected, GraphCopy, GraphLink, GraphNodeId, HeldNode, HeldValue,
};
pub use document::{
    AnimationGraphEditor, EditGraph, GraphChange, GraphDocument, GraphEdit, GraphEdited,
    GraphLevel, GraphSelection, HeldGraph, OpenTarget, ShownTransition, Sources, StateValue,
    TransitionRef, TransitionValue, node_types, read_graph, shown_transitions,
};
pub use param_editor::{ParamEditorAppExt, ParamNameEditor};
pub use params::{FoundParam, GraphParam, GraphParameters, find_params};
pub use preview::{
    GraphBytes, GraphPreviewState, PreviewNode, SetGraphParam, SetGraphPlayback, ShowGraphs,
    graph_preview,
};
pub use stage::{GraphLive, GraphPreview, LiveView, ParametersValue, PreviewValue, SentPreview};
pub use tools::{
    GraphAddNode, GraphAddState, GraphConnect, GraphInspect, GraphOpen, GraphRemove, GraphSelect,
    GraphSetParam, GraphUp,
};

use document::{
    apply_graph_edit, clear_status_on_edit, finish_state_creation, follow_value_edits, load_graph,
    restore_discarded_graph,
};
use panels::{build_graph_panels, name_new_states, sync_graph_panels};
use param_editor::{commit_param_fields, refresh_param_fields};
use save::save_graphs;
use stage::{follow_graph_state, sync_graph_preview};

/// Registers [`AnimationGraphEditor`] as a creatable asset editor, runs the graph tab's systems and adds its MCP tools.
pub struct AnimationGraphEditorPlugin;

impl Plugin for AnimationGraphEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset::<BoneMask>();
        app.register_param_editor::<bool>()
            .register_param_editor::<f32>()
            .register_param_editor::<u64>()
            .register_param_editor::<Vec2>()
            .register_param_editor::<Vec3>()
            .register_param_editor::<IkTarget>()
            .register_param_editor::<IgnoredBody>();
        if let Some(server) = app.get_resource::<AssetServer>() {
            server.name_asset::<AnimationGraph>();
            if !server.is_registered::<Scene>() {
                server.name_asset::<Scene>();
            }
        }
        app.register_creatable_asset_editor::<AnimationGraph, AnimationGraphEditor>()
            .expect("the editor's App holds the asset-editor registry")
            .register_event::<GraphEdited>()
            .forward_hosted_event::<GraphPreviewState>()
            .add_system(
                Update,
                (
                    follow_value_edits,
                    finish_state_creation,
                    save_graphs,
                    load_graph,
                    follow_graph_state,
                    sync_graph_preview,
                    build_graph_panels,
                    name_new_states,
                    sync_graph_panels,
                )
                    .chain(),
            )
            .add_system(Update, (commit_param_fields, refresh_param_fields).chain())
            .add_listener(apply_graph_edit)
            .add_listener(clear_status_on_edit)
            .add_listener(restore_discarded_graph);
        tools::build(app);
    }
}

#[cfg(test)]
mod tests;

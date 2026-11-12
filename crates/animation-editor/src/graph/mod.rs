//! The editor of an animation graph: its nodes on a canvas, wired and edited through signals, and saved as a fresh graph.
mod copy;
mod document;
mod panels;
mod save;
mod tools;

use concerto_animation::{graph::AnimationGraph, mask::BoneMask};
use concerto_app::{App, plugins::Plugin, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;
use concerto_editor::asset_editor::AssetEditorAppExt;
use concerto_foundation::assets::asset_server::AssetServer;

pub use copy::{Connected, GraphCopy, GraphLink, GraphNodeId, HeldNode, HeldValue};
pub use document::{
    AnimationGraphEditor, EditGraph, GraphChange, GraphDocument, GraphEdit, GraphEdited,
    GraphLevel, GraphSelection, HeldGraph, OpenTarget, ShownTransition, StateValue, TransitionRef,
    TransitionValue, graph_preview, node_types, read_graph, shown_transitions,
};
pub use tools::{
    GraphAddNode, GraphAddState, GraphConnect, GraphInspect, GraphOpen, GraphRemove, GraphSelect,
    GraphUp,
};

use document::{
    apply_graph_edit, clear_status_on_edit, finish_state_creation, follow_value_edits, load_graph,
    restore_discarded_graph,
};
use panels::{build_graph_panels, name_new_states, sync_graph_panels};
use save::save_graphs;

/// Registers [`AnimationGraphEditor`] as a creatable asset editor, runs the graph tab's systems and adds its MCP tools.
pub struct AnimationGraphEditorPlugin;

impl Plugin for AnimationGraphEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset::<BoneMask>();
        if let Some(server) = app.get_resource::<AssetServer>() {
            server.name_asset::<AnimationGraph>();
        }
        app.register_creatable_asset_editor::<AnimationGraph, AnimationGraphEditor>()
            .expect("the editor's App holds the asset-editor registry")
            .register_event::<GraphEdited>()
            .add_system(
                Update,
                (
                    follow_value_edits,
                    finish_state_creation,
                    save_graphs,
                    load_graph,
                    build_graph_panels,
                    name_new_states,
                    sync_graph_panels,
                )
                    .chain(),
            )
            .add_listener(apply_graph_edit)
            .add_listener(clear_status_on_edit)
            .add_listener(restore_discarded_graph);
        tools::build(app);
    }
}

#[cfg(test)]
mod tests;

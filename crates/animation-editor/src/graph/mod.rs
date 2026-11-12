//! The editor of an animation graph: its nodes on a canvas, wired and edited through signals, and saved as a fresh graph.
mod copy;
mod document;
mod panels;
mod save;
mod tools;

use concerto_animation::graph::AnimationGraph;
use concerto_app::{App, plugins::Plugin, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;
use concerto_editor::asset_editor::AssetEditorAppExt;

pub use copy::{Connected, GraphCopy, GraphLink, GraphNodeId, HeldNode, HeldValue};
pub use document::{
    AnimationGraphEditor, EditGraph, GraphChange, GraphDocument, GraphEdit, GraphEdited,
    GraphSelection, graph_preview, node_types, read_graph,
};
pub use tools::{GraphAddNode, GraphConnect, GraphInspect, GraphRemove, GraphSelect};

use document::{
    apply_graph_edit, clear_status_on_edit, follow_node_edits, load_graph, restore_discarded_graph,
};
use panels::{build_graph_panels, sync_graph_panels};
use save::save_graphs;

/// Registers [`AnimationGraphEditor`] as a creatable asset editor, runs the graph tab's systems and adds its MCP tools.
pub struct AnimationGraphEditorPlugin;

impl Plugin for AnimationGraphEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_creatable_asset_editor::<AnimationGraph, AnimationGraphEditor>()
            .expect("the editor's App holds the asset-editor registry")
            .register_event::<GraphEdited>()
            .add_system(
                Update,
                (
                    follow_node_edits,
                    save_graphs,
                    load_graph,
                    build_graph_panels,
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

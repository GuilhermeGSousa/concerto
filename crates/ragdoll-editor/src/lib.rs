//! The editor's way to generate a humanoid ragdoll from a retarget rig, as an extension of `concerto-editor`.
mod action;
mod create;

use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolRegistry};

pub use create::{ChainNames, CreateRagdoll, CreatedRagdoll, RagdollCreation};

/// Adds the Create ragdoll button to rig tabs, the `CreateRagdoll` signal behind it and the `create_ragdoll` tool.
pub struct RagdollEditorPlugin;

impl Plugin for RagdollEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_event::<RagdollCreation>()
            .add_listener(create::create_ragdoll)
            .add_system(Update, action::add_create_buttons);
        if app.get_resource::<ToolRegistry>().is_some() {
            app.add_tool::<create::CreateRagdollCall, _>(
                McpTool::new("create_ragdoll").needs(Needs::Project),
                create::create_ragdoll_call,
            )
            .add_system(LateUpdate, create::answer_creations);
        }
    }
}

/// Adds the ragdoll generator to the editor's `App`.
pub fn install(app: &mut App) {
    app.register_plugin(RagdollEditorPlugin);
}

//! The editor's tab and tools for a `Ragdoll`, and its generator from a retarget rig, as an extension of `concerto-editor`.
mod action;
mod create;
mod hit;
mod parts;
mod preview;
mod tab;
mod tools;

use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolRegistry};

pub use create::{ChainNames, CreateRagdoll, CreatedRagdoll, RagdollCreation};
pub use hit::HitCharacter;
pub use preview::{
    DropRagdoll, PlayRagdoll, PokeRagdoll, ResetRagdoll, SetRagdoll, ShowRagdollStage,
    ragdoll_preview,
};
pub use tab::{RagdollEditor, RagdollPreview};
pub use tools::{PreviewRagdoll, RagdollParts, SetRagdollPart, TestRagdoll};

/// Adds the `Ragdoll` tab and its tools, the Create ragdoll button on rig tabs, the `CreateRagdoll` signal behind it and the `create_ragdoll` tool.
pub struct RagdollEditorPlugin;

impl Plugin for RagdollEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_plugin(tab::RagdollTabPlugin);
        app.register_event::<RagdollCreation>()
            .add_listener(create::create_ragdoll)
            .add_system(Update, action::add_create_buttons);
        if app.get_resource::<ToolRegistry>().is_some() {
            app.add_tool::<create::CreateRagdollCall, _>(
                McpTool::new("create_ragdoll").needs(Needs::Project),
                create::create_ragdoll_call,
            )
            .add_system(LateUpdate, create::answer_creations)
            .add_tool::<HitCharacter, _>(hit::hit_tool(), hit::hit_character)
            .add_system(LateUpdate, hit::answer_hits)
            .add_tool::<tools::RagdollParts, _>(tools::parts_tool(), tools::ragdoll_parts)
            .add_tool::<tools::SetRagdollPart, _>(tools::set_part_tool(), tools::set_ragdoll_part)
            .add_tool::<tools::TestRagdoll, _>(tools::test_tool(), tools::test_ragdoll)
            .add_tool::<tools::PreviewRagdoll, _>(tools::preview_tool(), tools::preview_ragdoll);
        }
    }
}

/// Adds the ragdoll generator to the editor's `App`.
pub fn install(app: &mut App) {
    app.register_plugin(RagdollEditorPlugin);
}

//! An MCP server inside an [`App`]: tools are systems that answer calls from the client, in frames
//! or, for read-only tools, between them.

mod calls;
/// The headless host, which runs an `App` that draws into a texture instead of a window and serves
/// it over MCP.
pub mod host;
mod pending;
mod reply;
mod server;
mod tools;

use concerto_app::{
    App, Plugin,
    schedule_groups::{First, LateUpdate},
};
use concerto_ecs::{
    IntoSystemConfig, World,
    signal::{On, Signal},
    system::schedule::ScheduleLabel,
};

pub use calls::{
    McpCall, McpInbox, ToolCalls, answer_unanswered, receive_calls, receive_read_only_calls,
};
pub use pending::{
    FrameDemand, PendingCall, demand_frames, expire_pending_calls, reset_frame_demand,
};
pub use reply::{Reply, ToolError};
pub use server::ServerHandle;
pub use tools::{AppMcpExt, McpTool, Needs, ToolRegistry};

/// The schedule of tool systems in a frame, run from `First` right after [`receive_calls`].
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct McpServe;

/// The schedule of read-only tool systems between frames, run by a host through `App::serve`.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct McpRead;

struct ServeCalls;

impl Signal for ServeCalls {}

fn serve_calls(_: On<ServeCalls>, world: &mut World) {
    world.run_schedule(McpServe);
}

/// Receives MCP tool calls as [`McpCall`] events and answers them through tool systems.
pub struct McpPlugin;

impl Plugin for McpPlugin {
    fn build(&self, app: &mut App) {
        app.register_event::<McpCall>()
            .insert_resource(ToolRegistry::default())
            .insert_resource(McpInbox::new())
            .insert_resource(FrameDemand::default())
            .add_system(
                First,
                (
                    reset_frame_demand,
                    expire_pending_calls,
                    receive_calls
                        .after(reset_frame_demand)
                        .after(expire_pending_calls),
                ),
            )
            .add_system(LateUpdate, demand_frames)
            .add_system(McpServe, answer_unanswered)
            .add_system(
                McpRead,
                (
                    receive_read_only_calls,
                    answer_unanswered.after(receive_read_only_calls),
                ),
            )
            .add_listener(serve_calls);
    }
}

mod calls;
mod reply;
mod server;
mod tools;

use concerto_app::{App, Plugin, schedule_groups::First};
use concerto_ecs::{
    IntoSystemConfig, World,
    signal::{On, Signal},
    system::schedule::ScheduleLabel,
};

pub use calls::{
    McpCall, McpInbox, ToolCalls, answer_unanswered, receive_calls, receive_read_only_calls,
};
pub use reply::{Reply, ToolError};
pub use server::{ServerHandle, spawn_server};
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
            .add_system(First, receive_calls)
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

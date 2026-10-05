//! A runner for headless apps whose only input is MCP: frames run while there
//! is work, and the app sleeps on its inbox otherwise.
use std::time::{Duration, Instant};

use concerto_app::{App, Plugin, plugins::PluginsState, runner::AppExit};
use concerto_ecs::{Resource, World};

use crate::{
    McpInbox, McpTools, PendingRequests, ServerIdentity, serve_read_only, spawn_stdio_server,
};

/// The shortest frame while the app is busy but no call is queued, so a
/// background load does not spin a core.
const BUSY_FRAME: Duration = Duration::from_millis(16);

/// Set by app systems that have work in progress which needs more frames, for
/// example a load running on a worker thread. The runner clears it before each
/// frame and keeps running frames while something sets it.
#[derive(Resource, Default)]
pub struct McpActivity {
    busy: bool,
}

impl McpActivity {
    pub fn keep_awake(&mut self) {
        self.busy = true;
    }
}

/// Serves MCP over stdio and drives the app from it. The process exits when
/// the client closes stdin, so an agent never leaves the app running.
pub struct McpRunnerPlugin {
    pub identity: ServerIdentity,
}

impl Plugin for McpRunnerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(McpActivity::default());
        let identity = self.identity.clone();
        app.set_runner(move |mut app: App| {
            while app.plugin_state() != PluginsState::Ready {
                std::thread::yield_now();
            }
            app.finish_plugin_build();
            {
                let world = app.main_mut().world_mut();
                let tools = world
                    .get_resource::<McpTools>()
                    .expect("McpRunnerPlugin requires McpPlugin");
                let inbox = world
                    .get_resource::<McpInbox>()
                    .expect("McpRunnerPlugin requires McpPlugin");
                spawn_stdio_server(identity, tools, inbox);
            }
            run_until_closed(&mut app);
            AppExit::Success
        });
    }
}

/// Runs frames while a call needs one, a call is pending, or the app reports
/// activity, and blocks on the inbox otherwise. Read-only calls are answered
/// between frames. Returns when the client disconnects.
pub fn run_until_closed(app: &mut App) {
    // The first frame runs unconditionally: Startup, and whatever the app
    // started at launch, should not wait for the first call.
    let mut busy = false;
    let mut needs_frame = true;
    let mut last_frame = Instant::now();
    loop {
        if needs_frame {
            last_frame = Instant::now();
            app.update();
            busy = app
                .main_mut()
                .world_mut()
                .get_resource_mut::<McpActivity>()
                .is_some_and(|activity| std::mem::take(&mut activity.busy));
        }
        let world = app.main_mut().world_mut();
        let keep_running = busy || waiting(world);
        let inbox = world
            .get_resource_mut::<McpInbox>()
            .expect("run_until_closed requires McpPlugin");
        let open = if keep_running {
            inbox.wait_until(last_frame + BUSY_FRAME)
        } else {
            inbox.wait()
        };
        if !open {
            return;
        }
        serve_read_only(world);
        let queued = world
            .get_resource::<McpInbox>()
            .is_some_and(McpInbox::has_queued);
        needs_frame = keep_running || queued || waiting(world);
    }
}

fn waiting(world: &World) -> bool {
    world
        .get_resource::<PendingRequests>()
        .is_some_and(|pending| !pending.is_empty())
}

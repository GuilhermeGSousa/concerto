use std::{
    fmt,
    time::{Duration, Instant},
};

use concerto_app::{App, plugins::PluginsState};
use concerto_ecs::{Resource, signal::Signal};
use concerto_render::{
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    wgpu,
};
use concerto_window::plugin::{Window, WindowCommand};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{FrameDemand, McpInbox, McpRead, ToolRegistry, spawn_server};

const FRAME: Duration = Duration::from_micros(16_667);
const IDLE_CHECK: Duration = Duration::from_millis(100);
const CLOSING_GRACE: Duration = Duration::from_secs(5);

/// Triggered by the headless host when its client disconnects; the `App` then has five seconds to send `WindowCommand::Close`.
pub struct HostClosing;

impl Signal for HostClosing {}

/// The texture the headless host's `App` draws its window into.
#[derive(Resource, Clone)]
pub struct HostPicture(pub SharedTexture);

/// No GPU adapter exists, so the headless host has nothing to draw with.
#[derive(Debug)]
pub struct NoAdapter;

impl fmt::Display for NoAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "No GPU adapter is available. Without a GPU, install lavapipe, \
             the software Vulkan driver in the mesa-vulkan-drivers package.",
        )
    }
}

/// Serves the `App` that `build` makes over MCP on stdin and stdout, drawing into a texture of `size`; exits with status 1 without a GPU.
pub fn run(build: impl FnOnce(&mut App), size: (u32, u32)) {
    let io = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    if let Err(error) = run_with(io, build, size) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

/// Serves the `App` that `build` makes over MCP on `io` until the client disconnects or the `App` sends `WindowCommand::Close`.
pub fn run_with<IO>(io: IO, build: impl FnOnce(&mut App), size: (u32, u32)) -> Result<(), NoAdapter>
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let gpu = pollster::block_on(Gpu::request(&Gpu::instance(), None)).ok_or(NoAdapter)?;
    let picture = SharedTexture::new(wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut app = App::new();
    app.insert_resource(gpu)
        .insert_resource(DrawTarget::Texture(picture.clone()))
        .insert_resource(HostPicture(picture))
        .insert_resource(Window::new(size, 1.0));
    build(&mut app);
    while app.plugin_state() != PluginsState::Ready {
        std::thread::yield_now();
    }
    app.finish_plugin_build();
    let server = spawn_server(&app, io);

    let mut frames = Pacer::default();
    frames.run(&mut app);
    if closes(&mut app) {
        return Ok(());
    }
    while !server.is_finished() {
        let demanded = frame_demanded(&app);
        let timeout = if demanded { Duration::ZERO } else { IDLE_CHECK };
        let called = app
            .get_resource_mut::<McpInbox>()
            .expect("the headless host needs McpPlugin")
            .wait(timeout);
        if demanded || (called && queues_change(&app)) {
            frames.run(&mut app);
        } else if called {
            app.serve(McpRead);
        } else {
            continue;
        }
        if closes(&mut app) {
            return Ok(());
        }
    }

    app.main_mut().world_mut().trigger(HostClosing);
    let deadline = Instant::now() + CLOSING_GRACE;
    while !closes(&mut app) && Instant::now() < deadline {
        frames.run(&mut app);
    }
    Ok(())
}

#[derive(Default)]
struct Pacer {
    last: Option<Instant>,
}

impl Pacer {
    fn run(&mut self, app: &mut App) {
        if let Some(last) = self.last {
            let next = last + FRAME;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            }
        }
        self.last = Some(Instant::now());
        app.update();
    }
}

fn frame_demanded(app: &App) -> bool {
    app.get_resource::<FrameDemand>()
        .is_some_and(FrameDemand::is_requested)
}

fn queues_change(app: &App) -> bool {
    match (
        app.get_resource::<McpInbox>(),
        app.get_resource::<ToolRegistry>(),
    ) {
        (Some(inbox), Some(registry)) => inbox.queues_change(registry),
        _ => true,
    }
}

fn closes(app: &mut App) -> bool {
    app.get_resource_mut::<Window>()
        .is_some_and(|window| window.drain_commands().contains(&WindowCommand::Close))
}

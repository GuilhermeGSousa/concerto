//! A [Model Context Protocol](https://modelcontextprotocol.io) server inside an
//! app, so an agent can observe and drive it through tools.
//!
//! The protocol runs on its own thread. Every tool call becomes a request in
//! [`McpInbox`], and one exclusive system, [`serve_requests`], answers them at
//! the start of each frame with `&mut World`. Nothing else touches the world
//! from outside the main thread.
//!
//! Register tools with [`McpApp::add_mcp_tool`]. A tool that finishes at once
//! returns [`Handled::Done`]; one that waits on later frames (a load, a render)
//! returns [`Handled::Pending`] and is polled once per frame until it answers
//! or its deadline passes.
//!
//! The crate knows nothing about any particular app. The editor's tools live in
//! `concerto-editor`.
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    time::{Duration, Instant},
};

use concerto_app::{App, Plugin, schedule_groups::First};
use concerto_ecs::{Resource, World};
use crossbeam_channel::{Receiver, Sender};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

mod runner;
mod server;

pub use runner::{McpActivity, McpRunnerPlugin, run_until_closed};
pub use server::{ServerIdentity, spawn_server, spawn_stdio_server};

/// What a successful tool call returns: text the agent reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub text: String,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// Compact JSON, for values an agent will read and send back. Compact
    /// because pretty-printing puts every vector component on its own line.
    pub fn json(value: &Value) -> Self {
        Self::text(value.to_string())
    }
}

/// A failed tool call. It reaches the agent as a tool result with `isError`
/// set, not as a protocol error, so the agent sees it and can correct itself.
///
/// `code` is stable and machine-matchable; `message` says what to do next.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

pub type ToolResult = Result<ToolOutput, ToolError>;

type Poll = Box<dyn FnMut(&mut World) -> Option<ToolResult> + Send + Sync>;

/// A tool call that completes on a later frame.
pub struct Pending {
    deadline: Instant,
    timeout: String,
    poll: Poll,
}

impl Pending {
    /// `poll` runs once per frame, starting the frame after the call, until it
    /// returns `Some` or `timeout` elapses.
    pub fn new(
        timeout: Duration,
        poll: impl FnMut(&mut World) -> Option<ToolResult> + Send + Sync + 'static,
    ) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            timeout: format!("Still waiting after {}s.", timeout.as_secs_f32()),
            poll: Box::new(poll),
        }
    }

    /// Replaces the timeout message, so it can say where to look for progress.
    pub fn on_timeout(mut self, message: impl Into<String>) -> Self {
        self.timeout = message.into();
        self
    }
}

/// What a tool handler returns.
pub enum Handled {
    Done(ToolResult),
    Pending(Pending),
}

impl Handled {
    /// Waits until `ready` holds, then answers with `then` — which may itself
    /// wait. `then` runs at once if `ready` already holds. The whole call,
    /// waiting included, times out after `timeout`.
    pub fn after(
        world: &mut World,
        timeout: Duration,
        mut ready: impl FnMut(&World) -> bool + Send + Sync + 'static,
        then: impl FnOnce(&mut World) -> Handled + Send + Sync + 'static,
    ) -> Handled {
        if ready(world) {
            return then(world);
        }
        enum Stage<F> {
            Waiting(F),
            Running(Pending),
        }
        let mut stage = Some(Stage::Waiting(then));
        Pending::new(timeout, move |world| match stage.take()? {
            Stage::Waiting(then) if ready(world) => match then(world) {
                Handled::Done(result) => Some(result),
                // The inner call waits at least a frame, like any other.
                Handled::Pending(inner) => {
                    stage = Some(Stage::Running(inner));
                    None
                }
            },
            waiting @ Stage::Waiting(_) => {
                stage = Some(waiting);
                None
            }
            Stage::Running(mut inner) => {
                let result = (inner.poll)(world);
                if result.is_none() {
                    stage = Some(Stage::Running(inner));
                }
                result
            }
        })
        .into()
    }
}

impl From<ToolResult> for Handled {
    fn from(result: ToolResult) -> Self {
        Handled::Done(result)
    }
}

impl From<ToolOutput> for Handled {
    fn from(output: ToolOutput) -> Self {
        Handled::Done(Ok(output))
    }
}

impl From<ToolError> for Handled {
    fn from(error: ToolError) -> Self {
        Handled::Done(Err(error))
    }
}

impl From<Pending> for Handled {
    fn from(pending: Pending) -> Self {
        Handled::Pending(pending)
    }
}

type Call = Arc<dyn Fn(&mut World, Value) -> Handled + Send + Sync>;

/// One tool: its protocol description and its handler.
pub struct McpTool {
    descriptor: rmcp::model::Tool,
    call: Call,
    read_only: bool,
}

impl McpTool {
    /// A tool whose arguments deserialize into `A`. The input schema is
    /// generated from `A`, so its doc comments become the parameters'
    /// descriptions. Malformed arguments are rejected before `handler` runs.
    pub fn new<A, F>(name: &'static str, description: &'static str, handler: F) -> Self
    where
        A: DeserializeOwned + JsonSchema + 'static,
        F: Fn(&mut World, A) -> Handled + Send + Sync + 'static,
    {
        let descriptor =
            rmcp::model::Tool::new(name, description, Arc::new(serde_json::Map::new()))
                .with_input_schema::<A>();
        let call: Call = Arc::new(move |world: &mut World, arguments: Value| {
            match serde_json::from_value::<A>(arguments) {
                Ok(arguments) => handler(world, arguments),
                Err(error) => Handled::Done(Err(ToolError::new(
                    "invalid_arguments",
                    format!("{error}. The tool's input schema lists its parameters."),
                ))),
            }
        });
        Self {
            descriptor,
            call,
            read_only: false,
        }
    }

    /// Marks the tool as not changing anything. A client may then run it
    /// without asking, and a headless runner answers it between frames.
    pub fn read_only(mut self) -> Self {
        self.descriptor.annotations = Some(rmcp::model::ToolAnnotations::new().read_only(true));
        self.read_only = true;
        self
    }

    pub fn name(&self) -> &str {
        &self.descriptor.name
    }
}

/// Every registered tool, in registration order.
#[derive(Resource, Default)]
pub struct McpTools {
    tools: Vec<McpTool>,
}

impl McpTools {
    pub fn add(&mut self, tool: McpTool) {
        assert!(
            self.tools
                .iter()
                .all(|existing| existing.name() != tool.name()),
            "MCP tool `{}` is registered twice",
            tool.name()
        );
        self.tools.push(tool);
    }

    fn find(&self, name: &str) -> Option<&McpTool> {
        self.tools.iter().find(|tool| tool.name() == name)
    }

    pub(crate) fn descriptors(&self) -> Vec<rmcp::model::Tool> {
        self.tools
            .iter()
            .map(|tool| tool.descriptor.clone())
            .collect()
    }
}

pub trait McpApp {
    fn add_mcp_tool(&mut self, tool: McpTool) -> &mut Self;
}

impl McpApp for App {
    fn add_mcp_tool(&mut self, tool: McpTool) -> &mut Self {
        self.get_resource_mut::<McpTools>()
            .expect("register McpPlugin before adding MCP tools")
            .add(tool);
        self
    }
}

/// A tool call travelling from the protocol thread to the main thread.
pub(crate) struct McpRequest {
    pub tool: String,
    pub arguments: Value,
    pub reply: tokio::sync::oneshot::Sender<ToolResult>,
}

pub(crate) enum Message {
    Call(McpRequest),
    /// The client went away; the server thread is ending.
    Closed,
}

/// The main thread's end of the request channel.
#[derive(Resource)]
pub struct McpInbox {
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    queued: VecDeque<McpRequest>,
    closed: bool,
}

impl McpInbox {
    fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self {
            sender,
            receiver,
            queued: VecDeque::new(),
            closed: false,
        }
    }

    pub(crate) fn sender(&self) -> Sender<Message> {
        self.sender.clone()
    }

    fn accept(&mut self, message: Message) {
        match message {
            Message::Call(request) => self.queued.push_back(request),
            Message::Closed => self.closed = true,
        }
    }

    /// Moves everything already sent into the queue without waiting.
    fn collect(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            self.accept(message);
        }
    }

    /// Blocks until a message arrives, unless one is already queued. Returns
    /// `false` once the client has gone.
    pub(crate) fn wait(&mut self) -> bool {
        if self.queued.is_empty() && !self.closed {
            let message = self
                .receiver
                .recv()
                .expect("the inbox holds its own sender");
            self.accept(message);
        }
        self.collect();
        !self.closed
    }

    /// Like [`wait`](Self::wait), but gives up at `deadline`.
    pub(crate) fn wait_until(&mut self, deadline: Instant) -> bool {
        if self.queued.is_empty()
            && !self.closed
            && let Ok(message) = self.receiver.recv_deadline(deadline)
        {
            self.accept(message);
        }
        self.collect();
        !self.closed
    }

    pub fn has_queued(&self) -> bool {
        !self.queued.is_empty()
    }

    /// Whether the client has disconnected.
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

struct InFlight {
    tool: String,
    reply: tokio::sync::oneshot::Sender<ToolResult>,
    pending: Pending,
}

/// Tool calls waiting on later frames.
#[derive(Resource, Default)]
pub struct PendingRequests {
    in_flight: Vec<InFlight>,
}

impl PendingRequests {
    pub fn is_empty(&self) -> bool {
        self.in_flight.is_empty()
    }
}

/// Installs the inbox, the tool registry and [`serve_requests`]. Pair it with
/// [`McpRunnerPlugin`] for a headless app driven over stdio.
pub struct McpPlugin;

impl Plugin for McpPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(McpTools::default())
            .insert_resource(McpInbox::new())
            .insert_resource(PendingRequests::default())
            .add_system(First, serve_requests);
    }
}

/// Answers tool calls, at the start of the frame so that what a handler queues
/// is processed by this frame's systems.
///
/// Pending calls from earlier frames are polled first; new calls run after, so
/// a call that goes pending always waits at least one frame.
pub fn serve_requests(world: &mut World) {
    let Some(mut pending) = world.remove_resource::<PendingRequests>() else {
        return;
    };
    let now = Instant::now();
    let mut still_waiting = Vec::with_capacity(pending.in_flight.len());
    for mut call in pending.in_flight.drain(..) {
        if call.reply.is_closed() {
            continue;
        }
        let tool = call.tool.clone();
        match guarded(&tool, || (call.pending.poll)(world)) {
            Ok(Some(result)) => {
                let _ = call.reply.send(result);
            }
            Ok(None) if now >= call.pending.deadline => {
                let message = std::mem::take(&mut call.pending.timeout);
                let _ = call.reply.send(Err(ToolError::new("timeout", message)));
            }
            Ok(None) => still_waiting.push(call),
            Err(error) => {
                let _ = call.reply.send(Err(error));
            }
        }
    }
    pending.in_flight = still_waiting;

    let requests = match world.get_resource_mut::<McpInbox>() {
        Some(inbox) => {
            inbox.collect();
            std::mem::take(&mut inbox.queued)
        }
        None => VecDeque::new(),
    };
    for request in requests {
        pending.in_flight.extend(handle(world, request));
    }
    world.insert_resource(pending);
}

/// Answers read-only calls between frames, so looking at the app costs no
/// frame. Stops at the first call that may change something: that one, and
/// everything after it, waits for [`serve_requests`] inside a frame, where
/// change detection sees its writes and call order is kept.
pub(crate) fn serve_read_only(world: &mut World) {
    loop {
        let request = {
            let (Some(tools), Some(inbox)) = (
                world.get_resource::<McpTools>(),
                world.get_resource::<McpInbox>(),
            ) else {
                return;
            };
            let Some(front) = inbox.queued.front() else {
                return;
            };
            // An unknown tool changes nothing either; answer it now.
            if tools.find(&front.tool).is_some_and(|tool| !tool.read_only) {
                return;
            }
            world
                .get_resource_mut::<McpInbox>()
                .and_then(|inbox| inbox.queued.pop_front())
        };
        let Some(request) = request else {
            return;
        };
        if let Some(waiting) = handle(world, request) {
            world
                .get_resource_mut::<PendingRequests>()
                .expect("McpPlugin installs PendingRequests")
                .in_flight
                .push(waiting);
        }
    }
}

/// Runs one call, answering it unless it goes pending.
fn handle(world: &mut World, request: McpRequest) -> Option<InFlight> {
    let call = world
        .get_resource::<McpTools>()
        .and_then(|tools| tools.find(&request.tool))
        .map(|tool| Arc::clone(&tool.call));
    let Some(call) = call else {
        let _ = request.reply.send(Err(ToolError::new(
            "unknown_tool",
            format!("No tool named `{}`.", request.tool),
        )));
        return None;
    };
    let arguments = request.arguments;
    match guarded(&request.tool, || call(world, arguments)) {
        Ok(Handled::Done(result)) => {
            let _ = request.reply.send(result);
            None
        }
        Ok(Handled::Pending(waiting)) => Some(InFlight {
            tool: request.tool,
            reply: request.reply,
            pending: waiting,
        }),
        Err(error) => {
            let _ = request.reply.send(Err(error));
            None
        }
    }
}

/// Runs a handler, turning a panic into a tool error rather than taking the
/// app down with it. The panic message itself goes to stderr via the hook.
fn guarded<T>(tool: &str, run: impl FnOnce() -> T) -> Result<T, ToolError> {
    catch_unwind(AssertUnwindSafe(run)).map_err(|panic| {
        let reason = panic
            .downcast_ref::<&str>()
            .map(|reason| reason.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        ToolError::new(
            "internal",
            format!("`{tool}` panicked: {reason}. The app may be in an inconsistent state."),
        )
    })
}

/// Calls a tool the way a client would and runs frames until it answers, at
/// most `max_frames`. For tests and in-process hosts; needs [`McpPlugin`].
pub fn call_tool(app: &mut App, tool: &str, arguments: Value, max_frames: usize) -> ToolResult {
    let (reply, mut answer) = tokio::sync::oneshot::channel();
    app.get_resource::<McpInbox>()
        .expect("call_tool requires McpPlugin")
        .sender
        .send(Message::Call(McpRequest {
            tool: tool.into(),
            arguments,
            reply,
        }))
        .expect("the inbox holds its own receiver");
    for _ in 0..max_frames {
        app.update();
        if let Ok(result) = answer.try_recv() {
            return result;
        }
    }
    Err(ToolError::new(
        "timeout",
        format!("`{tool}` did not answer within {max_frames} frames."),
    ))
}

/// Arguments for a tool that takes none.
#[derive(Debug, Default, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArguments {}

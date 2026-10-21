use std::{collections::VecDeque, marker::PhantomData};

use concerto_ecs::{
    CommandQueue, Event, Query, Res, ResMut, Resource, World,
    events::{event_reader::EventReader, event_writer::EventWriter},
    system::{
        access::SystemAccess,
        input::{ReadOnlySystemInput, SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};
use crossbeam_channel::{Receiver, Sender};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{PendingCall, Reply, ServeCalls, ToolError, ToolRegistry};

/// One `tools/call` from the client, written in `First` and read by tools through [`ToolCalls`].
#[derive(Event, Clone)]
pub struct McpCall {
    pub tool: String,
    pub arguments: Value,
    pub reply: Reply,
}

/// The calls the server sent that no schedule has received yet.
#[derive(Resource)]
pub struct McpInbox {
    sender: Sender<McpCall>,
    receiver: Receiver<McpCall>,
    queued: VecDeque<McpCall>,
}

impl McpInbox {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self {
            sender,
            receiver,
            queued: VecDeque::new(),
        }
    }

    pub(crate) fn sender(&self) -> Sender<McpCall> {
        self.sender.clone()
    }

    pub(crate) fn holds_calls(&self) -> bool {
        !self.queued.is_empty()
    }

    fn take(&mut self, mut admit: impl FnMut(&str) -> bool) -> VecDeque<McpCall> {
        self.queued.extend(self.receiver.try_iter());
        let (taken, kept) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|call| admit(&call.tool));
        self.queued = kept;
        taken
    }
}

/// Writes the calls the server sent as [`McpCall`]s, then runs [`McpServe`](crate::McpServe); a changing call waits while another is pending.
pub fn receive_calls(
    mut inbox: ResMut<McpInbox>,
    registry: Res<ToolRegistry>,
    pending: Query<&PendingCall>,
    mut calls: EventWriter<McpCall>,
    mut cmd: CommandQueue,
) {
    let mut changing = pending.iter().next().is_some();
    let admitted = inbox.take(|tool| {
        if registry.is_read_only(tool) {
            return true;
        }
        !std::mem::replace(&mut changing, true)
    });
    for call in admitted {
        calls.write(call);
    }
    cmd.trigger(ServeCalls);
}

/// Writes the read-only calls the server sent as [`McpCall`]s and keeps the others for a frame.
pub fn receive_read_only_calls(
    mut inbox: ResMut<McpInbox>,
    registry: Res<ToolRegistry>,
    mut calls: EventWriter<McpCall>,
) {
    for call in inbox.take(|tool| registry.is_read_only(tool)) {
        calls.write(call);
    }
}

/// Answers every call still open with the error of its tool's unmet need, or `internal`.
pub fn answer_unanswered(mut calls: EventReader<McpCall>, registry: Res<ToolRegistry>) {
    for call in calls.read() {
        if call.reply.awaits_tool() {
            call.reply.err(registry.unanswered(&call.tool));
        }
    }
}

type Inputs = (
    EventReader<'static, 'static, McpCall>,
    Res<'static, ToolRegistry>,
);

/// The open calls of the tool whose arguments are `A`; calls with malformed arguments are answered `invalid_arguments`.
pub struct ToolCalls<'w, 's, A> {
    calls: EventReader<'w, 's, McpCall>,
    tool: &'static str,
    arguments: PhantomData<fn() -> A>,
}

impl<A: DeserializeOwned> ToolCalls<'_, '_, A> {
    /// Every open call of this tool not read before, with its parsed arguments and its reply.
    pub fn read(&mut self) -> impl Iterator<Item = (A, &Reply)> {
        let tool = self.tool;
        self.calls
            .read()
            .filter(move |call| call.tool == tool && call.reply.awaits_tool())
            .filter_map(
                |call| match serde_json::from_value(call.arguments.clone()) {
                    Ok(arguments) => Some((arguments, &call.reply)),
                    Err(error) => {
                        call.reply.err(ToolError::new(
                            "invalid_arguments",
                            format!("{error}. The tool's input schema lists its parameters."),
                        ));
                        None
                    }
                },
            )
    }
}

impl<A: 'static> SystemInput for ToolCalls<'_, '_, A> {
    type State = <Inputs as SystemInput>::State;
    type Data<'world, 'state> = ToolCalls<'world, 'state, A>;

    fn init_state(world: &mut World) -> Self::State {
        <Inputs>::init_state(world)
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares exactly the access of its parts.
        let (calls, registry) = unsafe { <Inputs>::get_data(state, world) }?;
        let tool = registry.name_of::<A>().ok_or_else(|| {
            ValidationError::missing::<Self>("the tool was not added with add_tool")
        })?;
        Ok(ToolCalls {
            calls,
            tool,
            arguments: PhantomData,
        })
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        <Inputs>::fill_access(meta, access);
    }
}

impl<A: 'static> ReadOnlySystemInput for ToolCalls<'_, '_, A> {}

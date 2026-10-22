use std::time::{Duration, Instant};

use concerto_ecs::{CommandQueue, Component, Entity, Query, Res, ResMut, Resource};

use crate::{McpInbox, Reply, ToolError};

/// A call answered on a later frame by the system that waits for `waiting_for`; changing calls queue behind it.
#[derive(Component)]
pub struct PendingCall {
    pub reply: Reply,
    pub deadline: Instant,
    pub waiting_for: &'static str,
}

impl PendingCall {
    /// How long a pending call waits unless its tool says otherwise.
    pub const TIMEOUT: Duration = Duration::from_secs(30);

    /// Takes `reply` from the call's tool and `answer_unanswered` until a waiting system answers it.
    pub fn new(reply: &Reply, waiting_for: &'static str) -> Self {
        reply.defer();
        Self {
            reply: reply.clone(),
            deadline: Instant::now() + Self::TIMEOUT,
            waiting_for,
        }
    }

    /// Answers `timeout` after `timeout` instead of the default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.deadline = Instant::now() + timeout;
        self
    }
}

/// Answers `timeout` to each pending call past its deadline, and despawns it and every answered one.
pub fn expire_pending_calls(pending: Query<(Entity, &PendingCall)>, mut cmd: CommandQueue) {
    let now = Instant::now();
    for (entity, call) in pending.iter() {
        if call.reply.is_open() && now >= call.deadline {
            call.reply.err(ToolError::new(
                "timeout",
                format!("Timed out waiting for {}.", call.waiting_for),
            ));
        }
        if !call.reply.is_open() {
            cmd.despawn(entity);
        }
    }
}

/// Whether the app needs the next frame; cleared at the start of each frame, so a system requests it on every frame it does.
#[derive(Resource, Default)]
pub struct FrameDemand {
    requested: bool,
    waiting: Vec<Reply>,
}

impl FrameDemand {
    /// Asks the host for the next frame.
    pub fn request(&mut self) {
        self.requested = true;
    }

    fn wait_for(&mut self, reply: &Reply) {
        self.waiting.push(reply.clone());
    }

    /// Whether a system asked for the next frame during this one, or a call pending during it is still open.
    pub fn is_requested(&self) -> bool {
        self.requested || self.waiting.iter().any(Reply::is_open)
    }
}

/// Clears [`FrameDemand`] at the start of `First`; a system in `First` that requests frames runs after it.
pub fn reset_frame_demand(mut demand: ResMut<FrameDemand>) {
    *demand = FrameDemand::default();
}

/// Requests the next frame while a call is held back, or a pending call is open when the host decides.
pub fn demand_frames(
    pending: Query<&PendingCall>,
    inbox: Res<McpInbox>,
    mut demand: ResMut<FrameDemand>,
) {
    for call in pending.iter() {
        demand.wait_for(&call.reply);
    }
    if inbox.holds_calls() {
        demand.request();
    }
}

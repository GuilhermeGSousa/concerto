use concerto_ecs::Component;

use crate::backend::PhysicsBackend;
use crate::ActiveBackend;

/// A handle to a body living inside the physics simulation.
///
/// Wraps the active backend's native handle (an id or index — never a
/// pointer, so it is trivially `Copy` and `Send + Sync`). Inserted on the
/// entity by [`Collider`](crate::collider::Collider)'s lifecycle when the
/// body is created.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Component)]
pub struct BodyId(pub(crate) <ActiveBackend as PhysicsBackend>::BodyHandle);

/// How a body is moving right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BodyActivity {
    /// The body never moves.
    Static,
    /// The body is moved by velocities its owner sets.
    Kinematic,
    /// The body is simulated.
    Dynamic,
    /// The body is dynamic or kinematic and has come to rest; a touch wakes it.
    Asleep,
}

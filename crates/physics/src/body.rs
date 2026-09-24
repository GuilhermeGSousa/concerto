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

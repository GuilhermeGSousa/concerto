use crate::{Entity, World, signal::listener::Listener, system::input::SystemArg};

/// The component lifecycle signals [`Add`](lifecycle::Add), [`Replace`](lifecycle::Replace),
/// [`Remove`](lifecycle::Remove) and [`Despawn`](lifecycle::Despawn).
pub mod lifecycle;
/// [`Listener`], the component that holds a system run for a signal.
pub mod listener;

/// A value sent to listeners with [`World::trigger`] or
/// [`CommandQueue::trigger`](crate::CommandQueue::trigger).
///
/// Every listener for the signal runs in registration order, and each one's commands apply before
/// the next runs.
pub trait Signal: Send + Sync + 'static {
    /// Prepares a listener for this signal as it is registered; lifecycle signals check it here.
    fn listener_added(_world: &mut World, _listener: &mut Listener<Self>)
    where
        Self: Sized,
    {
    }
}

/// A signal that can also target one entity's own listener, with [`World::trigger_on`] or
/// [`EntityCommandQueue::trigger`](crate::command::EntityCommandQueue::trigger).
pub trait EntitySignal: Signal {}

/// What a listener receives: the signal and the entity its listener sits on.
pub struct On<'w, S: Signal> {
    signal: &'w mut S,
    entity: Entity,
}

impl<S: Signal> On<'_, S> {
    /// Returns the signal.
    pub fn signal(&self) -> &S {
        self.signal
    }

    /// The entity whose listener is running.
    pub fn entity(&self) -> Entity {
        self.entity
    }
}

impl<S: Signal> SystemArg for On<'_, S> {
    type Arg<'i> = On<'i, S>;
}

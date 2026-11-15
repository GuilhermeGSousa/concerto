use crate::{Entity, World, signal::listener::Listener, system::input::SystemArg};

pub mod lifecycle;
pub mod listener;

pub trait Signal: Send + Sync + 'static {
    /// Prepares a listener for this signal as it is registered; lifecycle signals check it here.
    fn listener_added(_world: &mut World, _listener: &mut Listener<Self>)
    where
        Self: Sized,
    {
    }
}

pub trait EntitySignal: Signal {}

pub struct On<'w, S: Signal> {
    signal: &'w mut S,
    entity: Entity,
}

impl<S: Signal> On<'_, S> {
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

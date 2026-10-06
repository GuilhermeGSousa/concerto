use crate::{Entity, system::input::SystemArg};

pub mod listener;

pub trait Signal: Send + Sync + 'static {}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{System, World, signal::listener::IntoListenerSystem};

    struct Count(u32);

    impl Signal for Count {}

    #[test]
    fn listener_runs_with_a_borrowed_signal() {
        let mut world = World::new();
        let mut listener = (|on: On<Count>| on.signal.0 += 1).into_listener_system();
        listener.initialize(&mut world);

        let entity = world.spawn(());
        let mut count = Count(0);
        let signal = &mut count;
        listener.run_and_apply(On { signal, entity }, &mut world);
        let signal = &mut count;
        listener.run_and_apply(On { signal, entity }, &mut world);

        assert_eq!(count.0, 2);
    }
}

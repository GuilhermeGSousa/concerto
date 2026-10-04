use crate::{
    System,
    system::{IntoSystem, input::SystemArg},
};

pub trait Signal: Send + Sync + 'static {}

pub struct On<'w, S: Signal> {
    signal: &'w mut S,
}

pub trait ListenerSystem<S: Signal>: System<In = On<'static, S>> + 'static {}

impl<S: Signal, T: System<In = On<'static, S>>> ListenerSystem<S> for T {}

pub trait IntoListenerSystem<S: Signal, Marker> {
    type Listener: ListenerSystem<S>;

    fn into_listener(self) -> Self::Listener;
}

impl<S, Marker, T> IntoListenerSystem<S, Marker> for T
where
    S: Signal,
    T: IntoSystem<On<'static, S>, Marker>,
{
    type Listener = T::System;

    fn into_listener(self) -> Self::Listener {
        self.into_system()
    }
}

impl<S: Signal> SystemArg for On<'_, S> {
    type Arg<'i> = On<'i, S>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;

    struct Count(u32);

    impl Signal for Count {}

    #[test]
    fn listener_runs_with_a_borrowed_signal() {
        let mut world = World::new();
        let mut listener = (|on: On<Count>| on.signal.0 += 1).into_listener();
        listener.initialize(&mut world);

        let mut count = Count(0);
        listener.run_and_apply(On { signal: &mut count }, &mut world);
        listener.run_and_apply(On { signal: &mut count }, &mut world);

        assert_eq!(count.0, 2);
    }
}

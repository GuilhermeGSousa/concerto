use crate::system::input::SystemArg;

pub mod listener;

pub trait Signal: Send + Sync + 'static {}

pub struct On<'w, S: Signal> {
    signal: &'w mut S,
}

impl<S:Signal> On<'_, S>
{
    pub fn signal(&self) -> &S
    {
        self.signal
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

        let mut count = Count(0);
        listener.run_and_apply(On { signal: &mut count }, &mut world);
        listener.run_and_apply(On { signal: &mut count }, &mut world);

        assert_eq!(count.0, 2);
    }
}

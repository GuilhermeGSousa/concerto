use crate::System;


pub trait Signal: Send + Sync + 'static {}


pub struct On<'w, S: Signal>
{
    signal: &'w mut S
}


pub trait ListenerSystem<S: Signal>: System<In = On<'static, S>> + 'static {}

pub trait IntoListenerSystem<S: Signal> {

    type Listener: ListenerSystem<S>;

    fn into_listener(self) -> Self::Listener;
}
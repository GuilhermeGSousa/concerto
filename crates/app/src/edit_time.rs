use std::marker::PhantomData;

use concerto_ecs::{
    signal::{listener::IntoListener, Signal},
    IntoSystemConfig, SystemConfig, SystemTag, World,
};

/// The tag of systems that run in an edit `App`, given by [`AtEditTime::at_edit_time`].
#[derive(SystemTag, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EditTime;

/// A listener that [`App::add_listener`](crate::App::add_listener) also adds to an edit `App`.
pub struct EditTimeListener<L, S, M> {
    listener: L,
    marker: PhantomData<fn() -> (S, M)>,
}

#[doc(hidden)]
pub struct SystemMarker<M>(PhantomData<M>);

#[doc(hidden)]
pub struct ListenerMarker<S, M>(PhantomData<(S, M)>);

#[doc(hidden)]
pub struct Plain<M>(PhantomData<M>);

#[doc(hidden)]
pub struct Marked<M>(PhantomData<M>);

/// Marks a system or a listener to run in an edit `App` as well as in a game `App`.
pub trait AtEditTime<Marker> {
    type Output;

    fn at_edit_time(self) -> Self::Output;
}

impl<M, T: IntoSystemConfig<M>> AtEditTime<SystemMarker<M>> for T {
    type Output = SystemConfig;

    fn at_edit_time(self) -> SystemConfig {
        self.tagged(EditTime)
    }
}

impl<S: Signal, M, T: IntoListener<S, M>> AtEditTime<ListenerMarker<S, M>> for T {
    type Output = EditTimeListener<T, S, M>;

    fn at_edit_time(self) -> EditTimeListener<T, S, M> {
        EditTimeListener {
            listener: self,
            marker: PhantomData,
        }
    }
}

/// A listener [`App::add_listener`](crate::App::add_listener) accepts: a plain one or an [`EditTimeListener`].
pub trait AppListener<S: Signal, M> {
    fn add_to(self, world: &mut World, editing: bool);
}

impl<S: Signal, M, T: IntoListener<S, M>> AppListener<S, Plain<M>> for T {
    fn add_to(self, world: &mut World, editing: bool) {
        if !editing {
            world.add_listener(self);
        }
    }
}

impl<S: Signal, M, L: IntoListener<S, M>> AppListener<S, Marked<M>> for EditTimeListener<L, S, M> {
    fn add_to(self, world: &mut World, _editing: bool) {
        world.add_listener(self.listener);
    }
}

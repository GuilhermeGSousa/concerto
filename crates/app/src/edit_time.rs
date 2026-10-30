use std::marker::PhantomData;

use concerto_ecs::{
    signal::{listener::IntoListener, Signal},
    IntoSystemConfig, SystemConfig, SystemTag, World,
};

use crate::AppMode;

/// The tag of systems that run in an edit `App`, given by [`AtEditTime::at_edit_time`].
#[derive(SystemTag, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EditTime;

/// The tag of systems that run in a preview `App`, given by [`AtPreviewTime::at_preview_time`].
#[derive(SystemTag, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Preview;

/// A listener that [`App::add_listener`](crate::App::add_listener) also adds to an edit `App`.
pub struct EditTimeListener<L, S, M> {
    listener: L,
    marker: PhantomData<fn() -> (S, M)>,
}

/// A listener that [`App::add_listener`](crate::App::add_listener) also adds to a preview `App`.
pub struct PreviewTimeListener<L, S, M> {
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

/// Marks a system or a listener to run in a preview `App` as well as in a game `App`.
pub trait AtPreviewTime<Marker> {
    type Output;

    fn at_preview_time(self) -> Self::Output;
}

impl<M, T: IntoSystemConfig<M>> AtPreviewTime<SystemMarker<M>> for T {
    type Output = SystemConfig;

    fn at_preview_time(self) -> SystemConfig {
        self.tagged(Preview)
    }
}

impl<S: Signal, M, T: IntoListener<S, M>> AtPreviewTime<ListenerMarker<S, M>> for T {
    type Output = PreviewTimeListener<T, S, M>;

    fn at_preview_time(self) -> PreviewTimeListener<T, S, M> {
        PreviewTimeListener {
            listener: self,
            marker: PhantomData,
        }
    }
}

/// A listener [`App::add_listener`](crate::App::add_listener) accepts: a plain one, an [`EditTimeListener`] or a [`PreviewTimeListener`].
pub trait AppListener<S: Signal, M> {
    fn add_to(self, world: &mut World, mode: AppMode);
}

impl<S: Signal, M, T: IntoListener<S, M>> AppListener<S, Plain<M>> for T {
    fn add_to(self, world: &mut World, mode: AppMode) {
        if mode == AppMode::Game {
            world.add_listener(self);
        }
    }
}

impl<S: Signal, M, L: IntoListener<S, M>> AppListener<S, Marked<M>> for EditTimeListener<L, S, M> {
    fn add_to(self, world: &mut World, _mode: AppMode) {
        world.add_listener(self.listener);
    }
}

impl<S: Signal, M, L: IntoListener<S, M>> AppListener<S, Marked<M>>
    for PreviewTimeListener<L, S, M>
{
    fn add_to(self, world: &mut World, mode: AppMode) {
        if mode != AppMode::Edit {
            world.add_listener(self.listener);
        }
    }
}

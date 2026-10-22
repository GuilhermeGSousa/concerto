use std::marker::PhantomData;

use crate::{
    Component, Entity, World,
    signal::{
        Signal,
        listener::{ApplyListenerCommand, Listener},
    },
    world::UnsafeWorldCell,
};

macro_rules! lifecycle_signal {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        pub struct $name<T: Component> {
            pub entity: Entity,
            marker: PhantomData<fn() -> T>,
        }

        impl<T: Component> Signal for $name<T> {
            fn listener_added(world: &mut World, listener: &mut Listener<Self>) {
                prepare::<T, Self>(world, listener);
            }
        }

        impl<T: Component> LifecycleSignal for $name<T> {
            fn new(entity: Entity) -> Self {
                Self {
                    entity,
                    marker: PhantomData,
                }
            }
        }
    };
}

lifecycle_signal!(
    /// `T` was added to `entity`, or overwrote a value; the new value is readable.
    Add
);
lifecycle_signal!(
    /// `entity`'s `T` is about to be overwritten; the old value is readable.
    Replace
);
lifecycle_signal!(
    /// `entity`'s `T` is about to be removed; the value is readable.
    Remove
);
lifecycle_signal!(
    /// `entity`, which holds a `T`, is about to be despawned; the value is readable.
    Despawn
);

pub(crate) trait LifecycleSignal: Signal + Sized {
    fn new(entity: Entity) -> Self;
}

fn prepare<T: Component, S: Signal>(world: &mut World, listener: &mut Listener<S>) {
    if listener.access().reads_world() {
        panic!(
            "{} is a lifecycle listener and runs in the middle of a world operation, so it cannot take `&World` or `&mut World`; use `CommandQueue` for structural changes",
            listener.system_name()
        );
    }
    world.register_component_lifetimes::<T>();
    listener.initialize(world);
}

/// Runs the listeners of `S` for `entity` from inside the world operation the hook belongs to.
///
/// # Safety
/// The caller must hold the world exclusively, as a world operation running its hooks does.
pub(crate) unsafe fn dispatch<S: LifecycleSignal>(cell: UnsafeWorldCell, entity: Entity) {
    // SAFETY: the caller holds the world exclusively; the list is copied before any listener runs.
    let listeners = unsafe { cell.world() }.listeners().get::<S>().to_vec();
    for holder in listeners {
        let mut signal = S::new(entity);
        // SAFETY: as above; each listener runs alone, and its commands wait for the flush.
        if unsafe { Listener::<S>::run_in_operation(holder, &mut signal, cell) } {
            // SAFETY: as above; queuing a command does not touch component storage.
            unsafe { cell.world_mut() }
                .make_command_queue()
                .push(ApplyListenerCommand::<S>::new(holder));
        }
    }
}

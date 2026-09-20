pub mod access;
pub mod config;
pub mod executor;
mod graph;
pub mod input;
pub mod meta;
pub mod schedule;
mod sync_point;

use std::any::TypeId;

pub use config::{AlreadyConfigured, IntoSystemConfig, SystemConfig};

use input::SystemInput;
use typle::typle;

use crate::{
    system::{access::SystemAccess, meta::SystemMetadata},
    world::{UnsafeWorldCell, World},
};

/// Type alias for a boxed, type-erased system.
pub type BoxedSystem = Box<dyn System>;

/// Core trait implemented by all executable systems.
///
/// In normal use you don't implement this directly — plain Rust functions whose
/// parameters implement [`SystemInput`] automatically implement [`IntoSystem`], which
/// wraps them in a [`FunctionSystem`] that implements `System`.
///
pub trait System: Send + Sync + 'static {
    /// Returns the fully-qualified name of the underlying function or type.
    fn name(&self) -> &'static str;

    fn system_type(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    fn initialize(&mut self, world: &mut World);

    /// Describes which components and resources this system reads or writes.
    fn fill_access(&self, _meta: &mut SystemMetadata, _access: &mut SystemAccess);

    /// Executes the system against the world, then applies any deferred commands.
    fn run_and_apply(&mut self, world: &mut World) {
        self.run(world);
        self.apply(world);
    }

    /// Executes the system without applying deferred commands.
    fn run(&mut self, world: &mut World) {
        let world_cell = world.as_unsafe_world_cell_mut();
        unsafe { self.run_unsafe(world_cell) };
    }

    /// Unsafe version of run made to be used by multithreaded executors
    /// # Safety
    ///
    /// It is up to the user to ensure that no other system that isn't disjoint from this one
    /// is running simultaneously on this world
    unsafe fn run_unsafe(&mut self, world: UnsafeWorldCell);

    /// Applies any deferred mutations (e.g. spawned entities from [`CommandQueue`](crate::command::CommandQueue)).
    fn apply(&mut self, world: &mut World);
}

impl System for BoxedSystem {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn system_type(&self) -> TypeId {
        (**self).system_type()
    }

    fn apply(&mut self, world: &mut World) {
        (**self).apply(world);
    }

    unsafe fn run_unsafe(&mut self, world: UnsafeWorldCell) {
        unsafe { (**self).run_unsafe(world) };
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        (**self).fill_access(meta, access);
    }

    fn initialize(&mut self, world: &mut World) {
        (**self).initialize(world);
    }
}

/// Wraps a plain function (or closure) and its cached input state into a [`System`].
pub(crate) struct FunctionSystem<F, Input: SystemInput> {
    pub func: F,
    system_state: Option<Input::State>,
}

impl<F, Input> FunctionSystem<F, Input>
where
    Input: SystemInput + 'static,
{
    pub fn new(func: F) -> Self {
        Self {
            func,
            system_state: None,
        }
    }
}

#[allow(unused_variables, unused_mut, clippy::unit_arg)]
#[typle(Tuple for 0..=12)]
impl<F, T> System for FunctionSystem<F, T>
where
    F: Send + Sync + 'static,
    T: Tuple,
    T<_>: SystemInput + 'static,
    for<'w, 's> F:
        FnMut(typle_args!(i in .. => T<{i}>)) + FnMut(typle_args!(i in .. => T<{i}>::Data<'w, 's>)),
{
    fn name(&self) -> &'static str {
        std::any::type_name::<F>()
    }

    fn initialize(&mut self, world: &mut World) {
        self.system_state = Some(T::init_state(world));
    }

    fn apply(&mut self, world: &mut World) {
        for typle_index!(i) in 0..T::LEN {
            let state = self
                .system_state
                .as_mut()
                .expect("Attempted to run uninitialized system.");
            <T<{ i }>>::apply(&mut state[[i]], world);
        }
    }

    unsafe fn run_unsafe(&mut self, world: UnsafeWorldCell) {
        let state = self
            .system_state
            .as_mut()
            .expect("Attempted to run uninitialized system.");
        (self.func)(typle_args!(i in .. =>  {
            <T<{i}>>::get_data(&mut state[[i]], world)
        }));
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        for typle_index!(i) in 0..T::LEN {
            <T<{ i }>>::fill_access(meta, access);
        }
    }
}

/// Conversion trait that turns a compatible function or closure into a [`ScheduledSystem`].
///
/// Implemented automatically for functions whose parameters implement [`SystemInput`],
/// and for any type that already implements [`System`].
pub trait IntoSystem<Marker> {
    /// Wraps `self` in a [`ScheduledSystem`] ready to be added to a [`Schedule`](schedule::Schedule).
    fn into_system(self) -> BoxedSystem;
}

/// Marker used by the blanket [`IntoSystem`] impl for types that already implement [`System`].
pub struct AlreadySystem;

impl<S: System + 'static> IntoSystem<AlreadySystem> for S {
    fn into_system(self) -> BoxedSystem {
        Box::new(self)
    }
}

#[typle(Tuple for 0..=12)]
impl<F, T> IntoSystem<T> for F
where
    F: Send + Sync + 'static,
    T: Tuple,
    T<_>: SystemInput + 'static,
    for<'w, 's> F:
        FnMut(typle_args!(i in .. => T<{i}>)) + FnMut(typle_args!(i in .. => T<{i}>::Data<'w, 's>)),
{
    fn into_system(self) -> BoxedSystem {
        Box::new(FunctionSystem::new(self))
    }
}

pub struct NonSendMarker;

impl SystemInput for NonSendMarker {
    type State = ();

    type Data<'world, 'state> = NonSendMarker;

    fn init_state(_world: &mut World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        _world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        NonSendMarker
    }

    fn fill_access(meta: &mut SystemMetadata, _access: &mut SystemAccess) {
        meta.set_non_send();
    }
}

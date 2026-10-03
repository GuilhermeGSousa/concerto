pub mod access;
pub mod config;
pub mod executor;
mod graph;
pub mod input;
pub mod meta;
mod reachability;
pub mod schedule;
pub mod set;
mod sync_point;

use std::{any::TypeId, marker::PhantomData};

pub use config::{
    AlreadyConfigured, DependencyTarget, IntoDependencyTarget, IntoSystemConfig, SystemConfig,
};
pub use set::{InternedSystemSet, IntoSetConfig, SetConfig, SystemSet};

use input::SystemInput;
use typle::typle;

use crate::{
    system::{access::SystemAccess, input::SystemArg, meta::SystemMetadata},
    world::{UnsafeWorldCell, World},
};

pub type BoxedSystem = Box<dyn System<In = ()>>;

/// A unit of work a [`Schedule`](crate::Schedule) runs against a [`World`].
///
/// Functions become systems through [`IntoSystem`]; implementing this directly is rarely needed.
pub trait System: Send + Sync + 'static {
    type In: SystemArg;

    fn name(&self) -> &'static str;

    /// Returns the id used to name this system as an ordering target.
    fn system_type(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    fn initialize(&mut self, world: &mut World);

    fn fill_access(&self, _meta: &mut SystemMetadata, _access: &mut SystemAccess);

    fn run_and_apply(&mut self, args: Self::In, world: &mut World) {
        self.run(args, world);
        self.apply(world);
    }

    fn run(&mut self, args: Self::In, world: &mut World) {
        let world_cell = world.as_unsafe_world_cell_mut();
        unsafe { self.run_unsafe(args, world_cell) };
    }

    /// Runs the system through a shared world cell, for parallel executors.
    ///
    /// # Safety
    ///
    /// No system with conflicting access may run on the same world at the same time.
    unsafe fn run_unsafe(&mut self, args: Self::In, world: UnsafeWorldCell);

    fn apply(&mut self, world: &mut World);
}

impl System for BoxedSystem {
    type In = ();

    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn system_type(&self) -> TypeId {
        (**self).system_type()
    }

    fn apply(&mut self, world: &mut World) {
        (**self).apply(world);
    }

    unsafe fn run_unsafe(&mut self, args: Self::In, world: UnsafeWorldCell) {
        unsafe { (**self).run_unsafe(args, world) };
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        (**self).fill_access(meta, access);
    }

    fn initialize(&mut self, world: &mut World) {
        (**self).initialize(world);
    }
}

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
impl<F, Inputs> System for FunctionSystem<F, Inputs>
where
    F: Send + Sync + 'static,
    Inputs: Tuple,
    Inputs<_>: SystemInput + 'static,
    for<'w, 's> F: FnMut(typle_args!(i in .. => Inputs<{i}>))
        + FnMut(typle_args!(i in .. => Inputs<{i}>::Data<'w, 's>)),
{
    type In = ();

    fn name(&self) -> &'static str {
        std::any::type_name::<F>()
    }

    fn initialize(&mut self, world: &mut World) {
        self.system_state = Some(Inputs::init_state(world));
    }

    fn apply(&mut self, world: &mut World) {
        for typle_index!(i) in 0..Inputs::LEN {
            let state = self
                .system_state
                .as_mut()
                .expect("Attempted to run uninitialized system.");
            <Inputs<{ i }>>::apply(&mut state[[i]], world);
        }
    }

    unsafe fn run_unsafe(&mut self, args: Self::In, world: UnsafeWorldCell) {
        let state = self
            .system_state
            .as_mut()
            .expect("Attempted to run uninitialized system.");
        (self.func)(typle_args!(i in .. =>  {
            <Inputs<{i}>>::get_data(&mut state[[i]], world)
        }));
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        for typle_index!(i) in 0..Inputs::LEN {
            <Inputs<{ i }>>::fill_access(meta, access);
        }
    }
}

/// Converts a function, closure or [`System`] into a [`BoxedSystem`].
pub trait IntoSystem<Args: SystemArg, Marker>: Sized {
    type System: System<In = Args>;
    fn into_system(self) -> Self::System;

    fn with_args<T>(self, args: T) -> SystemWithArgs<Self::System, T>
    where
        T: Send + Sync + SystemArg + 'static,
    {
        SystemWithArgs::new(self, args)
    }
}

pub struct SystemWithArgs<S, Args>
where
    S: System<In = Args>,
    Args: SystemArg,
{
    system: S,
    args: Args,
}

impl<S, Args> SystemWithArgs<S, Args>
where
    S: System<In = Args>,
    Args: SystemArg,
{
    fn new<M>(system: impl IntoSystem<Args, M, System = S>, args: Args) -> Self {
        Self {
            system: system.into_system(),
            args,
        }
    }
}

impl<S, Args> System for SystemWithArgs<S, Args>
where
    S: System<In = Args>,
    Args: SystemArg + 'static,
{
    type In = ();

    fn name(&self) -> &'static str {
        self.system.name()
    }

    fn initialize(&mut self, world: &mut World) {
        self.system.initialize(world);
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        self.system.fill_access(meta, access);
    }

    unsafe fn run_unsafe(&mut self, _args: Self::In, world: UnsafeWorldCell) {
        unsafe { self.system.run_unsafe(self.args, world) };
    }

    fn apply(&mut self, world: &mut World) {
        self.system.apply(world);
    }
}

#[doc(hidden)]
pub struct AlreadySystem;

impl<S: System<In = ()> + 'static> IntoSystem<AlreadySystem> for S {
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

/// A system parameter that pins the system to the main thread.
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

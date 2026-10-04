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
    system::{
        access::SystemAccess,
        input::{SystemArg, SystemInputData},
        meta::SystemMetadata,
    },
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

/// A function that can run as a [`System`], optionally taking a leading [`SystemArg`].
pub trait SystemFunction<Marker>: Send + Sync + 'static {
    type In: SystemArg;
    type Inputs: SystemInput;

    fn call(&mut self, args: Self::In, data: SystemInputData<'_, '_, Self::Inputs>);
}

#[doc(hidden)]
pub struct WithArgs<Args, T>(PhantomData<fn(Args, T)>);

#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<F, T> SystemFunction<T> for F
where
    F: Send + Sync + 'static,
    T: Tuple,
    T<_>: SystemInput + 'static,
    for<'w, 's> F:
        FnMut(typle_args!(i in .. => T<{i}>)) + FnMut(typle_args!(i in .. => T<{i}>::Data<'w, 's>)),
{
    type In = ();
    type Inputs = T;

    fn call(&mut self, _args: (), data: SystemInputData<'_, '_, T>) {
        self(typle_args!(i in .. => data[[i]]));
    }
}

#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<F, Args, T> SystemFunction<WithArgs<Args, T>> for F
where
    F: Send + Sync + 'static,
    T: Tuple,
    T<_>: SystemInput + 'static,
    Args: SystemArg + 'static,
    for<'w, 's> F: FnMut(Args, typle_args!(i in .. => T<{i}>))
        + FnMut(Args, typle_args!(i in .. => T<{i}>::Data<'w, 's>)),
{
    type In = Args;
    type Inputs = T;

    fn call(&mut self, args: Args, data: SystemInputData<'_, '_, T>) {
        self(args, typle_args!(i in .. => data[[i]]));
    }
}

pub struct FunctionSystem<F: SystemFunction<Marker>, Marker> {
    pub func: F,
    system_state: Option<<F::Inputs as SystemInput>::State>,
    marker: PhantomData<fn() -> Marker>,
}

impl<F: SystemFunction<Marker>, Marker> FunctionSystem<F, Marker> {
    pub fn new(func: F) -> Self {
        Self {
            func,
            system_state: None,
            marker: PhantomData,
        }
    }
}

impl<F, Marker> System for FunctionSystem<F, Marker>
where
    F: SystemFunction<Marker>,
    Marker: 'static,
{
    type In = F::In;

    fn name(&self) -> &'static str {
        std::any::type_name::<F>()
    }

    fn initialize(&mut self, world: &mut World) {
        self.system_state = Some(F::Inputs::init_state(world));
    }

    fn apply(&mut self, world: &mut World) {
        let state = self
            .system_state
            .as_mut()
            .expect("Attempted to run uninitialized system.");
        F::Inputs::apply(state, world);
    }

    unsafe fn run_unsafe(&mut self, args: Self::In, world: UnsafeWorldCell) {
        let state = self
            .system_state
            .as_mut()
            .expect("Attempted to run uninitialized system.");
        self.func.call(args, F::Inputs::get_data(state, world));
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        F::Inputs::fill_access(meta, access);
    }
}

/// Converts a function, closure or [`System`] into a [`BoxedSystem`].
pub trait IntoSystem<Args: SystemArg, Marker>: Sized {
    type System: System<In = Args>;
    fn into_system(self) -> Self::System;

    /// Converts into a type-erased [`BoxedSystem`].
    fn into_boxed_system(self) -> BoxedSystem
    where
        Self::System: System<In = ()>,
    {
        Box::new(self.into_system())
    }

    fn with_args(self, args: Args) -> SystemWithArgs<Self::System, Args>
    where
        Args: Clone + 'static,
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
    Args: SystemArg + Clone + 'static,
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
        unsafe { self.system.run_unsafe(self.args.clone(), world) };
    }

    fn apply(&mut self, world: &mut World) {
        self.system.apply(world);
    }
}

#[doc(hidden)]
pub struct AlreadySystem;

impl<S: System<In = ()> + 'static> IntoSystem<(), AlreadySystem> for S {
    type System = S;

    fn into_system(self) -> Self::System {
        self
    }
}

#[doc(hidden)]
pub struct IsFunctionSystem;

impl<F, Marker> IntoSystem<F::In, (IsFunctionSystem, Marker)> for F
where
    F: SystemFunction<Marker>,
    Marker: 'static,
{
    type System = FunctionSystem<F, Marker>;

    fn into_system(self) -> Self::System {
        FunctionSystem::new(self)
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

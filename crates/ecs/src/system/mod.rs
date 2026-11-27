/// [`SystemAccess`], what a system reads and writes, from which the scheduler decides what may run
/// in parallel.
pub mod access;
/// [`SystemConfig`], a system with its ordering, sets and tags, as a schedule takes it.
pub mod config;
/// Executors that run a compiled schedule's systems, on one thread or across the compute pool.
pub mod executor;
mod graph;
/// System parameters: the [`SystemInput`] trait and the parameters that are not queries, resources
/// or commands.
pub mod input;
/// [`SystemMetadata`], properties of a system that constrain where it may run.
pub mod meta;
mod reachability;
/// [`Schedule`](schedule::Schedule), a set of systems compiled into an order and run on a world.
pub mod schedule;
/// [`SystemSet`], a named group of systems ordered as a unit.
pub mod set;
mod stand_in;
mod sync_point;
/// [`SystemTag`], a label that selects which systems a schedule compiles, without affecting order.
pub mod tag;

use std::{any::TypeId, marker::PhantomData};

pub use config::{
    AlreadyConfigured, DependencyTarget, IntoDependencyTarget, IntoSystemConfig, SystemConfig,
};
pub use set::{InternedSystemSet, IntoSetConfig, SetConfig, SystemSet};
pub use tag::{SystemTag, TagFilter};

use input::{SystemInput, ValidationError, ValidationKind};
use typle::typle;

use crate::{
    component::{CheckChangeTicks, SystemTicks, Tick},
    system::{
        access::SystemAccess,
        input::{SystemArg, SystemArgData, SystemInputData},
        meta::SystemMetadata,
    },
    world::{UnsafeWorldCell, World},
};

/// A type-erased system that takes no argument, as a schedule stores it.
pub type BoxedSystem = Box<dyn System<In = ()>>;

/// The argument a system of type `S` takes, with its borrow lifetime set to `'a`.
pub type SysArg<'a, S> = SystemArgData<'a, <S as System>::In>;

/// A unit of work a [`Schedule`](crate::Schedule) runs against a [`World`].
///
/// Functions become systems through [`IntoSystem`]; implementing this directly is rarely needed.
pub trait System: Send + Sync + 'static {
    /// The argument the system takes when run: `()` for an ordinary system,
    /// [`On`](crate::signal::On) for a listener.
    type In: SystemArg;

    /// Returns the system's name, for logs and errors; for a function, its Rust path.
    fn name(&self) -> &'static str;

    /// Returns the id used to name this system as an ordering target.
    fn system_type(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    /// Sets up the system's parameter state on `world`; call once before the first run.
    ///
    /// [`FunctionSystem`] panics here, naming the conflict, when its parameters alias.
    fn initialize(&mut self, world: &mut World);

    /// Adds what the system reads and writes to the access, and where it may run to the metadata.
    fn fill_access(&self, _meta: &mut SystemMetadata, _access: &mut SystemAccess);

    /// Runs the system on `world`, then applies its deferred commands.
    fn run_and_apply(&mut self, args: SysArg<'_, Self>, world: &mut World) {
        self.run(args, world);
        self.apply(world);
    }

    /// Runs the system on `world` without applying its deferred commands.
    fn run(&mut self, args: SysArg<'_, Self>, world: &mut World) {
        let world_cell = world.as_unsafe_world_cell_mut();
        // SAFETY: the exclusive borrow of `world` rules out any other running system.
        unsafe { self.run_unsafe(args, world_cell) };
    }

    /// Runs the system through a shared world cell, for parallel executors.
    ///
    /// # Safety
    ///
    /// No system with conflicting access may run on the same world at the same time.
    unsafe fn run_unsafe(&mut self, args: SysArg<'_, Self>, world: UnsafeWorldCell);

    /// Applies the commands and other deferred changes the system queued while it ran.
    fn apply(&mut self, world: &mut World);

    /// Clamps the ticks this system stores that are older than the check allows.
    fn check_change_tick(&mut self, check: CheckChangeTicks);
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
        // SAFETY: the caller upholds `run_unsafe`'s contract, which forwards unchanged to the boxed system.
        unsafe { (**self).run_unsafe(args, world) };
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        (**self).fill_access(meta, access);
    }

    fn initialize(&mut self, world: &mut World) {
        (**self).initialize(world);
    }

    fn check_change_tick(&mut self, check: CheckChangeTicks) {
        (**self).check_change_tick(check);
    }
}

/// A function that can run as a [`System`], optionally taking a leading [`SystemArg`].
pub trait SystemFunction<Marker>: Send + Sync + 'static {
    /// The argument the function takes before its parameters, or `()` when it takes none.
    type In: SystemArg;
    /// The function's parameters, as a tuple of [`SystemInput`]s.
    type Inputs: SystemInput;

    /// Calls the function with `args` and its parameters' data for this run.
    fn call(
        &mut self,
        args: SystemArgData<'_, Self::In>,
        data: SystemInputData<'_, '_, Self::Inputs>,
    );
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
    for<'i, 'w, 's> F: FnMut(Args, typle_args!(i in .. => T<{i}>))
        + FnMut(Args::Arg<'i>, typle_args!(i in .. => T<{i}>::Data<'w, 's>)),
{
    type In = Args;
    type Inputs = T;

    fn call(&mut self, args: SystemArgData<'_, Args>, data: SystemInputData<'_, '_, T>) {
        self(args, typle_args!(i in .. => data[[i]]));
    }
}

/// A [`System`] built from a function whose parameters are [`SystemInput`]s.
///
/// A run is skipped while a parameter declines, with a warning when the data it needs is missing.
pub struct FunctionSystem<F: SystemFunction<Marker>, Marker> {
    /// The function the system calls.
    pub func: F,
    system_state: Option<<F::Inputs as SystemInput>::State>,
    last_run: Tick,
    marker: PhantomData<fn() -> Marker>,
}

impl<F: SystemFunction<Marker>, Marker> FunctionSystem<F, Marker> {
    /// Wraps `func`; [`System::initialize`] must run before the first run.
    pub fn new(func: F) -> Self {
        Self {
            func,
            system_state: None,
            last_run: Tick::default(),
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
        self.last_run = world.change_tick().relative_to(Tick::MAX);
        let mut access = SystemAccess::default();
        self.fill_access(&mut SystemMetadata::default(), &mut access);
        if let Some(conflict) = access.conflict() {
            panic!(
                "System `{}` has parameters that conflict: {conflict}.",
                self.name()
            );
        }
    }

    fn apply(&mut self, world: &mut World) {
        let state = self
            .system_state
            .as_mut()
            .expect("Attempted to run uninitialized system.");
        F::Inputs::apply(state, world);
    }

    unsafe fn run_unsafe(&mut self, args: SysArg<'_, Self>, world: UnsafeWorldCell) {
        let state = self
            .system_state
            .as_mut()
            .expect("Attempted to run uninitialized system.");
        let ticks = SystemTicks {
            last_run: self.last_run,
            this_run: world.increment_change_tick(),
        };
        // SAFETY: the caller guarantees no conflicting system runs, and the inputs touch
        // only the access they declared for this system.
        let ran = match unsafe { F::Inputs::get_data(state, world, ticks) } {
            Ok(data) => {
                self.func.call(args, data);
                true
            }
            Err(error) if error.kind == ValidationKind::Missing => {
                log::warn!("{} skipped: {error}", std::any::type_name::<F>());
                false
            }
            Err(_) => false,
        };
        if ran {
            F::Inputs::ran(state);
            self.last_run = ticks.this_run;
        }
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        match &self.system_state {
            Some(state) => F::Inputs::fill_state_access(state, meta, access),
            None => F::Inputs::fill_access(meta, access),
        }
    }

    fn check_change_tick(&mut self, check: CheckChangeTicks) {
        if self.last_run.check_tick(check) {
            let age = *check.present_tick().relative_to(self.last_run);
            log::warn!(
                "System '{}' has not run for {age} ticks. Changes older than {} ticks will not be detected.",
                self.name(),
                *Tick::MAX - 1,
            );
        }
    }
}

/// Converts a function, closure or [`System`] into a [`BoxedSystem`].
pub trait IntoSystem<Args: SystemArg, Marker>: Sized {
    /// The system it converts into.
    type System: System<In = Args>;
    /// Converts into the system.
    fn into_system(self) -> Self::System;

    /// Converts into a type-erased [`BoxedSystem`].
    fn into_boxed_system(self) -> BoxedSystem
    where
        Self::System: System<In = ()>,
    {
        Box::new(self.into_system())
    }

    /// Binds `args` as the argument of every run, giving a system that takes none.
    fn with_args(self, args: Args) -> SystemWithArgs<Self::System, Args>
    where
        Args: for<'i> SystemArg<Arg<'i> = Args> + 'static,
    {
        SystemWithArgs::new(self, args)
    }
}

/// A system that takes an argument, run with a fixed copy of it each time; built by
/// [`IntoSystem::with_args`].
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
    Args: for<'i> SystemArg<Arg<'i> = Args> + Clone + 'static,
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
        // SAFETY: the caller upholds `run_unsafe`'s contract, which forwards unchanged to the wrapped system.
        unsafe { self.system.run_unsafe(self.args.clone(), world) };
    }

    fn apply(&mut self, world: &mut World) {
        self.system.apply(world);
    }

    fn check_change_tick(&mut self, check: CheckChangeTicks) {
        self.system.check_change_tick(check);
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

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        _world: UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        Ok(NonSendMarker)
    }

    fn fill_access(meta: &mut SystemMetadata, _access: &mut SystemAccess) {
        meta.set_non_send();
    }
}

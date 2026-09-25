/// Tracking of the data each system reads and writes.
pub mod access;
/// Ordering constraints and set membership for systems.
pub mod config;
/// Strategies for running a compiled schedule.
pub mod executor;
mod graph;
/// Parameters a system function can take.
pub mod input;
/// Scheduling metadata for systems.
pub mod meta;
mod reachability;
/// Schedules and their compilation.
pub mod schedule;
/// Named groups of systems.
pub mod set;
mod sync_point;

use std::any::TypeId;

pub use config::{
    AlreadyConfigured, DependencyTarget, IntoDependencyTarget, IntoSystemConfig, IntoSystemConfigs,
    SystemConfig,
};
pub use set::{InternedSystemSet, IntoSetConfig, IntoSetConfigs, SetConfig, SystemSet};

use input::SystemInput;
use typle::typle;

use crate::{
    system::{access::SystemAccess, meta::SystemMetadata},
    world::{UnsafeWorldCell, World},
};

/// A boxed, type-erased [`System`].
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSystem, system::BoxedSystem};
///
/// fn tick() {}
///
/// let system: BoxedSystem = tick.into_system();
/// assert!(system.name().ends_with("tick"));
/// ```
pub type BoxedSystem = Box<dyn System>;

/// A unit of work a [`Schedule`](crate::Schedule) runs against a [`World`].
///
/// Plain functions whose parameters implement [`SystemInput`] are converted into
/// systems by [`IntoSystem`], so this trait rarely needs implementing by hand.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{
///     System, World,
///     system::{access::SystemAccess, meta::SystemMetadata},
///     world::UnsafeWorldCell,
/// };
///
/// struct Noop;
///
/// impl System for Noop {
///     fn name(&self) -> &'static str {
///         "Noop"
///     }
///
///     fn initialize(&mut self, _world: &mut World) {}
///
///     fn fill_access(&self, _meta: &mut SystemMetadata, _access: &mut SystemAccess) {}
///
///     unsafe fn run_unsafe(&mut self, _world: UnsafeWorldCell) {}
///
///     fn apply(&mut self, _world: &mut World) {}
/// }
///
/// let mut world = World::new();
/// let mut system = Noop;
/// system.initialize(&mut world);
/// system.run_and_apply(&mut world);
/// ```
pub trait System: Send + Sync + 'static {
    /// Returns the fully-qualified name of the underlying function or type.
    fn name(&self) -> &'static str;

    /// Returns a [`TypeId`] identifying this system, used to name it as an ordering target.
    ///
    /// Defaults to the type implementing `System`; function systems return an id
    /// unique to their function.
    fn system_type(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    /// Prepares the system's cached state. Called once before the first run.
    fn initialize(&mut self, world: &mut World);

    /// Records which components and resources this system reads or writes.
    fn fill_access(&self, _meta: &mut SystemMetadata, _access: &mut SystemAccess);

    /// Runs the system against the world, then applies its deferred commands.
    fn run_and_apply(&mut self, world: &mut World) {
        self.run(world);
        self.apply(world);
    }

    /// Runs the system without applying its deferred commands.
    fn run(&mut self, world: &mut World) {
        let world_cell = world.as_unsafe_world_cell_mut();
        unsafe { self.run_unsafe(world_cell) };
    }

    /// Runs the system through a shared world cell, for executors that run systems in
    /// parallel.
    ///
    /// # Safety
    ///
    /// No other system whose access conflicts with this one may run on the same
    /// world at the same time.
    unsafe fn run_unsafe(&mut self, world: UnsafeWorldCell);

    /// Applies deferred mutations, such as entities spawned through a
    /// [`CommandQueue`](crate::CommandQueue).
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

/// Converts a function, closure or [`System`] into a [`BoxedSystem`].
///
/// Implemented for functions and closures of up to twelve parameters that each
/// implement [`SystemInput`], and for every type implementing [`System`].
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSystem, Res, Resource};
///
/// #[derive(Resource)]
/// struct Gravity(f32);
///
/// fn report(gravity: Res<Gravity>) {
///     println!("{}", gravity.0);
/// }
///
/// let system = report.into_system();
/// assert!(system.name().ends_with("report"));
/// ```
pub trait IntoSystem<Marker> {
    /// Boxes `self` as a system ready to be added to a [`Schedule`](crate::Schedule).
    fn into_system(self) -> BoxedSystem;
}

#[doc(hidden)]
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

/// A system parameter that pins the system to the main thread.
///
/// Take it in systems that touch data which must not leave the main thread, such as
/// window handles.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{Schedule, system::NonSendMarker};
///
/// fn poll_window(_main_thread: NonSendMarker) {}
///
/// let mut schedule = Schedule::new();
/// schedule.add_system(poll_window);
/// ```
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

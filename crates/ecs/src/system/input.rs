mod component_reader;
pub use component_reader::ComponentMetadata;

use std::{
    borrow::Cow,
    fmt,
    ops::{Deref, DerefMut},
};

use crate::{
    component::{CheckChangeTicks, SystemTicks, Tick},
    system::{access::SystemAccess, meta::SystemMetadata},
    utilities::SyncCell,
    world::{FromWorld, UnsafeWorldCell, World},
};
use derive_more::{Deref, DerefMut};
use typle::typle;

pub trait SystemArg: Send + Sync {
    type Arg<'i>: SystemArg;
}

// TODO: Cleanup and implement on tuple
impl SystemArg for () {
    type Arg<'i> = ();
}

/// Why a system input declined, which skips its system or listener.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub input: &'static str,
    pub kind: ValidationKind,
    pub message: Cow<'static, str>,
}

/// Whether a decline is expected and silent, or data the system needs is missing and warns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationKind {
    Skip,
    Missing,
}

impl ValidationError {
    /// The input `T` chose not to run now; its system is skipped silently.
    pub fn skip<T: ?Sized>(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new::<T>(ValidationKind::Skip, message)
    }

    /// The input `T` lacks data its system needs; its system is skipped with a warning.
    pub fn missing<T: ?Sized>(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new::<T>(ValidationKind::Missing, message)
    }

    fn new<T: ?Sized>(kind: ValidationKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            input: std::any::type_name::<T>(),
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.input, self.message)
    }
}

impl std::error::Error for ValidationError {}

pub trait SystemInput {
    type State: Send + Sync + Sized;
    type Data<'world, 'state>;

    fn init_state(world: &mut World) -> Self::State;

    /// Builds the input's data for one run with `ticks`, or declines with why.
    ///
    /// # Safety
    /// `world` may be used only for what [`SystemInput::fill_access`] declared, and no
    /// running system may hold access that conflicts with it.
    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError>;

    /// Runs after the system's body ran with this input's data; a skipped run never calls it.
    fn ran(_state: &mut Self::State) {}

    fn apply(_state: &mut Self::State, _world: &mut World) {}

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess);
}

pub trait ReadOnlySystemInput: SystemInput {}

#[allow(unused_variables, unused_mut)]
#[typle(Tuple for 0..=12)]
impl<T> SystemInput for T
where
    T: Tuple,
    T<_>: SystemInput + 'static,
{
    type State = typle_for!(i in .. => T<{i}>::State);
    type Data<'world, 'state> = typle_for!(i in .. => T<{i}>::Data<'world, 'state>);

    fn init_state(world: &mut World) -> Self::State {
        typle_for!(i in .. => <T<{i}>>::init_state(world))
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: the tuple's access is the union of its parts', which the caller upholds.
        Ok(typle_for!(i in .. => unsafe { <T<{i}>>::get_data(&mut state[[i]], world, ticks) }?))
    }

    fn ran(state: &mut Self::State) {
        typle_for!(i in .. => <T<{i}>>::ran(&mut state[[i]]));
    }

    fn apply(state: &mut Self::State, world: &mut World) {
        typle_for!(i in .. => <T<{i}>>::apply(&mut state[[i]], world));
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        typle_for!(i in .. => <T<{i}>>::fill_access(meta, access));
    }
}

pub type SystemInputData<'w, 's, P> = <P as SystemInput>::Data<'w, 's>;

pub type SystemArgData<'i, A> = <A as SystemArg>::Arg<'i>;

pub struct StaticSystemInput<'w, 's, P: SystemInput>(SystemInputData<'w, 's, P>);

impl<'w, 's, P: SystemInput> Deref for StaticSystemInput<'w, 's, P> {
    type Target = SystemInputData<'w, 's, P>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'w, 's, P: SystemInput> DerefMut for StaticSystemInput<'w, 's, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'w, 's, P: SystemInput> StaticSystemInput<'w, 's, P> {
    pub fn into_inner(self) -> SystemInputData<'w, 's, P> {
        self.0
    }
}

impl<'w, 's, P: SystemInput + 'static> SystemInput for StaticSystemInput<'w, 's, P> {
    type State = P::State;
    type Data<'world, 'state> = StaticSystemInput<'world, 'state, P>;

    fn init_state(world: &mut World) -> Self::State {
        P::init_state(world)
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares exactly `P`'s access.
        unsafe { P::get_data(state, world, ticks) }.map(StaticSystemInput)
    }

    fn ran(state: &mut Self::State) {
        P::ran(state);
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        P::fill_access(meta, access);
    }
}

/// An input's `last_run` in another world's tick space, advanced only once its system's body ran.
#[derive(Debug, Default)]
pub struct ForeignTicks {
    last_run: Option<Tick>,
    pending: Option<Tick>,
}

impl ForeignTicks {
    /// Takes this run's ticks from `world`'s counter and holds `this_run` until [`commit`](Self::commit).
    pub fn begin(&mut self, world: &World) -> SystemTicks {
        let this_run = world.increment_change_tick();
        let last_run = match &mut self.last_run {
            Some(last_run) => {
                last_run.check_tick(CheckChangeTicks(this_run));
                *last_run
            }
            None => this_run.relative_to(Tick::MAX),
        };
        self.pending = Some(this_run);
        SystemTicks { last_run, this_run }
    }

    /// Makes the last [`begin`](Self::begin)'s `this_run` the next `last_run`, from [`SystemInput::ran`].
    pub fn commit(&mut self) {
        if let Some(this_run) = self.pending.take() {
            self.last_run = Some(this_run);
        }
    }
}

#[derive(Debug, Deref, DerefMut)]
pub struct SystemLocal<'s, T: FromWorld + Send + 'static>(pub(crate) &'s mut T);

impl<'s, T: FromWorld + Send + 'static> SystemInput for SystemLocal<'s, T> {
    type State = SyncCell<T>;

    type Data<'world, 'state> = SystemLocal<'state, T>;

    fn init_state(world: &mut World) -> Self::State {
        SyncCell::new(T::from_world(world))
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        _world: UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        Ok(SystemLocal(state.get()))
    }

    fn fill_access(_meta: &mut SystemMetadata, _access: &mut SystemAccess) {}
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::resource::Resource;

    #[derive(Resource)]
    pub(crate) struct GateOpen(pub bool);

    pub(crate) struct Gate;

    impl SystemInput for Gate {
        type State = ();
        type Data<'world, 'state> = Gate;

        fn init_state(_world: &mut World) -> Self::State {}

        unsafe fn get_data<'world, 'state>(
            _state: &'state mut Self::State,
            world: UnsafeWorldCell<'world>,
            _ticks: SystemTicks,
        ) -> Result<Self::Data<'world, 'state>, ValidationError> {
            // SAFETY: reads only `GateOpen`, which `fill_access` declares.
            let open = unsafe { world.world() }
                .get_resource::<GateOpen>()
                .is_some_and(|open| open.0);
            if open {
                Ok(Gate)
            } else {
                Err(ValidationError::skip::<Self>("the gate is closed"))
            }
        }

        fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
            access.read_resource::<GateOpen>();
        }
    }
}

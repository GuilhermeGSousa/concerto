mod component_reader;
pub use component_reader::ComponentMetadata;

use std::{
    borrow::Cow,
    fmt,
    ops::{Deref, DerefMut},
};

use crate::{
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

    /// Builds the input's data for one run, or declines with why.
    ///
    /// # Safety
    /// `world` may be used only for what [`SystemInput::fill_access`] declared, and no
    /// running system may hold access that conflicts with it.
    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Result<Self::Data<'world, 'state>, ValidationError>;

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
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: the tuple's access is the union of its parts', which the caller upholds.
        Ok(typle_for!(i in .. => unsafe { <T<{i}>>::get_data(&mut state[[i]], world) }?))
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
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares exactly `P`'s access.
        unsafe { P::get_data(state, world) }.map(StaticSystemInput)
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        P::fill_access(meta, access);
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

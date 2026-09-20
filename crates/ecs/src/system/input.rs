use std::ops::{Deref, DerefMut};

use crate::{
    system::{access::SystemAccess, meta::SystemMetadata},
    utilities::SyncCell,
    world::{FromWorld, UnsafeWorldCell, World},
};
use derive_more::{Deref, DerefMut};
use typle::typle;

pub trait SystemInput {
    type State: Send + Sync + Sized;
    type Data<'world, 'state>;

    fn init_state(world: &mut World) -> Self::State;

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state>;

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

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        typle_for!(i in .. => <T<{i}>>::get_data(&mut state[[i]], world))
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        typle_for!(i in .. => <T<{i}>>::fill_access(meta, access));
    }
}

pub type SystemInputData<'w, 's, P> = <P as SystemInput>::Data<'w, 's>;

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

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        StaticSystemInput(P::get_data(state, world))
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

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        _world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        SystemLocal(state.get())
    }

    fn fill_access(_meta: &mut SystemMetadata, _access: &mut SystemAccess) {}
}

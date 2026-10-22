use std::any::TypeId;

use crate::{
    Component, Entity, World, archetype::Archetype, component::ComponentId,
    query::change_detection::Ref,
};

use typle::typle;

pub trait WorldQuery {
    type State: Send + Sync + Sized;

    fn init_state(world: &mut World) -> Self::State;

    fn matches(state: &Self::State, archetype: &Archetype) -> bool;
}

impl<T: Component> WorldQuery for &T {
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(state: &Self::State, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }
}

impl<T: Component> WorldQuery for &mut T {
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(state: &Self::State, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }
}

impl<T: Component> WorldQuery for Option<&T> {
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(_state: &Self::State, _archetype: &Archetype) -> bool {
        true
    }
}

impl<T: Component> WorldQuery for Option<&mut T> {
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(_state: &Self::State, _archetype: &Archetype) -> bool {
        true
    }
}

impl<T: Component> WorldQuery for Ref<'_, T> {
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(state: &Self::State, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }
}

impl WorldQuery for Entity {
    type State = ();

    fn init_state(_world: &mut World) -> Self::State {}

    fn matches(_state: &Self::State, _archetype: &Archetype) -> bool {
        true
    }
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> WorldQuery for T
where
    T: Tuple,
    T<_>: WorldQuery,
{
    type State = typle_for!(i in .. => T<{i}>::State);

    fn init_state(world: &mut World) -> Self::State {
        typle_for!(i in .. => <T<{i}>>::init_state(world))
    }

    fn matches(state: &Self::State, archetype: &Archetype) -> bool {
        let mut result = true;

        for typle_index!(i) in 0..T::LEN {
            result &= <T<{ i }>>::matches(&state[[i]], archetype);
        }

        result
    }
}

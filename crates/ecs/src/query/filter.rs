use std::{any::TypeId, marker::PhantomData};

use crate::{
    Component, World,
    archetype::Archetype,
    component::{ComponentId, bundle::ComponentBundle},
    entity::Entity,
    query::world_query::WorldQuery,
    world::UnsafeWorldCell,
};
use typle::typle;

/// Restricts which entities a [`Query`](super::Query) visits.
///
/// Multiple filters can be combined in a tuple: `(With<A>, Without<B>)` matches
/// entities that have `A` but not `B`.  [`Or`] can be used for disjunctions.
pub trait QueryFilter: WorldQuery {
    fn filter<'w>(_world: UnsafeWorldCell<'w>, _entity: Entity) -> bool {
        true
    }
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> QueryFilter for T
where
    T: Tuple,
    T<_>: QueryFilter,
{
    fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity) -> bool {
        for typle_index!(i) in 0..T::LEN {
            if !T::<{ i }>::filter(world, entity) {
                return false;
            }
        }
        true
    }
}

/// Matches entities where the given components were **added** this tick.
pub struct Added<T: Component> {
    _marker: PhantomData<T>,
}

impl<T> WorldQuery for Added<T>
where
    T: Component,
{
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(state: &Self::State, archetype: &crate::archetype::Archetype) -> bool {
        archetype.contains(*state)
    }
}

impl<T> QueryFilter for Added<T>
where
    T: Component,
{
    fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity) -> bool {
        for component_id in T::get_component_ids() {
            if !world.world().was_component_added(entity, component_id) {
                return false;
            }
        }
        true
    }
}

/// Matches entities where the given components were **mutated** this tick.
pub struct Changed<T: Component> {
    _marker: PhantomData<T>,
}

impl<T> WorldQuery for Changed<T>
where
    T: Component,
{
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(state: &Self::State, archetype: &crate::archetype::Archetype) -> bool {
        archetype.contains(*state)
    }
}

impl<T> QueryFilter for Changed<T>
where
    T: Component,
{
    fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity) -> bool {
        for component_id in T::get_component_ids() {
            if !world.world().was_component_changed(entity, component_id) {
                return false;
            }
        }
        true
    }
}

/// Matches entities that **have** all of the given components, without fetching them.
pub struct With<T: Component> {
    _marker: PhantomData<T>,
}

impl<T> WorldQuery for With<T>
where
    T: Component,
{
    type State = ComponentId;

    fn init_state(world: &mut World) -> Self::State {
        world.register_component::<T>();
        TypeId::of::<T>()
    }

    fn matches(state: &Self::State, archetype: &crate::archetype::Archetype) -> bool {
        archetype.contains(*state)
    }
}

impl<T> QueryFilter for With<T>
where
    T: Component,
{
    fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity) -> bool {
        let world = world.world();

        match world.entity_store().find_location(entity) {
            Some(location) => {
                let archetypes = world.archetypes();
                let archetype = &archetypes[location.archetype_index as usize];
                let archetype_ids = archetype.component_ids();
                let component_ids = T::get_component_ids();

                component_ids.iter().all(|id| archetype_ids.contains(id))
            }
            None => false,
        }
    }
}

/// Inverts a [`QueryFilter`]: matches entities that do **not** satisfy `T`.
pub struct Not<T: QueryFilter> {
    _marker: PhantomData<T>,
}

impl<T> WorldQuery for Not<T>
where
    T: QueryFilter,
{
    type State = T::State;

    fn init_state(world: &mut crate::World) -> Self::State {
        T::init_state(world)
    }

    fn matches(state: &Self::State, archetype: &crate::archetype::Archetype) -> bool {
        !T::matches(state, archetype)
    }
}

impl<T> QueryFilter for Not<T>
where
    T: QueryFilter,
{
    fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity) -> bool {
        !T::filter(world, entity)
    }
}

/// Matches entities that **do not** have the given component(s). Alias for `Not<With<T>>`.
pub type Without<T> = Not<With<T>>;

/// Matches entities that satisfy **at least one** of the filters in the tuple `T`.
///
/// # Example
/// ```ignore
/// // Entities that have either Health or Shield
/// Query<Entity, Or<(With<Health>, With<Shield>)>>
/// ```
pub struct Or<T: QueryFilter> {
    _marker: PhantomData<T>,
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> WorldQuery for Or<T>
where
    T: Tuple,
    T<_>: QueryFilter,
{
    type State = typle_for!(i in .. => <T<{i}> as WorldQuery>::State);

    fn init_state(world: &mut World) -> Self::State {
        typle_for!(i in .. => <T<{i}> as WorldQuery>::init_state(world))
    }

    fn matches(state: &Self::State, archetype: &Archetype) -> bool {
        let mut result = false;

        for typle_index!(i) in 0..T::LEN {
            result |= <T<{ i }>>::matches(&state[[i]], archetype);
        }

        result
    }

    fn refresh_state(state: &mut Self::State) -> bool {
        let mut changed = false;

        for typle_index!(i) in 0..T::LEN {
            changed |= <T<{ i }>>::refresh_state(&mut state[[i]]);
        }

        changed
    }
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> QueryFilter for Or<T>
where
    T: Tuple,
    T<_>: QueryFilter,
{
    fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity) -> bool {
        let mut result = false;

        for typle_index!(i) in 0..T::LEN {
            result |= <T<{ i }>>::filter(world, entity);
        }

        result
    }
}

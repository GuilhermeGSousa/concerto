use std::{any::TypeId, marker::PhantomData};

use crate::{
    Component, World,
    archetype::Archetype,
    component::{ComponentId, SystemTicks, bundle::ComponentBundle},
    entity::Entity,
    query::world_query::WorldQuery,
    system::access::SystemAccess,
    world::UnsafeWorldCell,
};
use typle::typle;

/// Restricts which entities a [`Query`](super::Query) visits.
///
/// Multiple filters can be combined in a tuple: `(With<A>, Without<B>)` matches
/// entities that have `A` but not `B`.  [`Or`] can be used for disjunctions.
pub trait QueryFilter: WorldQuery {
    /// Whether `entity` passes the filter.
    ///
    /// # Safety
    /// The caller must hold read access to the components the filter inspects.
    unsafe fn filter<'w>(
        _world: UnsafeWorldCell<'w>,
        _entity: Entity,
        _ticks: SystemTicks,
    ) -> bool {
        true
    }

    /// Registers what [`QueryFilter::filter`] reads with the scheduler's access tracker.
    fn fill_access(_access: &mut SystemAccess) {}
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> QueryFilter for T
where
    T: Tuple,
    T<_>: QueryFilter,
{
    unsafe fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity, ticks: SystemTicks) -> bool {
        for typle_index!(i) in 0..T::LEN {
            // SAFETY: the caller's access covers every filter in the tuple.
            if !unsafe { T::<{ i }>::filter(world, entity, ticks) } {
                return false;
            }
        }
        true
    }

    fn fill_access(access: &mut SystemAccess) {
        for typle_index!(i) in 0..T::LEN {
            <T<{ i }>>::fill_access(access);
        }
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
    unsafe fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity, ticks: SystemTicks) -> bool {
        for component_id in T::get_component_ids() {
            // SAFETY: `fill_access` declares read access to `T`, whose change ticks this reads.
            if !unsafe { world.world() }.was_component_added_in(entity, component_id, ticks) {
                return false;
            }
        }
        true
    }
    fn fill_access(access: &mut SystemAccess) {
        access.read_component::<T>();
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
    unsafe fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity, ticks: SystemTicks) -> bool {
        for component_id in T::get_component_ids() {
            // SAFETY: `fill_access` declares read access to `T`, whose change ticks this reads.
            if !unsafe { world.world() }.was_component_changed_in(entity, component_id, ticks) {
                return false;
            }
        }
        true
    }
    fn fill_access(access: &mut SystemAccess) {
        access.read_component::<T>();
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
    unsafe fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity, _ticks: SystemTicks) -> bool {
        // SAFETY: only the entity's archetype is read, under the caller's access.
        let world = unsafe { world.world() };

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
    unsafe fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity, ticks: SystemTicks) -> bool {
        // SAFETY: `Not` inspects what `T` inspects, which the caller covers.
        !unsafe { T::filter(world, entity, ticks) }
    }

    fn fill_access(access: &mut SystemAccess) {
        T::fill_access(access);
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
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> QueryFilter for Or<T>
where
    T: Tuple,
    T<_>: QueryFilter,
{
    unsafe fn filter<'w>(world: UnsafeWorldCell<'w>, entity: Entity, ticks: SystemTicks) -> bool {
        let mut result = false;

        for typle_index!(i) in 0..T::LEN {
            // SAFETY: the caller's access covers every filter in `Or`.
            result |= unsafe { <T<{ i }>>::filter(world, entity, ticks) };
        }

        result
    }

    fn fill_access(access: &mut SystemAccess) {
        for typle_index!(i) in 0..T::LEN {
            <T<{ i }>>::fill_access(access);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Query,
        system::{input::SystemInput, meta::SystemMetadata},
    };

    #[derive(crate::Component)]
    struct A;

    fn access<I: SystemInput>() -> SystemAccess {
        let mut access = SystemAccess::default();
        I::fill_access(&mut SystemMetadata::default(), &mut access);
        access
    }

    fn conflicts_with_writing_a<I: SystemInput>() -> bool {
        !SystemAccess::are_disjoint(&access::<I>(), &access::<Query<&mut A>>())
    }

    #[test]
    fn change_filters_read_their_component_so_they_conflict_with_a_writer() {
        assert!(conflicts_with_writing_a::<Query<Entity, Changed<A>>>());
        assert!(conflicts_with_writing_a::<Query<Entity, Added<A>>>());
        assert!(conflicts_with_writing_a::<
            Query<Entity, (Changed<A>, With<A>)>,
        >());
        assert!(conflicts_with_writing_a::<
            Query<Entity, Or<(Changed<A>, With<A>)>>,
        >());
        assert!(conflicts_with_writing_a::<Query<Entity, Not<Changed<A>>>>());
    }

    #[test]
    fn a_presence_filter_reads_only_archetypes_and_runs_beside_a_writer() {
        assert!(!conflicts_with_writing_a::<Query<Entity, With<A>>>());
        assert!(!conflicts_with_writing_a::<Query<Entity, Without<A>>>());
    }
}

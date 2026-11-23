use std::{any::TypeId, marker::PhantomData};

use crate::{
    Component, World,
    archetype::Archetype,
    component::{ComponentId, SystemTicks, bundle::ComponentBundle},
    query::{fetch::StampFetch, world_query::WorldQuery},
    system::access::{QueryShape, SystemAccess},
    world::UnsafeWorldCell,
};
use typle::typle;

/// Restricts which entities a [`Query`](super::Query) visits.
///
/// Multiple filters can be combined in a tuple: `(With<A>, Without<B>)` matches
/// entities that have `A` but not `B`.  [`Or`] can be used for disjunctions.
pub trait QueryFilter: WorldQuery {
    /// What the filter resolves once from an archetype, to check its entities by row.
    type Fetch<'w>;

    /// Whether [`WorldQuery::matches`] alone decides the filter, so an entity of a matched archetype needs no check.
    const IS_ARCHETYPAL: bool;

    /// Resolves what the filter reads in the archetype at index `archetype`.
    ///
    /// # Safety
    /// The caller must hold read access to the components the filter inspects for as long as `'w`.
    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w>;

    /// Whether the entity at `row` of the archetype `fetch` was resolved from passes the filter.
    ///
    /// # Safety
    /// No item that writes a component the filter inspects may be alive for the entity at `row`.
    unsafe fn filter(fetch: &Self::Fetch<'_>, row: usize) -> bool;

    /// Registers what [`QueryFilter::filter`] reads with the scheduler's access tracker.
    fn fill_access(_access: &mut SystemAccess) {}

    /// Adds the components every match of this filter has, or lacks, to `shape`.
    fn fill_shape(_shape: &mut QueryShape) {}

    /// Adds the components every entity this filter rejects has, or lacks, to `shape`.
    fn fill_rejected_shape(_shape: &mut QueryShape) {}
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> QueryFilter for T
where
    T: Tuple,
    T<_>: QueryFilter,
{
    type Fetch<'w> = typle_for!(i in .. => <T<{i}> as QueryFilter>::Fetch<'w>);
    const IS_ARCHETYPAL: bool = typle_all!(i in .. => <T<{i}>>::IS_ARCHETYPAL);

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w> {
        typle_for!(i in .. => {
            // SAFETY: the caller's access covers every filter in the tuple.
            unsafe { <T<{i}>>::set_archetype(world, archetype, ticks) }
        })
    }

    unsafe fn filter(fetch: &Self::Fetch<'_>, row: usize) -> bool {
        for typle_index!(i) in 0..T::LEN {
            // SAFETY: the caller's guarantee covers every filter in the tuple.
            if !unsafe { T::<{ i }>::filter(&fetch[[i]], row) } {
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

    fn fill_shape(shape: &mut QueryShape) {
        for typle_index!(i) in 0..T::LEN {
            <T<{ i }>>::fill_shape(shape);
        }
    }
}

/// Matches entities where the given components were **added** since the system last ran.
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
    type Fetch<'w> = StampFetch<'w>;
    const IS_ARCHETYPAL: bool = false;

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w> {
        // SAFETY: `fill_access` declares read access to `T`, whose stamps this resolves.
        unsafe { StampFetch::new::<T>(world, archetype, ticks) }
    }

    unsafe fn filter(fetch: &Self::Fetch<'_>, row: usize) -> bool {
        // SAFETY: the caller rules out an item writing `T` for this row.
        unsafe { fetch.is_added(row) }
    }
    fn fill_access(access: &mut SystemAccess) {
        access.read_component::<T>();
    }

    fn fill_shape(shape: &mut QueryShape) {
        for component in T::get_component_ids() {
            shape.require(component);
        }
    }
}

/// Matches entities where the given components were **added or mutated** since the system last ran.
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
    type Fetch<'w> = StampFetch<'w>;
    const IS_ARCHETYPAL: bool = false;

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w> {
        // SAFETY: `fill_access` declares read access to `T`, whose stamps this resolves.
        unsafe { StampFetch::new::<T>(world, archetype, ticks) }
    }

    unsafe fn filter(fetch: &Self::Fetch<'_>, row: usize) -> bool {
        // SAFETY: the caller rules out an item writing `T` for this row.
        unsafe { fetch.is_changed(row) }
    }
    fn fill_access(access: &mut SystemAccess) {
        access.read_component::<T>();
    }

    fn fill_shape(shape: &mut QueryShape) {
        for component in T::get_component_ids() {
            shape.require(component);
        }
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
    type Fetch<'w> = bool;
    const IS_ARCHETYPAL: bool = true;

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        _ticks: SystemTicks,
    ) -> Self::Fetch<'w> {
        world
            .archetypes()
            .get(archetype)
            .is_some_and(|archetype| archetype.contains(TypeId::of::<T>()))
    }

    unsafe fn filter(fetch: &Self::Fetch<'_>, _row: usize) -> bool {
        *fetch
    }

    fn fill_shape(shape: &mut QueryShape) {
        for component in T::get_component_ids() {
            shape.require(component);
        }
    }

    fn fill_rejected_shape(shape: &mut QueryShape) {
        if let [component] = T::get_component_ids()[..] {
            shape.exclude(component);
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
    type Fetch<'w> = T::Fetch<'w>;
    const IS_ARCHETYPAL: bool = T::IS_ARCHETYPAL;

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w> {
        // SAFETY: `Not` inspects what `T` inspects, which the caller covers.
        unsafe { T::set_archetype(world, archetype, ticks) }
    }

    unsafe fn filter(fetch: &Self::Fetch<'_>, row: usize) -> bool {
        // SAFETY: `Not` inspects what `T` inspects, which the caller covers.
        !unsafe { T::filter(fetch, row) }
    }

    fn fill_access(access: &mut SystemAccess) {
        T::fill_access(access);
    }

    fn fill_shape(shape: &mut QueryShape) {
        T::fill_rejected_shape(shape);
    }

    fn fill_rejected_shape(shape: &mut QueryShape) {
        T::fill_shape(shape);
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
    type Fetch<'w> = typle_for!(i in .. => <T<{i}> as QueryFilter>::Fetch<'w>);
    const IS_ARCHETYPAL: bool = typle_all!(i in .. => <T<{i}>>::IS_ARCHETYPAL);

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w> {
        typle_for!(i in .. => {
            // SAFETY: the caller's access covers every filter in `Or`.
            unsafe { <T<{i}>>::set_archetype(world, archetype, ticks) }
        })
    }

    unsafe fn filter(fetch: &Self::Fetch<'_>, row: usize) -> bool {
        let mut result = false;

        for typle_index!(i) in 0..T::LEN {
            // SAFETY: the caller's guarantee covers every filter in `Or`.
            result |= unsafe { <T<{ i }>>::filter(&fetch[[i]], row) };
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
        Entity, Query,
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
    fn a_filter_conflicts_with_a_writer_only_when_it_reads_the_component() {
        assert!(conflicts_with_writing_a::<Query<Entity, Changed<A>>>());
        assert!(conflicts_with_writing_a::<
            Query<Entity, Or<(Changed<A>, With<A>)>>,
        >());
        assert!(!conflicts_with_writing_a::<Query<Entity, With<A>>>());
    }
}

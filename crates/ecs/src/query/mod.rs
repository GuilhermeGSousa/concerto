use std::{any::TypeId, marker::PhantomData};

use fixedbitset::Ones;
use typle::typle;

pub mod change_detection;
pub mod filter;
pub mod state;
pub mod world_query;

use crate::{
    World,
    component::{Component, ComponentId, SystemTicks},
    entity::Entity,
    query::{
        change_detection::Mut, filter::QueryFilter, state::QueryState, world_query::WorldQuery,
    },
    system::{
        access::SystemAccess,
        input::{ReadOnlySystemInput, SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};

/// A type-safe view over all entities in a [`World`](crate::world::World) that
/// match a given set of components.
///
/// `T` is a [`QueryData`] that describes which components to fetch.  `F` is an
/// optional [`QueryFilter`] that further restricts which entities are visited.
///
/// Obtain a `Query` in a system by adding it as a parameter:
///
/// ```ignore
/// fn move_system(query: Query<(&mut Transform, &Velocity)>) {
///     for (transform, velocity) in query.iter() {
///         transform.translation += velocity.linear;
///     }
/// }
/// ```
pub struct Query<'world, 'state, T: QueryData, F: QueryFilter = ()> {
    world: UnsafeWorldCell<'world>,
    state: &'state QueryState<T, F>,
    ticks: SystemTicks,
}

/// Describes what data a [`Query`] fetches from each matching entity.
///
/// Implementations are provided for `&T`, `&mut T`, `Entity`, `Option<&T>`,
/// `Option<&mut T>`, and tuples of up to 12 elements.
pub trait QueryData: WorldQuery {
    /// The item type produced for each matched entity.
    type Item<'a>;

    /// Returns the [`ComponentId`]s that must be present on an entity for it to match.
    fn component_ids() -> Vec<ComponentId>;

    /// Fetches the data for `entity` from `world`, returning `None` if not possible.
    ///
    /// # Safety
    /// The caller must hold the access [`QueryData::fill_access`] declares.
    unsafe fn fetch<'w>(
        world: UnsafeWorldCell<'w>,
        entity: Entity,
        ticks: SystemTicks,
    ) -> Option<Self::Item<'w>>;

    /// Registers component access with the scheduler's access tracker.
    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess);
}

pub trait ReadOnlyQueryData: QueryData {}

impl<'world, 'state, T: QueryData, F: QueryFilter> Query<'world, 'state, T, F> {
    /// Constructs a new query for a run with `ticks` by scanning the world's archetypes.
    ///
    /// # Safety
    /// The caller must hold the access `T` and `F` declare for as long as the query lives.
    pub unsafe fn new(
        world: UnsafeWorldCell<'world>,
        state: &'state mut QueryState<T, F>,
        ticks: SystemTicks,
    ) -> Self {
        state.update_archetypes(world);

        Self {
            world,
            state,
            ticks,
        }
    }

    /// Returns an iterator over all matching entities.
    pub fn iter(&self) -> QueryIter<'world, 'state, T, F> {
        QueryIter {
            world: self.world,
            matched_archetypes: self.state.matched_archetypes(),
            current_entities: &[],
            current_row: 0,
            current_len: 0,
            ticks: self.ticks,
            _marker_data: PhantomData,
            _marker_filter: PhantomData,
        }
    }

    /// Fetches the query data for a specific entity, returning `None` if it doesn't match.
    pub fn get_entity(&self, entity: Entity) -> Option<T::Item<'world>> {
        // SAFETY: the query's system declared the access of `T` and `F`, and holds it while
        // the query lives.
        if !unsafe { F::filter(self.world, entity, self.ticks) } {
            return None;
        }
        // SAFETY: as above; the entity store is read only to locate `entity`.
        unsafe { self.world.world() }
            .entity_store()
            .find_location(entity)?;
        // SAFETY: as above.
        unsafe { T::fetch(self.world, entity, self.ticks) }
    }

    /// Returns `true` if `entity` matches this query.
    pub fn contains_entity(&self, entity: Entity) -> bool {
        self.get_entity(entity).is_some()
    }
}

pub struct QueryIter<'world, 'state, T, F> {
    world: UnsafeWorldCell<'world>,
    matched_archetypes: Ones<'state>,
    current_entities: &'world [Entity],
    current_row: usize,
    current_len: usize,
    ticks: SystemTicks,
    _marker_data: PhantomData<T>,
    _marker_filter: PhantomData<F>,
}

impl<'world, 's, T, F> Iterator for QueryIter<'world, 's, T, F>
where
    T: QueryData,
    F: QueryFilter,
{
    type Item = T::Item<'world>;

    fn next(&mut self) -> Option<Self::Item> {
        let archetypes = self.world.archetypes();
        loop {
            if self.current_row == self.current_len {
                let archetype_index = self.matched_archetypes.next()?;

                let archetype = &archetypes[archetype_index];

                self.current_row = 0;
                self.current_len = archetype.len();
                self.current_entities = archetype.entities();
            }

            if self.current_entities.is_empty() {
                continue;
            }

            let entity = self.current_entities[self.current_row];
            self.current_row += 1;
            // SAFETY: the query's system declared the access of `T` and `F`, and holds it
            // while the iterator lives.
            if !unsafe { F::filter(self.world, entity, self.ticks) } {
                continue;
            }

            // SAFETY: as above.
            return unsafe { T::fetch(self.world, entity, self.ticks) };
        }
    }
}

impl<T, F> SystemInput for Query<'_, '_, T, F>
where
    T: QueryData + 'static,
    F: QueryFilter + 'static,
{
    type State = QueryState<T, F>;
    type Data<'world, 'state> = Query<'world, 'state, T, F>;

    fn init_state(world: &mut World) -> Self::State {
        QueryState::<T, F>::new(world)
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares the access of `T` and `F`, which the caller holds.
        Ok(unsafe { Query::new(world, state, ticks) })
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        T::fill_access(meta, access);
        F::fill_access(access);
    }
}

impl<T, F> ReadOnlySystemInput for Query<'_, '_, T, F>
where
    T: ReadOnlyQueryData + 'static,
    F: QueryFilter + 'static,
{
}

impl<T> QueryData for &T
where
    T: Component,
{
    type Item<'w> = &'w T;

    fn component_ids() -> Vec<ComponentId> {
        vec![TypeId::of::<T>()]
    }

    unsafe fn fetch<'w>(
        world: UnsafeWorldCell<'w>,
        entity: Entity,
        _ticks: SystemTicks,
    ) -> Option<Self::Item<'w>> {
        // SAFETY: the caller holds read access to `T`, declared in `fill_access`.
        let world = unsafe { world.world() };
        world
            .entity_store()
            .find_location(entity)
            .and_then(|location| world.get_component_for_entity_location::<T>(location))
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_component::<T>();
    }
}

impl<T: Component> ReadOnlyQueryData for &T {}

impl<T> QueryData for &mut T
where
    T: Component,
{
    type Item<'w> = Mut<'w, T>;

    fn component_ids() -> Vec<ComponentId> {
        vec![TypeId::of::<T>()]
    }

    unsafe fn fetch<'w>(
        world: UnsafeWorldCell<'w>,
        entity: Entity,
        ticks: SystemTicks,
    ) -> Option<Self::Item<'w>> {
        // SAFETY: the caller holds write access to `T`, declared in `fill_access`.
        let world = unsafe { world.world_mut() };

        world
            .entity_store()
            .find_location(entity)
            .and_then(|location| {
                world
                    .get_component_for_entity_location_mut::<T>(location)
                    .map(|table_cell| Mut::new(table_cell.data, table_cell.changed_tick, ticks))
            })
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.write_component::<T>();
    }
}

impl QueryData for Entity {
    type Item<'a> = Entity;

    fn component_ids() -> Vec<ComponentId> {
        vec![]
    }

    unsafe fn fetch<'w>(
        _world: UnsafeWorldCell<'w>,
        entity: Entity,
        _ticks: SystemTicks,
    ) -> Option<Self::Item<'w>> {
        Some(entity)
    }

    fn fill_access(_meta: &mut SystemMetadata, _access: &mut SystemAccess) {}
}

impl ReadOnlyQueryData for Entity {}

impl<T> QueryData for Option<&T>
where
    T: Component,
{
    type Item<'w> = Option<&'w T>;

    fn component_ids() -> Vec<ComponentId> {
        vec![]
    }

    unsafe fn fetch<'w>(
        world: UnsafeWorldCell<'w>,
        entity: Entity,
        _ticks: SystemTicks,
    ) -> Option<Self::Item<'w>> {
        // SAFETY: the caller holds read access to `T`, declared in `fill_access`.
        let world = unsafe { world.world() };
        world
            .entity_store()
            .find_location(entity)
            .map(|location| world.get_component_for_entity_location(location))
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_component::<T>();
    }
}

impl<T> ReadOnlyQueryData for Option<&T> where T: Component {}

impl<T> QueryData for Option<&mut T>
where
    T: Component,
{
    type Item<'w> = Option<Mut<'w, T>>;

    fn component_ids() -> Vec<ComponentId> {
        vec![]
    }

    unsafe fn fetch<'w>(
        world: UnsafeWorldCell<'w>,
        entity: Entity,
        ticks: SystemTicks,
    ) -> Option<Self::Item<'w>> {
        // SAFETY: the caller holds write access to `T`, declared in `fill_access`.
        let world = unsafe { world.world_mut() };

        world.entity_store().find_location(entity).map(|location| {
            world
                .get_component_for_entity_location_mut::<T>(location)
                .map(|table_cell| Mut::new(table_cell.data, table_cell.changed_tick, ticks))
        })
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.write_component::<T>();
    }
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> QueryData for T
where
    T: Tuple,
    T<_>: QueryData,
{
    type Item<'w> = typle_for!(i in .. => T<{i}>::Item<'w>);

    #[allow(clippy::let_and_return)]
    fn component_ids() -> Vec<ComponentId> {
        {
            let mut res = Vec::new();

            for typle_index!(i) in 0..T::LEN {
                res.extend(T::<{ i }>::component_ids());
            }

            res
        }
    }

    unsafe fn fetch<'w>(
        world: UnsafeWorldCell<'w>,
        entity: Entity,
        ticks: SystemTicks,
    ) -> Option<Self::Item<'w>> {
        Some(typle_for!(i in .. => {
                unsafe { <T<{i}>>::fetch(world, entity, ticks) }?
            }
        ))
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        for typle_index!(i) in 0..T::LEN {
            <T<{ i }>>::fill_access(meta, access);
        }
    }
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> ReadOnlyQueryData for T
where
    T: Tuple,
    T<_>: ReadOnlyQueryData,
{
}

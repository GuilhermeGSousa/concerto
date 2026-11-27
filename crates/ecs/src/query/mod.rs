use std::any::TypeId;

use fixedbitset::Ones;
use typle::typle;

/// [`Ref`] and [`Mut`], component borrows that report and record changes.
pub mod change_detection;
/// How a query reads each kind of item from an archetype.
pub mod fetch;
/// Query filters: [`With`](filter::With), [`Without`](filter::Without), [`Added`](filter::Added),
/// [`Changed`](filter::Changed) and [`Or`](filter::Or).
pub mod filter;
/// [`QueryState`], a query's cached matching archetypes, for use outside systems.
pub mod state;
/// [`WorldQuery`], how a query term registers itself and picks archetypes.
pub mod world_query;

use crate::{
    World,
    component::{Component, ComponentId, SystemTicks},
    entity::{Entity, EntityLocation},
    query::{
        change_detection::{Mut, Ref},
        fetch::{RefFetch, WriteFetch},
        filter::QueryFilter,
        state::QueryState,
        world_query::WorldQuery,
    },
    system::{
        access::{QueryShape, SystemAccess},
        input::{ReadOnlySystemInput, SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};

/// A type-safe view over all entities in a [`World`] that
/// match a given set of components.
///
/// `T` is a [`QueryData`] that describes which components to fetch.  `F` is an
/// optional [`QueryFilter`] that further restricts which entities are visited.
///
/// Obtain a `Query` in a system by adding it as a parameter:
///
/// ```ignore
/// fn move_system(mut query: Query<(&mut Transform, &Velocity)>) {
///     for (mut transform, velocity) in query.iter_mut() {
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

    /// What the data resolves once from an archetype, to fetch its entities by row.
    type Fetch<'w>;

    /// Whether fetching this data writes nothing, so it may be fetched through a shared borrow of the query.
    const IS_READ_ONLY: bool;

    /// Returns the [`ComponentId`]s that must be present on an entity for it to match.
    fn component_ids() -> Vec<ComponentId>;

    /// Resolves the data's columns in the archetype at index `archetype`, or `None` if it lacks a required one.
    ///
    /// # Safety
    /// The caller must hold the access [`QueryData::fill_access`] declares for as long as `'w`.
    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>>;

    /// Fetches the data of `entity`, which sits at `row` of the archetype `fetch` was resolved from.
    ///
    /// # Safety
    /// No other item fetched for this row may be alive if the data writes.
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, entity: Entity, row: usize) -> Self::Item<'w>;

    /// Registers component access with the scheduler's access tracker.
    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess);
}

/// A [`QueryData`] that only reads, so its items may be fetched through a shared borrow.
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

    pub(crate) fn into_iter_exclusive(self) -> QueryIter<'world, 'state, T, F> {
        QueryIter::new(self.world, self.state, self.ticks)
    }

    /// Returns an iterator over all matching entities.
    ///
    /// Only a query whose data is read-only has it; use [`Self::iter_mut`] on one that writes.
    pub fn iter(&self) -> QueryIter<'_, 'state, T, F>
    where
        T: ReadOnlyQueryData,
    {
        const {
            assert!(
                T::IS_READ_ONLY,
                "Query::iter needs read-only data; use iter_mut on a query that writes"
            )
        };
        QueryIter::new(self.world, self.state, self.ticks)
    }

    /// Returns an iterator over a read-only query whose items live as long as the world borrow.
    pub fn iter_inner(&self) -> QueryIter<'world, 'state, T, F>
    where
        T: ReadOnlyQueryData,
    {
        QueryIter::new(self.world, self.state, self.ticks)
    }

    /// Returns an iterator over all matching entities, with write access to what `T` writes.
    pub fn iter_mut(&mut self) -> QueryIter<'_, 'state, T, F> {
        QueryIter::new(self.world, self.state, self.ticks)
    }

    /// Fetches the query data for a specific entity, returning `None` if it doesn't match.
    ///
    /// Only a query whose data is read-only has it; use [`Self::get_entity_mut`] on one that writes.
    pub fn get_entity(&self, entity: Entity) -> Option<T::Item<'_>>
    where
        T: ReadOnlyQueryData,
    {
        const {
            assert!(
                T::IS_READ_ONLY,
                "Query::get_entity needs read-only data; use get_entity_mut on a query that writes"
            )
        };
        // SAFETY: the data is read-only, so its items may share the query's borrow.
        unsafe { self.fetch(entity) }
    }

    /// # Safety
    /// No item this query fetched for `entity` through a shared borrow may be alive if `T` writes.
    unsafe fn fetch(&self, entity: Entity) -> Option<T::Item<'_>> {
        let location = self.matched_location(entity)?;
        if !self.passes(location) {
            return None;
        }
        // SAFETY: the query's system declared the access of `T` and holds it while the query lives.
        let data =
            unsafe { T::set_archetype(self.world, location.archetype_index as usize, self.ticks) }?;
        // SAFETY: the caller leaves no other item alive for this entity if `T` writes.
        Some(unsafe { T::fetch(&data, entity, *location.row) })
    }

    fn matched_location(&self, entity: Entity) -> Option<EntityLocation> {
        // SAFETY: the entity store is read only to locate `entity`, and no system changes it.
        unsafe { self.world.world() }
            .entity_store()
            .find_location(entity)
            .filter(|location| {
                self.state
                    .matches_archetype(location.archetype_index as usize)
            })
    }

    fn passes(&self, location: EntityLocation) -> bool {
        if F::IS_ARCHETYPAL {
            return true;
        }
        // SAFETY: the query's system declared the access of `F` and holds it while the query lives.
        let filter =
            unsafe { F::set_archetype(self.world, location.archetype_index as usize, self.ticks) };
        // SAFETY: a borrow of the query cannot coexist with an item of it that writes.
        unsafe { F::filter(&filter, *location.row) }
    }

    /// Fetches the query data for a specific entity, with write access to what `T` writes.
    pub fn get_entity_mut(&mut self, entity: Entity) -> Option<T::Item<'_>> {
        // SAFETY: the exclusive borrow of the query leaves no other item alive.
        unsafe { self.fetch(entity) }
    }

    /// Returns `true` if `entity` matches this query.
    pub fn contains_entity(&self, entity: Entity) -> bool {
        self.matched_location(entity)
            .is_some_and(|location| self.passes(location))
    }
}

struct ArchetypeFetch<'world, T: QueryData, F: QueryFilter> {
    data: T::Fetch<'world>,
    filter: F::Fetch<'world>,
}

/// An iterator over the items of a [`Query`] that pass its filter.
pub struct QueryIter<'world, 'state, T: QueryData, F: QueryFilter> {
    world: UnsafeWorldCell<'world>,
    matched_archetypes: Ones<'state>,
    current_entities: &'world [Entity],
    current_row: usize,
    current: Option<ArchetypeFetch<'world, T, F>>,
    ticks: SystemTicks,
}

impl<'world, 'state, T, F> QueryIter<'world, 'state, T, F>
where
    T: QueryData,
    F: QueryFilter,
{
    fn new(
        world: UnsafeWorldCell<'world>,
        state: &'state QueryState<T, F>,
        ticks: SystemTicks,
    ) -> Self {
        Self {
            world,
            matched_archetypes: state.matched_archetypes(),
            current_entities: &[],
            current_row: 0,
            current: None,
            ticks,
        }
    }

    fn enter(&mut self, archetype: usize) {
        self.current_row = 0;
        self.current_entities = &[];
        self.current = None;
        if self.world.archetypes()[archetype].is_empty() {
            return;
        }
        // SAFETY: the query's system declared the access of `F` and holds it while the iterator lives.
        let filter = unsafe { F::set_archetype(self.world, archetype, self.ticks) };
        // SAFETY: the query's system declared the access of `T` and holds it while the iterator lives.
        let Some(data) = (unsafe { T::set_archetype(self.world, archetype, self.ticks) }) else {
            return;
        };
        self.current_entities = self.world.archetypes()[archetype].entities();
        self.current = Some(ArchetypeFetch { data, filter });
    }
}

impl<'world, 's, T, F> Iterator for QueryIter<'world, 's, T, F>
where
    T: QueryData,
    F: QueryFilter,
{
    type Item = T::Item<'world>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_row == self.current_entities.len() {
                let archetype = self.matched_archetypes.next()?;
                self.enter(archetype);
                continue;
            }

            let row = self.current_row;
            self.current_row += 1;
            let current = self.current.as_ref()?;
            // SAFETY: the iterator has fetched no item for this row yet.
            if !F::IS_ARCHETYPAL && !unsafe { F::filter(&current.filter, row) } {
                continue;
            }

            // SAFETY: the iterator fetches each row once, and the query's borrow leaves no other item alive.
            return Some(unsafe { T::fetch(&current.data, self.current_entities[row], row) });
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
        let mut data = SystemAccess::default();
        T::fill_access(meta, &mut data);
        let mut filter = SystemAccess::default();
        F::fill_access(&mut filter);
        let mut shape = QueryShape::default();
        for component in T::component_ids() {
            shape.require(component);
        }
        F::fill_shape(&mut shape);
        access.add_query(data, filter, shape);
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
    type Fetch<'w> = &'w [T];
    const IS_READ_ONLY: bool = true;

    fn component_ids() -> Vec<ComponentId> {
        vec![TypeId::of::<T>()]
    }

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        _ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>> {
        // SAFETY: the caller holds read access to `T`, declared in `fill_access`.
        unsafe { world.column(archetype, TypeId::of::<T>()) }?.values::<T>()
    }

    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, _entity: Entity, row: usize) -> Self::Item<'w> {
        &fetch[row]
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
    type Fetch<'w> = WriteFetch<'w, T>;
    const IS_READ_ONLY: bool = false;

    fn component_ids() -> Vec<ComponentId> {
        vec![TypeId::of::<T>()]
    }

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>> {
        // SAFETY: the caller holds write access to `T`, declared in `fill_access`.
        unsafe { WriteFetch::new(world, archetype, ticks) }
    }

    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, _entity: Entity, row: usize) -> Self::Item<'w> {
        // SAFETY: the caller leaves no other item alive for this row.
        unsafe { fetch.get(row) }
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.write_component::<T>();
    }
}

impl<T> QueryData for Ref<'_, T>
where
    T: Component,
{
    type Item<'w> = Ref<'w, T>;
    type Fetch<'w> = RefFetch<'w, T>;
    const IS_READ_ONLY: bool = true;

    fn component_ids() -> Vec<ComponentId> {
        vec![TypeId::of::<T>()]
    }

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>> {
        // SAFETY: the caller holds read access to `T`, declared in `fill_access`.
        unsafe { RefFetch::new(world, archetype, ticks) }
    }

    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, _entity: Entity, row: usize) -> Self::Item<'w> {
        fetch.get(row)
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_component::<T>();
    }
}

impl<T: Component> ReadOnlyQueryData for Ref<'_, T> {}

impl QueryData for Entity {
    type Item<'a> = Entity;
    type Fetch<'w> = ();
    const IS_READ_ONLY: bool = true;

    fn component_ids() -> Vec<ComponentId> {
        vec![]
    }

    unsafe fn set_archetype<'w>(
        _world: UnsafeWorldCell<'w>,
        _archetype: usize,
        _ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>> {
        Some(())
    }

    unsafe fn fetch<'w>(_fetch: &Self::Fetch<'w>, entity: Entity, _row: usize) -> Self::Item<'w> {
        entity
    }

    fn fill_access(_meta: &mut SystemMetadata, _access: &mut SystemAccess) {}
}

impl ReadOnlyQueryData for Entity {}

impl<T> QueryData for Option<&T>
where
    T: Component,
{
    type Item<'w> = Option<&'w T>;
    type Fetch<'w> = Option<&'w [T]>;
    const IS_READ_ONLY: bool = true;

    fn component_ids() -> Vec<ComponentId> {
        vec![]
    }

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        _ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>> {
        // SAFETY: the caller holds read access to `T`, declared in `fill_access`.
        let column = unsafe { world.column(archetype, TypeId::of::<T>()) };
        Some(column.and_then(|column| column.values::<T>()))
    }

    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, _entity: Entity, row: usize) -> Self::Item<'w> {
        fetch.map(|values| &values[row])
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
    type Fetch<'w> = Option<WriteFetch<'w, T>>;
    const IS_READ_ONLY: bool = false;

    fn component_ids() -> Vec<ComponentId> {
        vec![]
    }

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>> {
        // SAFETY: the caller holds write access to `T`, declared in `fill_access`.
        Some(unsafe { WriteFetch::new(world, archetype, ticks) })
    }

    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, _entity: Entity, row: usize) -> Self::Item<'w> {
        // SAFETY: the caller leaves no other item alive for this row.
        fetch.as_ref().map(|fetch| unsafe { fetch.get(row) })
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
    type Fetch<'w> = typle_for!(i in .. => T<{i}>::Fetch<'w>);
    const IS_READ_ONLY: bool = typle_all!(i in .. => <T<{i}>>::IS_READ_ONLY);

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

    unsafe fn set_archetype<'w>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Option<Self::Fetch<'w>> {
        Some(typle_for!(i in .. => {
                // SAFETY: the caller's access covers every element of the tuple.
                unsafe { <T<{i}>>::set_archetype(world, archetype, ticks) }?
            }
        ))
    }

    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, entity: Entity, row: usize) -> Self::Item<'w> {
        typle_for!(i in .. => {
                // SAFETY: the caller's guarantee covers every element of the tuple.
                unsafe { <T<{i}>>::fetch(&fetch[[i]], entity, row) }
            }
        )
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        for typle_index!(i) in 0..T::LEN {
            let mut element = SystemAccess::default();
            <T<{ i }>>::fill_access(meta, &mut element);
            if let Some(component) = access.data_conflict(&element) {
                access.report_conflict(format!(
                    "one query both writes the component `{component}` and accesses it again"
                ));
            }
            access.combine(element);
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

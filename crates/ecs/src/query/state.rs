use fixedbitset::{FixedBitSet, Ones};

use crate::{
    Query, World,
    archetype::Archetype,
    query::{QueryData, QueryIter, filter::QueryFilter},
    world::UnsafeWorldCell,
};

pub struct QueryState<T: QueryData, F: QueryFilter = ()> {
    data_state: T::State,
    filter_state: F::State,
    current_archetype_count: usize,
    matched_archetypes: FixedBitSet,
}

impl<T: QueryData, F: QueryFilter> Clone for QueryState<T, F>
where
    T::State: Clone,
    F::State: Clone,
{
    fn clone(&self) -> Self {
        Self {
            data_state: self.data_state.clone(),
            filter_state: self.filter_state.clone(),
            current_archetype_count: self.current_archetype_count,
            matched_archetypes: self.matched_archetypes.clone(),
        }
    }
}

impl<T: QueryData, F: QueryFilter> QueryState<T, F> {
    pub(crate) fn new(world: &mut World) -> Self {
        Self {
            data_state: T::init_state(world),
            filter_state: F::init_state(world),
            current_archetype_count: 0,
            matched_archetypes: Default::default(),
        }
    }

    pub(crate) fn update_archetypes(&mut self, world: UnsafeWorldCell) {
        if world.archetypes().len() == self.current_archetype_count {
            return;
        }

        let archetypes = world.archetypes();

        for new_archetype in &archetypes[self.current_archetype_count..] {
            self.add_archetype(new_archetype);
        }

        self.current_archetype_count = world.archetypes().len();
    }

    pub(crate) fn matched_archetypes(&self) -> Ones<'_> {
        self.matched_archetypes.ones()
    }

    pub fn iter<'world, 'state>(
        &'state mut self,
        world: &'world mut World,
    ) -> QueryIter<'world, 'state, T, F> {
        let ticks = world.out_of_system_ticks();
        // SAFETY: the exclusive borrow of `world` covers any access.
        unsafe { Query::new(world.as_unsafe_world_cell_mut(), self, ticks) }.iter()
    }

    fn add_archetype(&mut self, archetype: &Archetype) {
        if !T::matches(&self.data_state, archetype) || !F::matches(&self.filter_state, archetype) {
            return;
        }

        if !self.matched_archetypes.contains(*archetype.id()) {
            self.matched_archetypes.grow_and_insert(*archetype.id());
        }
    }
}

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
        let data_changed = T::refresh_state(&mut self.data_state, world.world());
        let filter_changed = F::refresh_state(&mut self.filter_state, world.world());
        if data_changed || filter_changed {
            self.matched_archetypes.clear();
            self.current_archetype_count = 0;
        }

        if world.archetypes().len() == self.current_archetype_count {
            return;
        }

        let archetypes = world.archetypes();

        for new_archetype in &archetypes[self.current_archetype_count..] {
            self.add_archetype(new_archetype);
        }

        self.current_archetype_count = world.archetypes().len();
    }

    pub(crate) fn data_state(&self) -> &T::State {
        &self.data_state
    }

    pub(crate) fn matched_archetypes(&self) -> Ones<'_> {
        self.matched_archetypes.ones()
    }

    pub fn iter<'world, 'state>(
        &'state mut self,
        world: &'world mut World,
    ) -> QueryIter<'world, 'state, T, F> {
        Query::new(world.as_unsafe_world_cell_mut(), self).iter()
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

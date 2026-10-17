use fixedbitset::{FixedBitSet, Ones};

use crate::{
    Query, World,
    archetype::Archetype,
    query::{QueryData, QueryIter, filter::QueryFilter},
    world::UnsafeWorldCell,
    world_id::WorldId,
};

pub struct QueryState<T: QueryData, F: QueryFilter = ()> {
    data_state: T::State,
    filter_state: F::State,
    cached_for: Option<WorldId>,
    current_archetype_count: usize,
    matched_archetypes: FixedBitSet,
}

impl<T: QueryData, F: QueryFilter> QueryState<T, F> {
    pub(crate) fn new(world: &mut World) -> Self {
        Self {
            data_state: T::init_state(world),
            filter_state: F::init_state(world),
            cached_for: None,
            current_archetype_count: 0,
            matched_archetypes: Default::default(),
        }
    }

    pub(crate) fn update_archetypes(&mut self, world: UnsafeWorldCell) {
        let id = world.world().id();
        if self.cached_for != Some(id) {
            self.cached_for = Some(id);
            self.current_archetype_count = 0;
            self.matched_archetypes.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Component;

    #[derive(Component)]
    struct A(u32);

    #[derive(Component)]
    struct B;

    fn values(state: &mut QueryState<&'static A>, world: &mut World) -> Vec<u32> {
        let mut values: Vec<u32> = state.iter(world).map(|a| a.0).collect();
        values.sort_unstable();
        values
    }

    fn state_warmed_on(world: &mut World) -> QueryState<&'static A> {
        let mut state = QueryState::<&A>::new(world);
        values(&mut state, world);
        state
    }

    #[test]
    fn a_state_follows_a_world_with_the_same_number_of_archetypes() {
        let mut first = World::new();
        first.spawn(B);
        first.spawn(A(1));
        let mut second = World::new();
        second.spawn(A(2));
        second.spawn(B);
        let mut state = state_warmed_on(&mut first);

        assert_eq!(values(&mut state, &mut second), vec![2]);
    }

    #[test]
    fn a_state_follows_a_world_with_fewer_archetypes() {
        let mut first = World::new();
        first.spawn(A(1));
        first.spawn((A(3), B));
        first.spawn(B);
        let mut second = World::new();
        second.spawn(A(2));
        let mut state = state_warmed_on(&mut first);

        assert_eq!(values(&mut state, &mut second), vec![2]);
    }

    #[test]
    fn a_state_follows_a_world_with_more_archetypes() {
        let mut first = World::new();
        first.spawn(A(1));
        let mut second = World::new();
        second.spawn(B);
        second.spawn((A(2), B));
        second.spawn(A(4));
        let mut state = state_warmed_on(&mut first);

        assert_eq!(values(&mut state, &mut second), vec![2, 4]);
    }

    #[test]
    fn a_state_still_reads_its_first_world_after_visiting_another() {
        let mut first = World::new();
        first.spawn(A(1));
        let mut second = World::new();
        second.spawn(B);
        second.spawn(A(2));
        let mut state = state_warmed_on(&mut first);
        values(&mut state, &mut second);

        assert_eq!(values(&mut state, &mut first), vec![1]);
    }

    #[test]
    fn two_worlds_have_different_ids_and_a_world_keeps_its_own() {
        let first = World::new();
        let second = World::new();

        assert_ne!(first.id(), second.id());
        assert_eq!(first.id(), first.id());
    }
}

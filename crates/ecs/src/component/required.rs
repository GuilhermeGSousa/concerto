use std::{collections::HashSet, sync::Arc};

use super::components::Components;
use super::{Component, ComponentId};
use crate::{
    archetype::Archetype,
    table::{Table, TableRowIndex},
    utilities::TypeIdMap,
};

type WriteRequired = Arc<dyn Fn(&mut Archetype, TableRowIndex, u32) + Send + Sync>;
type RegisterRequired = fn(&mut Components, &mut Vec<(ComponentId, &'static str)>);

/// One required component, with the erased functions that add its column and write its default.
#[derive(Clone)]
pub struct RequiredComponent {
    id: ComponentId,
    name: &'static str,
    add_column: fn(&mut Table),
    write: WriteRequired,
    register: RegisterRequired,
}

impl RequiredComponent {
    /// The required component's id.
    pub fn id(&self) -> ComponentId {
        self.id
    }

    /// The required component's type name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Adds an empty column for the required component to `table`.
    pub fn add_column(&self, table: &mut Table) {
        (self.add_column)(table);
    }

    /// Pushes a freshly built value for `row` unless the column already holds one there.
    pub fn write(&self, archetype: &mut Archetype, row: TableRowIndex, tick: u32) {
        (self.write)(archetype, row, tick);
    }
}

/// The components a component type requires, filled by `Component::register_required`.
#[derive(Default, Clone)]
pub struct RequiredComponents {
    entries: Vec<RequiredComponent>,
}

impl RequiredComponents {
    /// Declares that the owning component requires `T`, built by `constructor` when absent.
    pub fn register<T: Component>(&mut self, constructor: fn() -> T) {
        if self.contains(ComponentId::of::<T>()) {
            return;
        }
        self.set(constructor);
    }

    pub(crate) fn set<T: Component>(&mut self, constructor: fn() -> T) {
        let id = ComponentId::of::<T>();
        let entry = RequiredComponent {
            id,
            name: T::name(),
            add_column: Table::add_column::<T>,
            write: Arc::new(move |archetype, row, tick| {
                if !archetype.has_value_at::<T>(row) {
                    archetype.push_component(constructor(), tick);
                }
            }),
            register: Components::register_required_in::<T>,
        };
        match self.entries.iter_mut().find(|existing| existing.id == id) {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
    }

    pub(crate) fn contains(&self, id: ComponentId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }

    /// The required components, deepest requirement first.
    pub fn iter(&self) -> impl Iterator<Item = &RequiredComponent> {
        self.entries.iter()
    }

    /// The number of required components.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is required.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn register_all(
        &self,
        components: &mut Components,
        stack: &mut Vec<(ComponentId, &'static str)>,
    ) {
        for entry in &self.entries {
            (entry.register)(components, stack);
        }
    }

    pub(crate) fn flatten<'a>(
        &self,
        tables: &impl Fn(ComponentId) -> Option<&'a RequiredComponents>,
    ) -> RequiredComponents {
        let mut chosen: TypeIdMap<RequiredComponent> = TypeIdMap::default();
        for entry in &self.entries {
            chosen.insert(entry.id, entry.clone());
        }
        for entry in &self.entries {
            if let Some(table) = tables(entry.id) {
                for deeper in &table.entries {
                    chosen.entry(deeper.id).or_insert_with(|| deeper.clone());
                }
            }
        }
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for entry in &self.entries {
            visit(entry.id, tables, &chosen, &mut seen, &mut entries);
        }
        RequiredComponents { entries }
    }
}

fn visit<'a>(
    id: ComponentId,
    tables: &impl Fn(ComponentId) -> Option<&'a RequiredComponents>,
    chosen: &TypeIdMap<RequiredComponent>,
    seen: &mut HashSet<ComponentId>,
    out: &mut Vec<RequiredComponent>,
) {
    if !seen.insert(id) {
        return;
    }
    if let Some(table) = tables(id) {
        for deeper in &table.entries {
            visit(deeper.id, tables, chosen, seen, out);
        }
    }
    out.push(chosen[&id].clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[derive(Component, Default)]
    struct Leaf;

    #[derive(Component, Default)]
    struct Mid(u32);

    #[derive(Component, Default)]
    #[require(Leaf)]
    struct Chain;

    #[derive(Component, Default)]
    #[require(Mid, Leaf)]
    struct Top;

    #[derive(Component, Default)]
    #[require(Chain)]
    struct Deep;

    #[derive(Component, Default)]
    #[require(Mid = Mid(1))]
    struct Near;

    #[derive(Component, Default)]
    #[require(Near, Mid = Mid(2))]
    struct Wins;

    #[derive(Component, Default)]
    #[require(CycleB)]
    struct CycleA;

    #[derive(Component, Default)]
    #[require(CycleA)]
    struct CycleB;

    #[derive(Component, Default)]
    #[require(SelfCycle)]
    struct SelfCycle;

    fn ids(world: &World, id: ComponentId) -> Vec<ComponentId> {
        world
            .required_components(id)
            .unwrap()
            .iter()
            .map(RequiredComponent::id)
            .collect()
    }

    #[test]
    fn unregistered_type_has_no_table() {
        let world = World::new();

        assert!(
            world
                .required_components(ComponentId::of::<Chain>())
                .is_none()
        );
    }

    #[test]
    fn component_without_requirements_has_an_empty_table() {
        let mut world = World::new();
        world.register_component::<Leaf>();

        assert!(
            world
                .required_components(ComponentId::of::<Leaf>())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn direct_requirement_is_listed() {
        let mut world = World::new();
        world.register_component::<Chain>();

        assert_eq!(
            ids(&world, ComponentId::of::<Chain>()),
            vec![ComponentId::of::<Leaf>()]
        );
    }

    #[test]
    fn requirements_are_flattened_deepest_first() {
        let mut world = World::new();
        world.register_component::<Deep>();

        assert_eq!(
            ids(&world, ComponentId::of::<Deep>()),
            vec![ComponentId::of::<Leaf>(), ComponentId::of::<Chain>()]
        );
    }

    #[test]
    fn duplicate_requirements_appear_once() {
        let mut world = World::new();
        world.register_component::<Top>();

        let listed = ids(&world, ComponentId::of::<Top>());

        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&ComponentId::of::<Mid>()));
        assert!(listed.contains(&ComponentId::of::<Leaf>()));
    }

    #[test]
    fn requiring_a_type_registers_its_table() {
        let mut world = World::new();
        world.register_component::<Deep>();

        assert!(
            world
                .required_components(ComponentId::of::<Chain>())
                .is_some()
        );
        assert!(
            world
                .required_components(ComponentId::of::<Leaf>())
                .is_some()
        );
    }

    #[test]
    fn nearest_constructor_wins() {
        let mut world = World::new();
        world.register_component::<Wins>();

        let entry = world
            .required_components(ComponentId::of::<Wins>())
            .unwrap()
            .iter()
            .find(|entry| entry.id() == ComponentId::of::<Mid>())
            .unwrap()
            .clone();
        let mut table = Table::new();
        entry.add_column(&mut table);
        let mut archetype = Archetype::new(
            table,
            vec![ComponentId::of::<Mid>()],
            crate::archetype::ArchetypeId::from(0),
        );
        entry.write(&mut archetype, TableRowIndex::new(0), 1);

        let value = unsafe { archetype.get_component_unsafe::<Mid>(TableRowIndex::new(0)) };
        assert_eq!(value.unwrap().0, 2);
    }

    #[test]
    fn write_skips_a_column_that_already_has_a_value() {
        let mut world = World::new();
        world.register_component::<Near>();
        let entry = world
            .required_components(ComponentId::of::<Near>())
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .clone();
        let mut table = Table::new();
        entry.add_column(&mut table);
        let mut archetype = Archetype::new(
            table,
            vec![ComponentId::of::<Mid>()],
            crate::archetype::ArchetypeId::from(0),
        );
        archetype.push_component(Mid(9), 1);

        entry.write(&mut archetype, TableRowIndex::new(0), 2);

        let value = unsafe { archetype.get_component_unsafe::<Mid>(TableRowIndex::new(0)) };
        assert_eq!(value.unwrap().0, 9);
    }

    #[test]
    #[should_panic(expected = "CycleA")]
    fn cycle_panics_naming_the_types() {
        let mut world = World::new();
        world.register_component::<CycleA>();
    }

    #[test]
    #[should_panic(expected = "SelfCycle")]
    fn self_requirement_panics() {
        let mut world = World::new();
        world.register_component::<SelfCycle>();
    }

    fn mid(world: &World, entity: crate::entity::Entity) -> Option<u32> {
        world
            .get_component_for_entity::<Mid>(entity)
            .map(|mid| mid.0)
    }

    #[test]
    fn spawn_adds_a_default_required_component() {
        let mut world = World::new();

        let entity = world.spawn(Chain);

        assert!(world.get_component_for_entity::<Leaf>(entity).is_some());
    }

    #[test]
    fn spawn_adds_transitive_requirements() {
        let mut world = World::new();

        let entity = world.spawn(Deep);

        assert!(world.get_component_for_entity::<Chain>(entity).is_some());
        assert!(world.get_component_for_entity::<Leaf>(entity).is_some());
    }

    #[test]
    fn spawn_uses_the_declared_constructor() {
        let mut world = World::new();

        let near = world.spawn(Near);
        let wins = world.spawn(Wins);

        assert_eq!(mid(&world, near), Some(1));
        assert_eq!(mid(&world, wins), Some(2));
    }

    #[test]
    fn bundle_value_wins_in_either_order() {
        let mut world = World::new();

        let first = world.spawn((Near, Mid(7)));
        let second = world.spawn((Mid(8), Near));

        assert_eq!(mid(&world, first), Some(7));
        assert_eq!(mid(&world, second), Some(8));
    }

    #[test]
    fn insert_adds_a_default_required_component() {
        let mut world = World::new();
        let entity = world.spawn(Leaf);

        world.insert(Near, entity);

        assert_eq!(mid(&world, entity), Some(1));
        assert!(world.get_component_for_entity::<Leaf>(entity).is_some());
    }

    #[test]
    fn insert_keeps_an_existing_required_component() {
        let mut world = World::new();
        let entity = world.spawn(Mid(7));

        world.insert(Near, entity);

        assert_eq!(mid(&world, entity), Some(7));
    }

    #[test]
    fn insert_bundle_value_wins_over_the_default() {
        let mut world = World::new();
        let entity = world.spawn(Leaf);

        world.insert((Mid(9), Near), entity);

        assert_eq!(mid(&world, entity), Some(9));
    }

    #[test]
    fn removing_the_requirer_leaves_the_required_component() {
        let mut world = World::new();
        let entity = world.spawn(Chain);

        world.remove_component::<Chain>(entity);

        assert!(world.get_component_for_entity::<Leaf>(entity).is_some());
    }

    #[test]
    fn added_matches_a_required_component() {
        use crate::query::filter::Added;

        let mut world = World::new();
        world.tick();
        world.spawn(Chain);

        let mut state = world.query::<crate::entity::Entity, Added<Leaf>>();

        assert_eq!(state.iter(&mut world).count(), 1);
    }

    #[test]
    fn requirer_inserted_before_the_required_value_ends_with_the_stored_value() {
        let mut world = World::new();
        let entity = world.spawn(Leaf);

        world.insert(Near, entity);
        world.insert(Mid(5), entity);

        assert_eq!(mid(&world, entity), Some(5));
    }
}

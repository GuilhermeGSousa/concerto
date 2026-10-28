use std::{collections::HashSet, sync::Arc};

use super::{Component, ComponentId};
use crate::{
    archetype::Archetype,
    table::{Table, TableRowIndex},
    utilities::TypeIdMap,
    world::World,
};

type WriteRequired = Arc<dyn Fn(&mut Archetype, TableRowIndex, u32) + Send + Sync>;
type RegisterRequired = fn(&mut World, &mut Vec<(ComponentId, &'static str)>);

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
        let id = ComponentId::of::<T>();
        if self.entries.iter().any(|entry| entry.id == id) {
            return;
        }
        self.entries.push(RequiredComponent {
            id,
            name: T::name(),
            add_column: Table::add_column::<T>,
            write: Arc::new(move |archetype, row, tick| {
                if !archetype.has_value_at::<T>(row) {
                    archetype.push_component(constructor(), tick);
                }
            }),
            register: World::register_required_in::<T>,
        });
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
        world: &mut World,
        stack: &mut Vec<(ComponentId, &'static str)>,
    ) {
        for entry in &self.entries {
            (entry.register)(world, stack);
        }
    }

    pub(crate) fn flatten(&self, tables: &TypeIdMap<RequiredComponents>) -> RequiredComponents {
        let mut chosen: TypeIdMap<RequiredComponent> = TypeIdMap::default();
        for entry in &self.entries {
            chosen.insert(entry.id, entry.clone());
        }
        for entry in &self.entries {
            if let Some(table) = tables.get(&entry.id) {
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

fn visit(
    id: ComponentId,
    tables: &TypeIdMap<RequiredComponents>,
    chosen: &TypeIdMap<RequiredComponent>,
    seen: &mut HashSet<ComponentId>,
    out: &mut Vec<RequiredComponent>,
) {
    if !seen.insert(id) {
        return;
    }
    if let Some(table) = tables.get(&id) {
        for deeper in &table.entries {
            visit(deeper.id, tables, chosen, seen, out);
        }
    }
    out.push(chosen[&id].clone());
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

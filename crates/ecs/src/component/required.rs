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
    constructor: usize,
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
            constructor: constructor as usize,
        };
        match self.entries.iter_mut().find(|existing| existing.id == id) {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
    }

    pub(crate) fn built_by<T: Component>(&self, constructor: fn() -> T) -> bool {
        let id = ComponentId::of::<T>();
        self.entries
            .iter()
            .any(|entry| entry.id == id && entry.constructor == constructor as usize)
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
    use crate::{entity::Entity, world::World};

    #[derive(Component, Default)]
    struct Leaf;

    #[derive(Component, Default)]
    struct Mid(u32);

    #[derive(Component, Default)]
    #[require(Leaf)]
    struct Chain;

    #[derive(Component, Default)]
    #[require(Chain)]
    struct Deep;

    #[derive(Component, Default)]
    #[require(Mid = Mid(1))]
    struct Near;

    #[derive(Component, Default)]
    #[require(Near, Mid = Mid(2))]
    struct Wins;

    #[derive(Debug, PartialEq)]
    struct Holds {
        mid: Option<u32>,
        leaf: bool,
        chain: bool,
    }

    struct SpawnCase {
        name: &'static str,
        build: fn(&mut World) -> Entity,
        holds: Holds,
    }

    fn holds(world: &World, entity: Entity) -> Holds {
        Holds {
            mid: world
                .get_component_for_entity::<Mid>(entity)
                .map(|mid| mid.0),
            leaf: world.get_component_for_entity::<Leaf>(entity).is_some(),
            chain: world.get_component_for_entity::<Chain>(entity).is_some(),
        }
    }

    #[test]
    fn spawn_and_insert_fill_in_missing_requirements_and_keep_given_values() {
        let cases = [
            SpawnCase {
                name: "spawn adds transitive requirements",
                build: |world| world.spawn(Deep),
                holds: Holds {
                    mid: None,
                    leaf: true,
                    chain: true,
                },
            },
            SpawnCase {
                name: "the nearest constructor wins",
                build: |world| world.spawn(Wins),
                holds: Holds {
                    mid: Some(2),
                    leaf: false,
                    chain: false,
                },
            },
            SpawnCase {
                name: "a bundle value before the requirer wins",
                build: |world| world.spawn((Mid(8), Near)),
                holds: Holds {
                    mid: Some(8),
                    leaf: false,
                    chain: false,
                },
            },
            SpawnCase {
                name: "insert adds a default requirement",
                build: |world| {
                    let entity = world.spawn(Leaf);
                    world.insert(Near, entity);
                    entity
                },
                holds: Holds {
                    mid: Some(1),
                    leaf: true,
                    chain: false,
                },
            },
            SpawnCase {
                name: "insert keeps a requirement already present",
                build: |world| {
                    let entity = world.spawn(Mid(7));
                    world.insert(Near, entity);
                    entity
                },
                holds: Holds {
                    mid: Some(7),
                    leaf: false,
                    chain: false,
                },
            },
        ];
        for case in cases {
            let mut world = World::new();

            let entity = (case.build)(&mut world);

            assert_eq!(holds(&world, entity), case.holds, "{}", case.name);
        }
    }

    #[test]
    fn added_matches_a_required_component() {
        use crate::query::filter::Added;

        let mut world = World::new();
        world.tick();
        world.spawn(Chain);

        let mut state = world.query::<Entity, Added<Leaf>>();

        assert_eq!(state.iter(&mut world).count(), 1);
    }
}

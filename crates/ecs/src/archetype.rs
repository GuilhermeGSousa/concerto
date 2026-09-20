use derive_more::{Deref, From};

use crate::{
    component::{Component, ComponentId, Tick},
    entity::Entity,
    table::{MutableCellAccessor, Table, TableRowIndex},
};

pub struct Archetype {
    data_table: Table,
    component_ids: Vec<ComponentId>,
    id: ArchetypeId,
}

impl Archetype {
    pub fn new(data_table: Table, component_ids: Vec<ComponentId>, id: ArchetypeId) -> Archetype {
        Archetype {
            data_table,
            component_ids,
            id,
        }
    }

    /// Whether `T`'s column already holds a value at `row`.
    pub fn has_value_at<T: Component>(&self, row: TableRowIndex) -> bool {
        self.data_table
            .get_column(ComponentId::of::<T>())
            .is_some_and(|column| *row < column.len())
    }

    /// Appends `value` to the end of `T`'s column.
    pub fn push_component<T: Component>(&mut self, value: T, current_tick: u32) {
        if let Some(column) = self.data_table.get_column_mut(ComponentId::of::<T>()) {
            column.push(value, current_tick);
        }
    }

    /// Overwrites the `T` already stored at `row`.
    pub fn insert_component<T: Component>(
        &mut self,
        value: T,
        current_tick: u32,
        row: TableRowIndex,
    ) {
        if let Some(column) = self.data_table.get_column_mut(ComponentId::of::<T>()) {
            column.insert(value, current_tick, row);
        }
    }

    pub fn add_entity(&mut self, entity: Entity) {
        self.data_table.add_entity(entity);
    }

    pub fn contains(&self, component_id: ComponentId) -> bool {
        self.data_table.has_column(component_id)
    }

    pub fn len(&self) -> usize {
        self.data_table.get_row_count()
    }

    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) unsafe fn get_component_unsafe<T: 'static>(&self, row: TableRowIndex) -> Option<&T> {
        unsafe {
            self.data_table
                .get_column(ComponentId::of::<T>())?
                .get_unsafe(row)
        }
    }

    pub fn was_entity_added(
        &self,
        component_id: ComponentId,
        row: TableRowIndex,
        current_tick: u32,
    ) -> bool {
        self.data_table.was_added(row, component_id, current_tick)
    }

    pub(crate) fn has_component_changed_since(
        &self,
        component_id: ComponentId,
        row: TableRowIndex,
        since: Tick,
        current_tick: Tick,
    ) -> bool {
        self.data_table
            .has_changed_since(row, component_id, since, current_tick)
    }

    pub fn was_entity_changed(
        &self,
        component_id: ComponentId,
        row: TableRowIndex,
        current_tick: u32,
    ) -> bool {
        self.data_table.was_changed(row, component_id, current_tick)
    }

    pub(crate) unsafe fn get_component_unsafe_mut<T: 'static>(
        &mut self,
        row: TableRowIndex,
    ) -> Option<MutableCellAccessor<'_, T>> {
        let column = self.data_table.get_column_mut(ComponentId::of::<T>())?;
        unsafe { column.get_unsafe_mut(row) }
    }

    pub fn entities(&self) -> &[Entity] {
        self.data_table.entities()
    }

    /// An empty table shaped like this archetype's, ready to be extended into a new archetype.
    pub(crate) fn clone_empty_table(&self) -> Table {
        self.data_table.clone_empty()
    }

    /// Moves the row at `row` into `dst`, returning the entity that occupied it.
    ///
    /// See [`Table::move_row_to`] for how missing columns are handled.
    pub(crate) fn move_row_to(&mut self, row: TableRowIndex, dst: &mut Archetype) -> Entity {
        self.data_table.move_row_to(row, &mut dst.data_table)
    }

    /// Swap-removes the row at `row`, dropping every component in it.
    pub(crate) fn drop_row(&mut self, row: TableRowIndex) {
        self.data_table.drop_row(row);
    }

    pub fn component_ids(&self) -> &[ComponentId] {
        &self.component_ids
    }
}

#[derive(Clone, Copy, Deref, From)]
pub struct ArchetypeId(usize);

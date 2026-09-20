use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};

use any_vec::{AnyVec, any_value::AnyValueWrapper};

use crate::{
    component::{Component, ComponentId, Tick},
    entity::Entity,
};

pub struct Column {
    data: AnyVec,
    added_ticks: Vec<Tick>,
    changed_ticks: Vec<Tick>,
}

impl Column {
    pub fn new<T: Component>() -> Self {
        Self {
            data: AnyVec::new::<T>(),
            added_ticks: Vec::new(),
            changed_ticks: Vec::new(),
        }
    }

    /// An empty column holding the same component type as `self`.
    pub(crate) fn clone_empty(&self) -> Column {
        Column {
            data: self.data.clone_empty(),
            added_ticks: Vec::new(),
            changed_ticks: Vec::new(),
        }
    }

    pub fn push<T: Component>(&mut self, value: T, tick: u32) {
        self.data.push(AnyValueWrapper::new(value));
        self.added_ticks.push(Tick::new(tick));
        self.changed_ticks.push(Tick::new(0));
    }

    pub fn insert<T: Component>(&mut self, value: T, tick: u32, row: TableRowIndex) {
        if let Some(mut element) = self.data.get_mut(*row)
            && let Some(slot) = element.downcast_mut::<T>()
        {
            *slot = value;
        }

        self.added_ticks[*row].set(tick);
        self.changed_ticks[*row].set(tick);
    }

    /// Moves the value at `row` out of `self` and appends it to `dst`, ticks included.
    pub(crate) fn move_row_to(&mut self, row: TableRowIndex, dst: &mut Column) {
        dst.data.push(self.data.swap_remove(*row));
        dst.added_ticks.push(self.added_ticks.swap_remove(*row));
        dst.changed_ticks.push(self.changed_ticks.swap_remove(*row));
    }

    /// Swap-removes the value at `row` and drops it.
    pub(crate) fn drop_row(&mut self, row: TableRowIndex) {
        self.data.swap_remove(*row);
        self.added_ticks.swap_remove(*row);
        self.changed_ticks.swap_remove(*row);
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub(crate) unsafe fn get_unsafe<T: 'static>(&self, row: TableRowIndex) -> Option<&T> {
        unsafe { self.data.get_unchecked(*row).downcast_ref() }
    }

    pub(crate) unsafe fn get_unsafe_mut<T: 'static>(
        &mut self,
        row: TableRowIndex,
    ) -> Option<MutableCellAccessor<'_, T>> {
        unsafe {
            self.data
                .get_unchecked_mut(*row)
                .downcast_mut()
                .map(|data| MutableCellAccessor {
                    data,
                    changed_tick: &mut self.changed_ticks[*row],
                })
        }
    }

    pub fn set_changed(&mut self, row: TableRowIndex, current_tick: u32) {
        self.changed_ticks[*row].set(current_tick);
    }
}

pub struct Table {
    columns: HashMap<ComponentId, Column>,
    entities: Vec<Entity>,
}

impl Table {
    pub fn new() -> Self {
        Self {
            columns: HashMap::new(),
            entities: Vec::new(),
        }
    }

    /// An empty table with the same columns as `self`.
    pub(crate) fn clone_empty(&self) -> Table {
        Table {
            columns: self
                .columns
                .iter()
                .map(|(id, column)| (*id, column.clone_empty()))
                .collect(),
            entities: Vec::new(),
        }
    }

    pub fn add_column<T: Component>(&mut self) {
        self.columns
            .insert(ComponentId::of::<T>(), Column::new::<T>());
    }

    pub(crate) fn remove_column(&mut self, component_id: ComponentId) {
        self.columns.remove(&component_id);
    }

    pub(crate) fn merge(&mut self, other: Table) {
        for (id, column) in other.columns {
            self.columns.entry(id).or_insert(column);
        }
    }

    pub fn add_entity(&mut self, entity: Entity) {
        self.entities.push(entity);
    }

    /// Moves the row at `row` out of `self` and appends it to `dst`, returning its entity.
    ///
    /// A component is dropped rather than moved when `dst` has no column for it — that is how
    /// component removal happens.
    pub(crate) fn move_row_to(&mut self, row: TableRowIndex, dst: &mut Table) -> Entity {
        for (id, column) in self.columns.iter_mut() {
            match dst.columns.get_mut(id) {
                Some(dst_column) => column.move_row_to(row, dst_column),
                None => column.drop_row(row),
            }
        }

        let entity = self.entities.swap_remove(*row);
        dst.entities.push(entity);
        entity
    }

    /// Swap-removes the row at `row`, dropping every component in it.
    pub(crate) fn drop_row(&mut self, row: TableRowIndex) {
        for column in self.columns.values_mut() {
            column.drop_row(row);
        }

        self.entities.swap_remove(*row);
    }

    pub fn get_row_count(&self) -> usize {
        self.entities.len()
    }

    pub fn has_column(&self, type_id: ComponentId) -> bool {
        self.columns.contains_key(&type_id)
    }

    pub fn get_column(&self, type_id: ComponentId) -> Option<&Column> {
        self.columns.get(&type_id)
    }

    pub fn get_column_mut(&mut self, type_id: ComponentId) -> Option<&mut Column> {
        self.columns.get_mut(&type_id)
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn was_added(
        &self,
        row: TableRowIndex,
        component_id: ComponentId,
        current_tick: u32,
    ) -> bool {
        if let Some(column) = self.columns.get(&component_id) {
            *column.added_ticks[*row] == current_tick
        } else {
            false
        }
    }

    pub(crate) fn has_changed_since(
        &self,
        row: TableRowIndex,
        component_id: ComponentId,
        since: Tick,
        current_tick: Tick,
    ) -> bool {
        let Some(column) = self.columns.get(&component_id) else {
            return false;
        };
        let age = current_tick.wrapping_sub(*since);
        [column.added_ticks[*row], column.changed_ticks[*row]]
            .iter()
            .any(|tick| current_tick.wrapping_sub(**tick) <= age)
    }

    pub fn was_changed(
        &self,
        row: TableRowIndex,
        component_id: ComponentId,
        current_tick: u32,
    ) -> bool {
        if let Some(column) = self.columns.get(&component_id) {
            *column.changed_ticks[*row] == current_tick || *column.added_ticks[*row] == current_tick
        } else {
            false
        }
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct TableRowIndex(usize);

impl TableRowIndex {
    pub const fn new(index: usize) -> TableRowIndex {
        TableRowIndex(index)
    }
}

impl Deref for TableRowIndex {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TableRowIndex {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub(crate) struct MutableCellAccessor<'w, T> {
    pub(crate) data: &'w mut T,
    pub(crate) changed_tick: &'w mut Tick,
}

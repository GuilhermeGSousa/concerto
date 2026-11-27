use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use any_vec::{AnyVec, any_value::AnyValueWrapper};

use crate::{
    component::{CheckChangeTicks, Component, ComponentId, SystemTicks, Tick},
    entity::Entity,
    utilities::TypeIdMap,
};

/// The values of one component type for every row of a [`Table`], with the tick each was added and
/// last changed.
pub struct Column {
    data: AnyVec,
    added_ticks: Vec<Tick>,
    changed_ticks: Vec<Tick>,
}

impl Column {
    /// Creates an empty column for `T`.
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

    /// Appends `value`, stamping it added and changed at `tick`.
    pub fn push<T: Component>(&mut self, value: T, tick: u32) {
        self.data.push(AnyValueWrapper::new(value));
        self.added_ticks.push(Tick::new(tick));
        self.changed_ticks.push(Tick::new(tick));
    }

    /// Overwrites the value at `row` and stamps it changed at `tick`.
    pub fn insert<T: Component>(&mut self, value: T, tick: u32, row: TableRowIndex) {
        if let Some(mut element) = self.data.get_mut(*row)
            && let Some(slot) = element.downcast_mut::<T>()
        {
            *slot = value;
        }

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

    /// Returns the number of values.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns whether the column holds no values.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub(crate) unsafe fn get_unsafe<T: 'static>(&self, row: TableRowIndex) -> Option<&T> {
        // SAFETY: the caller guarantees `row` is less than the column's length.
        unsafe { self.data.get_unchecked(*row).downcast_ref() }
    }

    pub(crate) fn values<T: 'static>(&self) -> Option<&[T]> {
        Some(self.data.downcast_ref::<T>()?.as_slice())
    }

    pub(crate) fn ticks(&self) -> ColumnTicks<'_> {
        ColumnTicks {
            added: self.added_ticks.as_ptr(),
            changed: self.changed_ticks.as_ptr(),
            len: self.added_ticks.len().min(self.changed_ticks.len()),
            _column: PhantomData,
        }
    }

    pub(crate) fn cells_mut<T: 'static>(&mut self) -> Option<ColumnCells<'_, T>> {
        let values = self.data.downcast_mut::<T>()?.as_mut_slice();
        Some(ColumnCells {
            len: values
                .len()
                .min(self.added_ticks.len())
                .min(self.changed_ticks.len()),
            values: values.as_mut_ptr(),
            added: self.added_ticks.as_ptr(),
            changed: self.changed_ticks.as_mut_ptr(),
            _column: PhantomData,
        })
    }

    pub(crate) unsafe fn get_unsafe_mut<T: 'static>(
        &mut self,
        row: TableRowIndex,
    ) -> Option<MutableCellAccessor<'_, T>> {
        // SAFETY: the caller guarantees `row` is less than the column's length.
        unsafe {
            self.data
                .get_unchecked_mut(*row)
                .downcast_mut()
                .map(|data| MutableCellAccessor {
                    data,
                    added_tick: self.added_ticks[*row],
                    changed_tick: &mut self.changed_ticks[*row],
                })
        }
    }

    /// Stamps the value at `row` changed at `current_tick`.
    pub fn set_changed(&mut self, row: TableRowIndex, current_tick: u32) {
        self.changed_ticks[*row].set(current_tick);
    }

    pub(crate) fn check_change_ticks(&mut self, check: CheckChangeTicks) {
        for tick in self.added_ticks.iter_mut().chain(&mut self.changed_ticks) {
            tick.check_tick(check);
        }
    }
}

/// The component columns of an archetype and the entity in each row.
pub struct Table {
    columns: TypeIdMap<Column>,
    entities: Vec<Entity>,
}

impl Table {
    /// Creates a table with no columns and no rows.
    pub fn new() -> Self {
        Self {
            columns: TypeIdMap::default(),
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

    /// Adds an empty column for `T`, replacing any column for `T` already there.
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

    /// Appends `entity` as a new row; the components for that row are pushed to the columns
    /// separately.
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

    /// Returns the number of rows.
    pub fn get_row_count(&self) -> usize {
        self.entities.len()
    }

    /// Returns whether the table has a column for the component `type_id`.
    pub fn has_column(&self, type_id: ComponentId) -> bool {
        self.columns.contains_key(&type_id)
    }

    /// Returns the column for the component `type_id`, or `None` when the table has none.
    pub fn get_column(&self, type_id: ComponentId) -> Option<&Column> {
        self.columns.get(&type_id)
    }

    /// Returns the column for the component `type_id` mutably, or `None` when the table has none.
    pub fn get_column_mut(&mut self, type_id: ComponentId) -> Option<&mut Column> {
        self.columns.get_mut(&type_id)
    }

    /// Returns the entity in each row, in row order.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Returns whether the component at `row` was added since the system with `ticks` last ran;
    /// `false` when the table has no such column.
    pub fn was_added(
        &self,
        row: TableRowIndex,
        component_id: ComponentId,
        ticks: SystemTicks,
    ) -> bool {
        self.columns
            .get(&component_id)
            .is_some_and(|column| ticks.is_changed(column.added_ticks[*row]))
    }

    pub(crate) fn changed_since(
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

    /// Returns whether the component at `row` was added or changed since the system with `ticks`
    /// last ran; `false` when the table has no such column.
    pub fn was_changed(
        &self,
        row: TableRowIndex,
        component_id: ComponentId,
        ticks: SystemTicks,
    ) -> bool {
        self.columns.get(&component_id).is_some_and(|column| {
            ticks.is_changed(column.changed_ticks[*row])
                || ticks.is_changed(column.added_ticks[*row])
        })
    }

    pub(crate) fn check_change_ticks(&mut self, check: CheckChangeTicks) {
        for column in self.columns.values_mut() {
            column.check_change_ticks(check);
        }
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// The index of a row in a [`Table`].
#[derive(Clone, Copy, PartialEq)]
pub struct TableRowIndex(usize);

impl TableRowIndex {
    /// Creates a row index.
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

/// The stamps of one row of a column.
#[derive(Clone, Copy)]
pub struct RowTicks {
    pub(crate) added: Tick,
    pub(crate) changed: Tick,
}

/// The stamps of one column, read by row while a [`ColumnCells`] may write other rows.
pub struct ColumnTicks<'w> {
    added: *const Tick,
    changed: *const Tick,
    len: usize,
    _column: PhantomData<&'w [Tick]>,
}

impl ColumnTicks<'_> {
    /// # Safety
    /// No exclusive borrow of the stamps at `row` may be alive.
    pub(crate) unsafe fn get(&self, row: usize) -> RowTicks {
        assert!(row < self.len, "row {row} is outside its column");
        // SAFETY: `row` is inside both buffers, which live for `'w`, and nothing borrows it exclusively.
        unsafe {
            RowTicks {
                added: *self.added.add(row),
                changed: *self.changed.add(row),
            }
        }
    }
}

/// The values and stamps of one column, borrowed a row at a time.
pub struct ColumnCells<'w, T> {
    values: *mut T,
    added: *const Tick,
    changed: *mut Tick,
    len: usize,
    _column: PhantomData<&'w mut [T]>,
}

impl<'w, T> ColumnCells<'w, T> {
    /// # Safety
    /// No other borrow of the value or the changed stamp at `row` may be alive while the returned one is.
    pub(crate) unsafe fn get(&self, row: usize) -> MutableCellAccessor<'w, T> {
        assert!(row < self.len, "row {row} is outside its column");
        // SAFETY: `row` is inside the three buffers, which live for `'w`, and nothing else borrows it.
        unsafe {
            MutableCellAccessor {
                data: &mut *self.values.add(row),
                added_tick: *self.added.add(row),
                changed_tick: &mut *self.changed.add(row),
            }
        }
    }
}

pub(crate) struct MutableCellAccessor<'w, T> {
    pub(crate) data: &'w mut T,
    pub(crate) added_tick: Tick,
    pub(crate) changed_tick: &'w mut Tick,
}

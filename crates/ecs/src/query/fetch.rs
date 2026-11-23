use std::any::TypeId;

use crate::{
    component::{Component, SystemTicks},
    query::change_detection::{Mut, Ref},
    table::{ColumnCells, ColumnTicks},
    world::UnsafeWorldCell,
};

/// What `&mut T` resolves from one archetype: the cells of `T`'s column and the ticks of the run.
pub struct WriteFetch<'w, T> {
    cells: ColumnCells<'w, T>,
    ticks: SystemTicks,
}

impl<'w, T: Component> WriteFetch<'w, T> {
    /// # Safety
    /// The caller must hold write access to `T` for as long as `'w`.
    pub(crate) unsafe fn new(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Option<Self> {
        // SAFETY: the caller holds write access to `T`, so nothing else borrows its column.
        let column = unsafe { world.column_mut(archetype, TypeId::of::<T>()) }?;
        Some(Self {
            cells: column.cells_mut::<T>()?,
            ticks,
        })
    }

    /// # Safety
    /// No other borrow of the value or the changed stamp at `row` may be alive while the returned one is.
    pub(crate) unsafe fn get(&self, row: usize) -> Mut<'w, T> {
        // SAFETY: the caller rules out another borrow of this row.
        let cell = unsafe { self.cells.get(row) };
        Mut::new(cell.data, cell.added_tick, cell.changed_tick, self.ticks)
    }
}

/// What `Ref<T>` resolves from one archetype: the values and stamps of `T`'s column and the ticks of the run.
pub struct RefFetch<'w, T> {
    values: &'w [T],
    stamps: ColumnTicks<'w>,
    ticks: SystemTicks,
}

impl<'w, T: Component> RefFetch<'w, T> {
    /// # Safety
    /// The caller must hold read access to `T` for as long as `'w`.
    pub(crate) unsafe fn new(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Option<Self> {
        // SAFETY: the caller holds read access to `T`, so nothing writes its column.
        let column = unsafe { world.column(archetype, TypeId::of::<T>()) }?;
        Some(Self {
            values: column.values::<T>()?,
            stamps: column.ticks(),
            ticks,
        })
    }

    pub(crate) fn get(&self, row: usize) -> Ref<'w, T> {
        // SAFETY: read access to `T` rules out an exclusive borrow of its stamps.
        let stamps = unsafe { self.stamps.get(row) };
        Ref::new(&self.values[row], stamps.added, stamps.changed, self.ticks)
    }
}

/// What `Changed<T>` and `Added<T>` resolve from one archetype: the stamps of `T`'s column, if it has one, and the ticks of the run.
pub struct StampFetch<'w> {
    stamps: Option<ColumnTicks<'w>>,
    ticks: SystemTicks,
}

impl<'w> StampFetch<'w> {
    /// # Safety
    /// The caller must hold read access to `T` for as long as `'w`.
    pub(crate) unsafe fn new<T: Component>(
        world: UnsafeWorldCell<'w>,
        archetype: usize,
        ticks: SystemTicks,
    ) -> Self {
        // SAFETY: the caller holds read access to `T`, so nothing moves or frees its column.
        let column = unsafe { world.column(archetype, TypeId::of::<T>()) };
        Self {
            stamps: column.map(|column| column.ticks()),
            ticks,
        }
    }

    /// # Safety
    /// No exclusive borrow of the stamps at `row` may be alive.
    pub(crate) unsafe fn is_added(&self, row: usize) -> bool {
        self.stamps.as_ref().is_some_and(|stamps| {
            // SAFETY: the caller rules out an exclusive borrow of this row's stamps.
            let stamps = unsafe { stamps.get(row) };
            self.ticks.is_changed(stamps.added)
        })
    }

    /// # Safety
    /// No exclusive borrow of the stamps at `row` may be alive.
    pub(crate) unsafe fn is_changed(&self, row: usize) -> bool {
        self.stamps.as_ref().is_some_and(|stamps| {
            // SAFETY: the caller rules out an exclusive borrow of this row's stamps.
            let stamps = unsafe { stamps.get(row) };
            self.ticks.is_changed(stamps.changed) || self.ticks.is_changed(stamps.added)
        })
    }
}

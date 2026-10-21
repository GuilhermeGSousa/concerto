use std::ops::{Deref, DerefMut};

use crate::component::{Component, SystemTicks, Tick};

/// A mutable reference to a component that automatically marks the component as changed.
///
/// Obtained from a `Query<&mut T>`.  Deref gives `&T`; DerefMut gives `&mut T` and
/// records a change tick so that [`Changed<T>`](super::query_filter::Changed) filters work.
pub struct Mut<'w, T: Component> {
    data: &'w mut T,
    changed_tick: &'w mut Tick,
    ticks: SystemTicks,
    was_changed: bool,
}

impl<'w, T> Mut<'w, T>
where
    T: Component,
{
    /// Wraps `data` for a run with `ticks`; a write stamps `changed_tick` with `ticks.this_run`.
    pub fn new(data: &'w mut T, changed_tick: &'w mut Tick, ticks: SystemTicks) -> Self {
        Self {
            data,
            changed_tick,
            ticks,
            was_changed: false,
        }
    }
}

impl<'w, T> Deref for Mut<'w, T>
where
    T: Component,
{
    type Target = &'w mut T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<'w, T> DerefMut for Mut<'w, T>
where
    T: Component,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        if !self.was_changed {
            self.was_changed = true;
            *self.changed_tick = self.ticks.this_run;
        }
        &mut self.data
    }
}

/// Trait for types that can report whether their underlying data changed this tick.
pub trait DetectChanges {
    /// Returns `true` if the value was modified during the current tick.
    fn has_changed(&self) -> bool;
}

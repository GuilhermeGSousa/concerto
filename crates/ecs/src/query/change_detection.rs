use std::ops::{Deref, DerefMut};

use crate::component::{Component, SystemTicks, Tick};

/// A mutable reference to a component that automatically marks the component as changed.
///
/// Obtained from a `Query<&mut T>`.  Deref gives `&T`; DerefMut gives `&mut T` and
/// records a change tick so that [`Changed<T>`](super::query_filter::Changed) filters work.
pub struct Mut<'w, T: Component> {
    data: &'w mut T,
    added_tick: Tick,
    changed_tick: &'w mut Tick,
    ticks: SystemTicks,
    was_changed: bool,
}

impl<'w, T> Mut<'w, T>
where
    T: Component,
{
    /// Wraps `data` for a run with `ticks`; a write stamps `changed_tick` with `ticks.this_run`.
    pub fn new(
        data: &'w mut T,
        added_tick: Tick,
        changed_tick: &'w mut Tick,
        ticks: SystemTicks,
    ) -> Self {
        Self {
            data,
            added_tick,
            changed_tick,
            ticks,
            was_changed: false,
        }
    }
}

impl<'w, T> Mut<'w, T>
where
    T: Component + PartialEq,
{
    /// Writes `value` and stamps the change only if it differs; returns whether it wrote.
    pub fn set_if_neq(&mut self, value: T) -> bool {
        if *self.data == value {
            return false;
        }
        *self.deref_mut().deref_mut() = value;
        true
    }
}

impl<T> DetectChanges for Mut<'_, T>
where
    T: Component,
{
    fn is_changed(&self) -> bool {
        self.ticks.is_changed(*self.changed_tick)
    }

    fn is_added(&self) -> bool {
        self.ticks.is_changed(self.added_tick)
    }
}

/// A read-only component reference from a `Query<Ref<T>>` that reports whether it was added or changed.
pub struct Ref<'w, T: Component> {
    data: &'w T,
    added_tick: Tick,
    changed_tick: Tick,
    ticks: SystemTicks,
}

impl<'w, T> Ref<'w, T>
where
    T: Component,
{
    /// Wraps `data` for a run with `ticks`, carrying the component's own stamps.
    pub fn new(data: &'w T, added_tick: Tick, changed_tick: Tick, ticks: SystemTicks) -> Self {
        Self {
            data,
            added_tick,
            changed_tick,
            ticks,
        }
    }
}

impl<T> Deref for Ref<'_, T>
where
    T: Component,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<T> DetectChanges for Ref<'_, T>
where
    T: Component,
{
    fn is_changed(&self) -> bool {
        self.ticks.is_changed(self.changed_tick)
    }

    fn is_added(&self) -> bool {
        self.ticks.is_changed(self.added_tick)
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

/// Trait for types that can report whether their underlying data changed since the reading system last ran.
pub trait DetectChanges {
    /// Returns `true` if the value was added or modified since the reading system last ran.
    fn is_changed(&self) -> bool;

    /// Returns `true` if the value was added since the reading system last ran.
    fn is_added(&self) -> bool;
}

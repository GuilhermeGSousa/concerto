use std::ops::{Deref, DerefMut};

use crate::component::Tick;

/// A mutable reference to a component that automatically marks the component as changed.
///
/// Obtained from a `Query<&mut T>`.  Deref gives `&T`; DerefMut gives `&mut T` and
/// records a change tick so that [`Changed<T>`](super::query_filter::Changed) filters work.
///
/// `T` may be unsized so that a trait query can hand out `Mut<dyn Trait>` over a concrete
/// component's storage.
pub struct Mut<'w, T: ?Sized> {
    data: &'w mut T,
    changed_tick: &'w mut Tick,
    current_tick: Tick,
    was_changed: bool,
}

impl<'w, T> Mut<'w, T>
where
    T: ?Sized,
{
    pub fn new(data: &'w mut T, changed_tick: &'w mut Tick, current_tick: Tick) -> Self {
        Self {
            data,
            changed_tick,
            current_tick,
            was_changed: false,
        }
    }
}

impl<'w, T> Deref for Mut<'w, T>
where
    T: ?Sized,
{
    type Target = &'w mut T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<'w, T> DerefMut for Mut<'w, T>
where
    T: ?Sized,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        if !self.was_changed {
            self.was_changed = true;
            *self.changed_tick = self.current_tick;
        }
        &mut self.data
    }
}

/// Trait for types that can report whether their underlying data changed this tick.
pub trait DetectChanges {
    /// Returns `true` if the value was modified during the current tick.
    fn has_changed(&self) -> bool;
}

use crate::{
    archetype::Archetype,
    component::{Component, ComponentId},
    table::{Table, TableRowIndex},
};
use std::any::TypeId;

use typle::typle;

/// Where a [`ComponentBundle`] writes its components, one at a time.
#[doc(hidden)]
pub trait ComponentSink {
    fn write<T: Component>(&mut self, value: T, current_tick: u32);
}

/// Always appends — for a row known to be brand new.
pub(crate) struct PushRow<'a>(pub &'a mut Archetype);

impl ComponentSink for PushRow<'_> {
    fn write<T: Component>(&mut self, value: T, current_tick: u32) {
        self.0.push_component(value, current_tick);
    }
}

/// Always overwrites — for a row known to already hold a value in every column being written.
pub(crate) struct ReplaceRow<'a>(pub &'a mut Archetype, pub TableRowIndex);

impl ComponentSink for ReplaceRow<'_> {
    fn write<T: Component>(&mut self, value: T, current_tick: u32) {
        self.0.insert_component(value, current_tick, self.1);
    }
}

/// Appends where a migration didn't carry a value over for this column, overwrites where it
/// did — for a row a migration just produced, which may be a mix of both.
pub(crate) struct MergeRow<'a>(pub &'a mut Archetype, pub TableRowIndex);

impl ComponentSink for MergeRow<'_> {
    fn write<T: Component>(&mut self, value: T, current_tick: u32) {
        if self.0.has_value_at::<T>(self.1) {
            self.0.insert_component(value, current_tick, self.1);
        } else {
            self.0.push_component(value, current_tick);
        }
    }
}

pub trait ComponentBundle: Send + Sync + Sized {
    fn get_component_ids() -> Vec<ComponentId>;

    fn generate_empty_table() -> Table;

    /// Writes every component in the bundle onto `sink`, one at a time.
    #[doc(hidden)]
    fn write_into<S: ComponentSink>(self, sink: &mut S, current_tick: u32);
}

/// Anything that can be spawned: a bundle, or a builder that produces one.
pub trait IntoBundle: Send + Sync + Sized {
    type Bundle: ComponentBundle;

    fn into_bundle(self) -> Self::Bundle;
}

impl<T: ComponentBundle> IntoBundle for T {
    type Bundle = T;

    fn into_bundle(self) -> T {
        self
    }
}

impl<T> ComponentBundle for T
where
    T: Component,
{
    fn get_component_ids() -> Vec<ComponentId> {
        vec![TypeId::of::<T>()]
    }

    fn generate_empty_table() -> Table {
        let mut table: Table = Table::new();
        table.add_column::<T>();
        table
    }

    fn write_into<S: ComponentSink>(self, sink: &mut S, current_tick: u32) {
        sink.write(self, current_tick);
    }
}

#[allow(unused_mut)]
#[allow(unused_variables)]
#[typle(Tuple for 0..=12)]
impl<T> ComponentBundle for T
where
    T: Tuple,
    T<_>: IntoBundle,
{
    fn get_component_ids() -> Vec<ComponentId> {
        let mut type_ids = Vec::new();
        for typle_index!(i) in 0..T::LEN {
            type_ids.extend(<<T<{ i }> as IntoBundle>::Bundle>::get_component_ids());
        }

        type_ids.sort();
        type_ids.dedup();
        type_ids
    }

    fn write_into<S: ComponentSink>(self, sink: &mut S, current_tick: u32) {
        for typle_index!(i) in 0..T::LEN {
            self[[i]].into_bundle().write_into(sink, current_tick);
        }
    }

    #[allow(clippy::let_and_return)]
    fn generate_empty_table() -> Table {
        let mut table = Table::new();
        for typle_index!(i) in 0..T::LEN {
            table.merge(<<T<{ i }> as IntoBundle>::Bundle>::generate_empty_table());
        }
        table
    }
}

use std::borrow::Borrow;

use crate::component::ComponentId;

/// A set of component ids, sorted and without repeats: an archetype's identity, or the components one move adds or removes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArchetypeType(Box<[ComponentId]>);

impl ArchetypeType {
    /// The set holding `component_ids`, whatever their order and however often each repeats.
    pub fn of(component_ids: &[ComponentId]) -> Self {
        let mut ids = Vec::new();
        Self::sort_into(component_ids, &mut ids);
        Self(ids.into_boxed_slice())
    }

    /// Writes `component_ids` into `sorted` the way a set stores them, to look one up without allocating.
    pub fn sort_into(component_ids: &[ComponentId], sorted: &mut Vec<ComponentId>) {
        sorted.clear();
        sorted.extend_from_slice(component_ids);
        sorted.sort_unstable();
        sorted.dedup();
    }
}

impl Borrow<[ComponentId]> for ArchetypeType {
    fn borrow(&self) -> &[ComponentId] {
        &self.0
    }
}

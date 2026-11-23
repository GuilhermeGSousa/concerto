use crate::component::ComponentId;

/// The identity of an archetype: its component ids, sorted and without repeats.
pub fn archetype_key(component_ids: &[ComponentId]) -> Box<[ComponentId]> {
    let mut key = component_ids.to_vec();
    key.sort();
    key.dedup();
    key.into_boxed_slice()
}

//! Bookkeeping shared by the panels that keep one tree item per thing they list.
use concerto_ecs::{
    Entity, Query,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    query::change_detection::Mut,
};
use std::collections::{HashMap, HashSet};

pub(crate) fn despawn_outermost(
    items: &HashSet<Entity>,
    parents: &Query<&ChildOf>,
    cmd: &mut CommandQueue,
) {
    for &item in items {
        let parent = parents.get_entity(item).map(ChildOf::parent);
        if !parent.is_some_and(|parent| items.contains(&parent)) {
            cmd.despawn(item);
        }
    }
}

pub(crate) fn follow_order(children: &mut Mut<Children>, order: &[Entity]) {
    let rank: HashMap<Entity, usize> = order
        .iter()
        .enumerate()
        .map(|(rank, item)| (*item, rank))
        .collect();
    let present: Vec<Entity> = children
        .iter()
        .copied()
        .filter(|child| rank.contains_key(child))
        .collect();
    let mut sorted = present.clone();
    sorted.sort_by_key(|item| rank[item]);
    if present == sorted {
        return;
    }
    let slots = children
        .iter()
        .enumerate()
        .filter(|(_, child)| rank.contains_key(child))
        .map(|(slot, _)| slot);
    let mut key: HashMap<Entity, usize> = children
        .iter()
        .enumerate()
        .map(|(slot, child)| (*child, slot))
        .collect();
    for (slot, item) in slots.zip(sorted) {
        key.insert(item, slot);
    }
    children.sort_by_key(|child| key[&child]);
}

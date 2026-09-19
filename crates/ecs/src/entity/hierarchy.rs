use std::ops::Deref;

use crate::{component::Component, entity::Entity};

pub struct Children {
    children: Vec<Entity>,
}

impl Children {
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Entity> + ExactSizeIterator {
        self.children.iter()
    }

    pub fn sort_by_key<K: Ord>(&mut self, mut key: impl FnMut(Entity) -> K) {
        self.children.sort_by_key(|entity| key(*entity));
    }

    pub(crate) fn from_children(children: Vec<Entity>) -> Self {
        Self { children }
    }

    pub(crate) fn add_child(&mut self, child: Entity) {
        if !self.children.contains(&child) {
            self.children.push(child);
        }
    }

    pub(crate) fn remove_child(&mut self, child: Entity) {
        self.children.retain(|candidate| *candidate != child);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl Component for Children {
    fn name() -> &'static str
    where
        Self: Sized,
    {
        std::any::type_name::<Self>()
    }

    fn on_despawn() -> Option<crate::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let Some(children) = world.get_component_for_entity::<Children>(context.entity) else {
                return;
            };

            let children: Vec<_> = children.into_iter().map(|e| *e).collect();
            for child in children {
                world.despawn(child);
            }
        })
    }
}

impl IntoIterator for Children {
    type Item = Entity;

    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.children.into_iter()
    }
}

impl<'a> IntoIterator for &'a Children {
    type Item = <&'a Vec<Entity> as IntoIterator>::Item;
    type IntoIter = <&'a Vec<Entity> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.children.iter()
    }
}

#[allow(dead_code)]
pub struct ChildOf {
    parent: Entity,
}

impl Component for ChildOf {
    fn on_add() -> Option<crate::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let Some(parent) = world
                .get_component_for_entity::<ChildOf>(context.entity)
                .map(ChildOf::parent)
            else {
                return;
            };

            match world.get_component_for_entity_mut::<Children>(parent) {
                Some(children) => children.add_child(context.entity),
                None => world.insert(Children::from_children(vec![context.entity]), parent, true),
            }
        })
    }

    fn on_despawn() -> Option<crate::component::ComponentLifecycleCallback> {
        Self::on_remove()
    }

    fn on_remove() -> Option<crate::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let Some(parent) = world
                .get_component_for_entity::<ChildOf>(context.entity)
                .map(ChildOf::parent)
            else {
                return;
            };

            let remove_empty_children = world
                .get_component_for_entity_mut::<Children>(parent)
                .is_some_and(|children| {
                    children.remove_child(context.entity);
                    children.is_empty()
                });
            if remove_empty_children {
                world.remove_component::<Children>(parent, true);
            }
        })
    }
}

impl ChildOf {
    pub fn new(parent: Entity) -> Self {
        Self { parent }
    }

    pub fn parent(&self) -> Entity {
        self.parent
    }
}

impl Deref for ChildOf {
    type Target = Entity;

    fn deref(&self) -> &Self::Target {
        &self.parent
    }
}

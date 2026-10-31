use std::{any::TypeId, sync::Arc};

use concerto_ecs::{
    Component, Entity, World,
    reflect::{AppTypeRegistry, ReflectComponent},
    system::input::ComponentMetadata,
};
use concerto_reflect::{PropertyPath, Reflect};

use crate::hosted::HostedApp;

/// A reflected value the inspector can show and edit.
pub trait PropertyTarget: Send + Sync + 'static {
    /// The card's title, and the name MCP addresses it by.
    fn name(&self) -> String;
    /// The world this target's value lives in.
    fn world(&self) -> TargetWorld;
    /// Calls `read` with the value; false when it's gone.
    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool;
    /// Calls `edit` with the value; false when it's gone.
    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool;
    /// The registry this value's types are in.
    fn types(&self, world: &World) -> Option<AppTypeRegistry>;
    /// The document that holds this value, if a document does.
    fn document(&self) -> Option<Entity> {
        None
    }
    /// This target as a component's address, if it is one.
    fn component(&self) -> Option<ComponentTarget> {
        None
    }
}

/// Where a target's value lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetWorld {
    /// The world of the scene being edited, or of the running game while playing.
    EditTarget,
    /// The editor's own world.
    Editor,
}

/// A component on an entity in the edit target's world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentTarget {
    pub entity: Entity,
    pub component: TypeId,
}

impl ComponentTarget {
    fn reflected(&self, world: &World) -> Option<ReflectComponent> {
        world
            .get_resource::<AppTypeRegistry>()?
            .get_type_data::<ReflectComponent>(self.component)
    }
}

impl PropertyTarget for ComponentTarget {
    fn name(&self) -> String {
        format!("{:?} on {:?}", self.component, self.entity)
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::EditTarget
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let value = self
            .reflected(world)
            .and_then(|component| (component.read)(&ComponentMetadata::new(world), self.entity));
        let Some(value) = value else {
            return false;
        };
        read(value);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        self.reflected(world)
            .is_some_and(|component| (component.edit)(world, self.entity, edit))
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        world.get_resource::<AppTypeRegistry>().cloned()
    }

    fn component(&self) -> Option<ComponentTarget> {
        Some(*self)
    }
}

/// A value inside `C`, a component on a document in the editor's world.
pub struct DocumentValue<C> {
    pub name: String,
    pub document: Entity,
    pub read: Box<dyn Fn(&C) -> Option<&dyn Reflect> + Send + Sync>,
    pub edit: Box<dyn Fn(&mut C) -> Option<&mut dyn Reflect> + Send + Sync>,
}

impl<C: Component> PropertyTarget for DocumentValue<C> {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let value = world
            .get_component_for_entity::<C>(self.document)
            .and_then(|held| (self.read)(held));
        let Some(value) = value else {
            return false;
        };
        read(value);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        let value = world
            .get_component_for_entity_mut::<C>(self.document)
            .and_then(|held| (self.edit)(held));
        let Some(value) = value else {
            return false;
        };
        edit(value);
        true
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        world
            .get_component_for_entity::<HostedApp>(self.document)
            .and_then(HostedApp::type_registry)
            .or_else(|| world.get_resource::<AppTypeRegistry>().cloned())
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }
}

/// One property of a target, captured at rebuild, so a queued edit always addresses its original target.
#[derive(Clone)]
pub struct PropertyAddress {
    pub target: Arc<dyn PropertyTarget>,
    pub path: PropertyPath,
    pub type_id: TypeId,
}

impl PropertyAddress {
    /// Whether this address is of `target`.
    pub fn reaches(&self, target: &Arc<dyn PropertyTarget>) -> bool {
        Arc::ptr_eq(&self.target, target)
            || self
                .target
                .component()
                .is_some_and(|component| target.component() == Some(component))
    }
}

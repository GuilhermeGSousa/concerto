use concerto_ecs::{Component, Entity, World, reflect::AppTypeRegistry};
use concerto_reflect::Reflect;

use crate::hosted::HostedApp;
use crate::property::{PropertyTarget, TargetWorld};

/// The type registry a document's values are in, ahead of its hosted `App`'s and the editor's.
#[derive(Component, Clone)]
pub struct DocumentTypes(pub AppTypeRegistry);

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
            .get_component_for_entity::<DocumentTypes>(self.document)
            .map(|types| types.0.clone())
            .or_else(|| {
                world
                    .get_component_for_entity::<HostedApp>(self.document)
                    .and_then(HostedApp::type_registry)
            })
            .or_else(|| world.get_resource::<AppTypeRegistry>().cloned())
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }
}

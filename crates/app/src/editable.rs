use std::{any::TypeId, collections::HashMap};

use concerto_ecs::{system::input::ComponentMetadata, Component, Entity, Resource, World};
use concerto_editable::Editable;

use crate::App;

/// How to read, change, add, and remove one editable component on a world.
#[derive(Clone, Copy)]
pub struct EditableComponent {
    pub name: &'static str,
    pub read: for<'w> fn(&ComponentMetadata<'w>, Entity) -> Option<&'w dyn Editable>,
    pub edit: fn(&mut World, Entity, &mut dyn FnMut(&mut dyn Editable)) -> bool,
    pub insert: Option<fn(&mut World, Entity)>,
    pub remove: fn(&mut World, Entity),
}

/// The components an editor may show and change, registered by the plugins that own them.
#[derive(Resource, Default)]
pub struct EditableComponents {
    components: HashMap<TypeId, EditableComponent>,
}

impl EditableComponents {
    /// Registers a component the editor may also add, at its default value.
    pub fn register<T: Component + Editable + Default>(&mut self) {
        self.insert::<T>(Some(|world, entity| world.insert(T::default(), entity)));
    }

    /// Registers a component the editor edits and removes but never offers to add.
    pub fn register_inspectable<T: Component + Editable>(&mut self) {
        self.insert::<T>(None);
    }

    pub fn get(&self, id: TypeId) -> Option<EditableComponent> {
        self.components.get(&id).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (TypeId, EditableComponent)> + '_ {
        self.components
            .iter()
            .map(|(&id, &component)| (id, component))
    }

    fn insert<T: Component + Editable>(&mut self, insert: Option<fn(&mut World, Entity)>) {
        let path = std::any::type_name::<T>();
        self.components.insert(
            TypeId::of::<T>(),
            EditableComponent {
                name: path.rsplit("::").next().unwrap_or(path),
                read: read::<T>,
                edit: edit::<T>,
                insert,
                remove: |world, entity| world.remove_component::<T>(entity),
            },
        );
    }
}

fn read<'w, T: Component + Editable>(
    components: &ComponentMetadata<'w>,
    entity: Entity,
) -> Option<&'w dyn Editable> {
    components
        .get::<T>(entity)
        .map(|value| value as &dyn Editable)
}

fn edit<T: Component + Editable>(
    world: &mut World,
    entity: Entity,
    visit: &mut dyn FnMut(&mut dyn Editable),
) -> bool {
    let Some(value) = world.get_component_for_entity_mut::<T>(entity) else {
        return false;
    };
    visit(&mut *value);
    true
}

impl App {
    /// Registers a component the editor may show, change, add, and remove.
    pub fn register_editable<T: Component + Editable + Default>(&mut self) -> &mut Self {
        self.editable_components().register::<T>();
        self
    }

    /// Registers a component the editor may show, change, and remove, but not add.
    pub fn register_inspectable<T: Component + Editable>(&mut self) -> &mut Self {
        self.editable_components().register_inspectable::<T>();
        self
    }

    fn editable_components(&mut self) -> &mut EditableComponents {
        let world = self.main_mut().world_mut();
        if world.get_resource::<EditableComponents>().is_none() {
            world.insert_resource(EditableComponents::default());
        }
        world.get_resource_mut::<EditableComponents>().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{plugins::TransformPlugin, App};
    use concerto_ecs::{system::input::ComponentMetadata, Component};
    use concerto_editable::Editable;
    use concerto_foundation::transform::Transform;
    use std::any::TypeId;

    #[derive(Component, Editable, Default)]
    struct Health {
        points: f32,
    }

    #[derive(Component, Editable)]
    struct Lens {
        length: f32,
    }

    #[test]
    fn the_transform_plugin_registers_transform() {
        let mut app = App::new();
        app.register_plugin(crate::main_schedule::MainSchedulePlugin)
            .register_plugin(crate::plugins::TimePlugin)
            .register_plugin(TransformPlugin);

        let table = app.get_resource::<EditableComponents>().unwrap();
        let entry = table.get(TypeId::of::<Transform>()).unwrap();
        assert_eq!(entry.name, "Transform");
        assert!(entry.insert.is_some());
    }

    #[test]
    fn a_recipe_registers_its_own_components() {
        let mut app = App::new();
        app.register_editable::<Health>()
            .register_inspectable::<Lens>();

        let table = app.get_resource::<EditableComponents>().unwrap();
        assert!(table.get(TypeId::of::<Health>()).unwrap().insert.is_some());
        assert!(table.get(TypeId::of::<Lens>()).unwrap().insert.is_none());
    }

    #[test]
    fn an_entry_reads_edits_inserts_and_removes_its_component() {
        let mut app = App::new();
        app.register_editable::<Health>();
        let entry = app
            .get_resource::<EditableComponents>()
            .unwrap()
            .get(TypeId::of::<Health>())
            .unwrap();
        let world = app.main_mut().world_mut();
        let entity = world.spawn(());

        assert!((entry.read)(&ComponentMetadata::new(world), entity).is_none());
        assert!(!(entry.edit)(world, entity, &mut |_| {}));

        (entry.insert.unwrap())(world, entity);
        assert!((entry.read)(&ComponentMetadata::new(world), entity).is_some());

        let mut visited = false;
        assert!((entry.edit)(world, entity, &mut |_| visited = true));
        assert!(visited);

        (entry.remove)(world, entity);
        assert!((entry.read)(&ComponentMetadata::new(world), entity).is_none());
    }
}

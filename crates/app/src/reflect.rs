use std::{any::TypeId, collections::HashMap};

use concerto_ecs::{system::input::ComponentMetadata, Component, Entity, Resource, World};
use concerto_reflect::Reflect;

use crate::App;

/// How to read, change, add, and remove one reflected component on a world.
#[derive(Clone, Copy)]
pub struct ReflectComponent {
    pub name: &'static str,
    pub read: for<'w> fn(&ComponentMetadata<'w>, Entity) -> Option<&'w dyn Reflect>,
    pub edit: fn(&mut World, Entity, &mut dyn FnMut(&mut dyn Reflect)) -> bool,
    pub insert: Option<fn(&mut World, Entity)>,
    pub remove: fn(&mut World, Entity),
}

/// The components an editor may show and change, registered by the plugins that own them.
#[derive(Resource, Default)]
pub struct ReflectComponents {
    components: HashMap<TypeId, ReflectComponent>,
}

impl ReflectComponents {
    /// Registers a component the editor may also add, at its default value.
    pub fn register<T: Component + Reflect + Default>(&mut self) {
        self.insert::<T>(Some(|world, entity| world.insert(T::default(), entity)));
    }

    /// Registers a component the editor edits and removes but never offers to add.
    pub fn register_readonly<T: Component + Reflect>(&mut self) {
        self.insert::<T>(None);
    }

    pub fn get(&self, id: TypeId) -> Option<ReflectComponent> {
        self.components.get(&id).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (TypeId, ReflectComponent)> + '_ {
        self.components
            .iter()
            .map(|(&id, &component)| (id, component))
    }

    fn insert<T: Component + Reflect>(&mut self, insert: Option<fn(&mut World, Entity)>) {
        let path = std::any::type_name::<T>();
        self.components.insert(
            TypeId::of::<T>(),
            ReflectComponent {
                name: path.rsplit("::").next().unwrap_or(path),
                read: read::<T>,
                edit: edit::<T>,
                insert,
                remove: |world, entity| world.remove_component::<T>(entity),
            },
        );
    }
}

fn read<'w, T: Component + Reflect>(
    components: &ComponentMetadata<'w>,
    entity: Entity,
) -> Option<&'w dyn Reflect> {
    components
        .get::<T>(entity)
        .map(|value| value as &dyn Reflect)
}

fn edit<T: Component + Reflect>(
    world: &mut World,
    entity: Entity,
    visit: &mut dyn FnMut(&mut dyn Reflect),
) -> bool {
    let Some(value) = world.get_component_for_entity_mut::<T>(entity) else {
        return false;
    };
    visit(&mut *value);
    true
}

impl App {
    /// Registers a component the editor may show, change, add, and remove.
    pub fn register_reflect<T: Component + Reflect + Default>(&mut self) -> &mut Self {
        self.reflect_components().register::<T>();
        self
    }

    /// Registers a component the editor may show, change, and remove, but not add.
    pub fn register_reflect_readonly<T: Component + Reflect>(&mut self) -> &mut Self {
        self.reflect_components().register_readonly::<T>();
        self
    }

    fn reflect_components(&mut self) -> &mut ReflectComponents {
        let world = self.main_mut().world_mut();
        if world.get_resource::<ReflectComponents>().is_none() {
            world.insert_resource(ReflectComponents::default());
        }
        world.get_resource_mut::<ReflectComponents>().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{plugins::TransformPlugin, App};
    use concerto_ecs::{system::input::ComponentMetadata, Component};
    use concerto_foundation::transform::Transform;
    use concerto_reflect::Reflect;
    use std::any::TypeId;

    #[derive(Component, Reflect, Default)]
    struct Health {
        points: f32,
    }

    #[derive(Component, Reflect)]
    struct Lens {
        length: f32,
    }

    #[test]
    fn the_transform_plugin_registers_transform() {
        let mut app = App::new();
        app.register_plugin(crate::main_schedule::MainSchedulePlugin)
            .register_plugin(crate::plugins::TimePlugin)
            .register_plugin(TransformPlugin);

        let table = app.get_resource::<ReflectComponents>().unwrap();
        let entry = table.get(TypeId::of::<Transform>()).unwrap();
        assert_eq!(entry.name, "Transform");
        assert!(entry.insert.is_some());
    }

    #[test]
    fn a_recipe_registers_its_own_components() {
        let mut app = App::new();
        app.register_reflect::<Health>()
            .register_reflect_readonly::<Lens>();

        let table = app.get_resource::<ReflectComponents>().unwrap();
        assert!(table.get(TypeId::of::<Health>()).unwrap().insert.is_some());
        assert!(table.get(TypeId::of::<Lens>()).unwrap().insert.is_none());
    }

    #[test]
    fn an_entry_reads_edits_inserts_and_removes_its_component() {
        let mut app = App::new();
        app.register_reflect::<Health>();
        let entry = app
            .get_resource::<ReflectComponents>()
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

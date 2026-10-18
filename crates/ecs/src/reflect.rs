use std::any::{Any, TypeId};

use concerto_reflect::{
    FromType, Reflect, ReflectDefault, TypeData, TypeRegistration, TypeRegistry,
};
use derive_more::{Deref, DerefMut};
use serde_json::Value;

use crate::{
    Component, Entity, Resource, World,
    component::scene::{SceneComponent, SceneSpawnContext},
    system::input::ComponentMetadata,
};

/// The world's type registry.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct AppTypeRegistry(pub TypeRegistry);

/// How to read, change, add and remove a reflected component on a world.
#[derive(Clone, Copy)]
pub struct ReflectComponent {
    pub read: for<'w> fn(&ComponentMetadata<'w>, Entity) -> Option<&'w dyn Reflect>,
    pub edit: fn(&mut World, Entity, &mut dyn FnMut(&mut dyn Reflect)) -> bool,
    pub insert_boxed: fn(&mut World, Entity, Box<dyn Reflect>) -> bool,
    pub remove: fn(&mut World, Entity),
}

impl ReflectComponent {
    /// Inserts the type `id` at its `Default`; false when it did not declare `Default`.
    pub fn insert_default(&self, id: TypeId, world: &mut World, entity: Entity) -> bool {
        let Some(default) = world
            .get_resource::<AppTypeRegistry>()
            .and_then(|registry| registry.get_type_data::<ReflectDefault>(id))
            .map(ReflectDefault::default)
        else {
            return false;
        };
        (self.insert_boxed)(world, entity, default)
    }
}

impl TypeData for ReflectComponent {}

impl<T: Component + Reflect> FromType<T> for ReflectComponent {
    fn from_type() -> Self {
        Self {
            read: read::<T>,
            edit: edit::<T>,
            insert_boxed: insert_boxed::<T>,
            remove: |world, entity| world.remove_component::<T>(entity),
        }
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

fn insert_boxed<T: Component + Reflect>(
    world: &mut World,
    entity: Entity,
    value: Box<dyn Reflect>,
) -> bool {
    let value: Box<dyn Any> = value;
    let Ok(value) = value.downcast::<T>() else {
        return false;
    };
    world.insert(*value, entity);
    true
}

/// How a scene file spawns and captures a component; serde-backed until scenes use reflection.
#[derive(Clone, Copy)]
pub struct ReflectSceneComponent {
    pub apply: fn(&str, Entity, &mut SceneSpawnContext<'_>) -> Result<(), serde_json::Error>,
    pub to_json: fn(&World, Entity) -> Option<String>,
    pub read: fn(&World, Entity) -> Option<Value>,
}

impl TypeData for ReflectSceneComponent {}

impl<T: SceneComponent + Reflect> FromType<T> for ReflectSceneComponent {
    fn from_type() -> Self {
        Self {
            apply: apply::<T>,
            to_json: |world, entity| {
                serde_json::to_string(world.get_component_for_entity::<T>(entity)?).ok()
            },
            read: |world, entity| {
                serde_json::to_value(world.get_component_for_entity::<T>(entity)?).ok()
            },
        }
    }
}

fn apply<T: SceneComponent>(
    json: &str,
    entity: Entity,
    ctx: &mut SceneSpawnContext<'_>,
) -> Result<(), serde_json::Error> {
    let value: T = serde_json::from_str(json)?;
    value.apply(entity, ctx);
    Ok(())
}

/// A scene component an entity carries, as `World::component_types` lists it.
pub struct SceneComponentType<'w> {
    registration: &'w TypeRegistration,
    scene: ReflectSceneComponent,
}

impl<'w> SceneComponentType<'w> {
    pub(crate) fn new(registration: &'w TypeRegistration, scene: ReflectSceneComponent) -> Self {
        Self {
            registration,
            scene,
        }
    }

    /// The canonical full type path, e.g. `concerto_foundation::transform::Transform`.
    pub fn name(&self) -> &'static str {
        self.registration.type_path()
    }

    /// The final `::` segment, e.g. `Transform`.
    pub fn short(&self) -> &'static str {
        self.registration.short_name()
    }

    pub fn registration(&self) -> &'w TypeRegistration {
        self.registration
    }

    /// The component's value as the JSON text a scene file stores.
    pub fn to_json(&self, world: &World, entity: Entity) -> Option<String> {
        (self.scene.to_json)(world, entity)
    }

    /// The component's current value off `entity`.
    pub fn read(&self, world: &World, entity: Entity) -> Option<Value> {
        (self.scene.read)(world, entity)
    }
}

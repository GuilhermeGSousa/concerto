use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use concerto_asset_format::AssetId;
use concerto_reflect::{
    EncodeError, FromType, LeafError, LeafHook, Reflect, ReflectDefault, ReflectedValue, TypeData,
    TypeRegistration, TypeRegistry, ValueWarning, apply_value_with, to_value_with,
};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{Component, Entity, Resource, World, system::input::ComponentMetadata};

/// The world's type registry, shared by every clone; take one guard at a time.
#[derive(Resource, Clone, Default)]
pub struct AppTypeRegistry(Arc<RwLock<TypeRegistry>>);

impl AppTypeRegistry {
    /// Locks the registry for reading.
    pub fn read(&self) -> RwLockReadGuard<'_, TypeRegistry> {
        self.0.read()
    }

    /// Locks the registry for writing.
    pub fn write(&self) -> RwLockWriteGuard<'_, TypeRegistry> {
        self.0.write()
    }

    /// A copy of the type data `D` registered for `id`, taken under the read lock.
    pub fn get_type_data<D: TypeData + Copy>(&self, id: TypeId) -> Option<D> {
        self.0.read().get_type_data::<D>(id).copied()
    }
}

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
            .map(|data| data.default())
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

/// How a scene spawns a component from stored data and captures it back.
#[derive(Clone, Copy)]
pub struct ReflectSceneComponent {
    pub spawn: fn(&mut World, Entity, &ReflectedValue, &[Entity]) -> Vec<ValueWarning>,
    pub capture:
        fn(&World, Entity, &mut SceneCapture) -> Option<Result<ReflectedValue, EncodeError>>,
}

impl TypeData for ReflectSceneComponent {}

impl<T: Component + Reflect + Default> FromType<T> for ReflectSceneComponent {
    fn from_type() -> Self {
        Self {
            spawn: spawn::<T>,
            capture: capture::<T>,
        }
    }
}

fn spawn<T: Component + Reflect + Default>(
    world: &mut World,
    entity: Entity,
    stored: &ReflectedValue,
    node_entities: &[Entity],
) -> Vec<ValueWarning> {
    let mut value = T::default();
    let warnings = apply_value_with(
        &mut value,
        stored,
        &mut SpawnHook(SceneSpawn {
            world,
            node_entities,
        }),
    );
    world.insert(value, entity);
    warnings
}

fn capture<T: Component + Reflect>(
    world: &World,
    entity: Entity,
    capture: &mut SceneCapture,
) -> Option<Result<ReflectedValue, EncodeError>> {
    let value = world.get_component_for_entity::<T>(entity)?;
    Some(to_value_with(
        value,
        &mut CaptureHook {
            registry: world.get_resource::<AppTypeRegistry>().map(|r| r.read()),
            capture,
        },
    ))
}

/// How a leaf that needs the world is spawned from a scene and captured into one.
#[derive(Clone, Copy)]
pub struct ReflectSceneLeaf {
    pub spawn: fn(&mut dyn Reflect, &[u8], &SceneSpawn<'_>) -> Result<(), LeafError>,
    pub capture: fn(&dyn Reflect, &mut SceneCapture) -> Result<Vec<u8>, LeafError>,
}

impl TypeData for ReflectSceneLeaf {}

/// What a leaf sees while its scene is spawned.
pub struct SceneSpawn<'w> {
    pub world: &'w World,
    /// One entity per scene node, by node index.
    pub node_entities: &'w [Entity],
}

/// What a scene capture collects beside the values: node indices in, referenced assets out.
#[derive(Default)]
pub struct SceneCapture {
    node_indices: HashMap<Entity, usize>,
    referenced_assets: Vec<AssetId>,
}

impl SceneCapture {
    pub fn new(node_indices: HashMap<Entity, usize>) -> Self {
        Self {
            node_indices,
            referenced_assets: Vec::new(),
        }
    }

    /// The node index `entity` is captured as, if it is in the scene.
    pub fn index_of(&self, entity: Entity) -> Option<usize> {
        self.node_indices.get(&entity).copied()
    }

    /// Records an asset the captured values refer to; the nil id is not an asset.
    pub fn reference(&mut self, id: AssetId) {
        if !id.is_nil() && !self.referenced_assets.contains(&id) {
            self.referenced_assets.push(id);
        }
    }

    pub fn referenced_assets(&self) -> &[AssetId] {
        &self.referenced_assets
    }

    pub fn into_referenced_assets(self) -> Vec<AssetId> {
        self.referenced_assets
    }
}

struct SpawnHook<'w>(SceneSpawn<'w>);

impl LeafHook for SpawnHook<'_> {
    fn encode(&mut self, _: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        None
    }

    fn decode(&mut self, leaf: &mut dyn Reflect, bytes: &[u8]) -> Option<Result<(), LeafError>> {
        let spawn = self
            .0
            .world
            .get_resource::<AppTypeRegistry>()?
            .get_type_data::<ReflectSceneLeaf>((&*leaf as &dyn Any).type_id())?
            .spawn;
        Some(spawn(leaf, bytes, &self.0))
    }
}

struct CaptureHook<'a> {
    registry: Option<RwLockReadGuard<'a, TypeRegistry>>,
    capture: &'a mut SceneCapture,
}

impl LeafHook for CaptureHook<'_> {
    fn encode(&mut self, leaf: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        let capture = self
            .registry
            .as_ref()?
            .get_type_data::<ReflectSceneLeaf>((leaf as &dyn Any).type_id())?
            .capture;
        Some(capture(leaf, self.capture))
    }

    fn decode(&mut self, _: &mut dyn Reflect, _: &[u8]) -> Option<Result<(), LeafError>> {
        None
    }
}

/// A scene component an entity carries, as `World::component_types` lists it.
pub struct SceneComponentType {
    registration: Arc<TypeRegistration>,
    scene: ReflectSceneComponent,
}

impl SceneComponentType {
    pub(crate) fn new(registration: Arc<TypeRegistration>, scene: ReflectSceneComponent) -> Self {
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

    pub fn registration(&self) -> &TypeRegistration {
        &self.registration
    }

    /// The component's value off `entity` as scene data, mapping entities and recording assets in `capture`.
    pub fn capture(
        &self,
        world: &World,
        entity: Entity,
        capture: &mut SceneCapture,
    ) -> Result<ReflectedValue, EncodeError> {
        (self.scene.capture)(world, entity, capture).unwrap_or_else(|| {
            Err(EncodeError {
                path: String::new(),
                error: LeafError(format!("{} is not on {entity:?}", self.name())),
            })
        })
    }
}

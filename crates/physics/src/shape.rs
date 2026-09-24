use std::{collections::HashMap, sync::Arc};

use concerto_core::assets::{asset_store::AssetStore, handle::AssetLifetimeEvent, AssetId};
use concerto_ecs::{
    component::scene::{SceneComponent, SceneSpawnContext},
    events::event_reader::EventReader,
    CommandQueue, Component, Entity, Query, Res, ResMut, Resource, With, Without,
};
use concerto_mesh::{Mesh, MeshComponent};
use log::warn;
use serde::Serialize;

use crate::{
    aabb::Aabb,
    backend::{MeshShapeCreationError, PhysicsBackend},
    collider::Collider,
    ActiveBackend,
};

pub struct PhysicsShape(pub(crate) <ActiveBackend as PhysicsBackend>::ShapeHandle);

impl PhysicsShape {
    pub fn from_mesh(mesh: &Mesh) -> Result<Self, MeshShapeCreationError> {
        Ok(Self(ActiveBackend::create_shape_from_mesh(mesh)?))
    }

    pub fn create_sphere_shape(radius: f32) -> Self {
        Self(ActiveBackend::create_sphere_shape(radius))
    }

    pub fn create_cuboid_shape(width: f32, height: f32, length: f32) -> Self {
        Self(ActiveBackend::create_cuboid_shape(width, height, length))
    }

    pub fn create_capsule_shape(half_height: f32, radius: f32) -> Self {
        Self(ActiveBackend::create_capsule_shape(half_height, radius))
    }

    /// The shape's axis-aligned bounds (min, max) in its own local space,
    /// before any body pose or scale.
    pub fn local_aabb(&self) -> Aabb {
        <ActiveBackend as PhysicsBackend>::shape_local_aabb(&self.0)
    }
}

pub type SharedPhysicsShape = Arc<PhysicsShape>;

#[derive(Resource, Default)]
pub struct PhysicsMeshShapes {
    cache: HashMap<AssetId, SharedPhysicsShape>,
}

impl PhysicsMeshShapes {
    pub(crate) fn get_or_create_mesh_shape(
        &mut self,
        asset_id: AssetId,
        mesh: &Mesh,
    ) -> Result<SharedPhysicsShape, MeshShapeCreationError> {
        if let Some(shape) = self.cache.get(&asset_id) {
            return Ok(shape.clone());
        }

        let shape = SharedPhysicsShape::new(PhysicsShape::from_mesh(mesh)?);
        self.cache.insert(asset_id, shape.clone());
        Ok(shape)
    }

    pub(crate) fn drop_shape(&mut self, asset_id: &AssetId) {
        self.cache.remove(asset_id);
    }
}

#[derive(Component, Serialize)]
pub struct MeshCollider;

// The Blender add-on writes unit-struct extras as `{}`, but serde's derived
// impl for a unit struct only accepts JSON `null`. Accept any shape so a
// `"MeshCollider": {}` payload deserializes.
impl<'de> serde::Deserialize<'de> for MeshCollider {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_ignored_any(serde::de::IgnoredAny)?;
        Ok(MeshCollider)
    }
}

impl SceneComponent for MeshCollider {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

pub(crate) fn generate_mesh_shapes(
    meshes_to_generate: Query<(Entity, &MeshComponent), (With<MeshCollider>, Without<Collider>)>,
    meshes: Res<AssetStore<Mesh>>,
    mut mesh_shapes: ResMut<PhysicsMeshShapes>,
    mut cmd: CommandQueue,
) {
    for (entity, mesh_comp) in meshes_to_generate.iter() {
        let Some(mesh) = meshes.get(&mesh_comp.handle) else {
            continue;
        };

        // The mesh data will not change, so a rejected mesh would be rejected
        // identically every frame: drop the request rather than retrying.
        let shape = match mesh_shapes.get_or_create_mesh_shape(mesh_comp.handle.id(), mesh) {
            Ok(shape) => shape,
            Err(error) => {
                warn!("Skipping collider for entity {entity:?}: {error}");
                cmd.remove::<MeshCollider>(entity);
                continue;
            }
        };

        cmd.insert(Collider::from_shape(shape), entity);
    }
}

pub(crate) fn clean_shapes_for_dropped_meshes(
    mut mesh_shapes: ResMut<PhysicsMeshShapes>,
    mut dropped_events: EventReader<AssetLifetimeEvent>,
) {
    for e in dropped_events.read() {
        match e {
            AssetLifetimeEvent::Dropped(asset_id) => {
                mesh_shapes.drop_shape(asset_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MeshCollider;

    // The Blender add-on emits unit-struct extras as `{}`; a plain derived
    // `Deserialize` would only accept `null`. Both must round-trip.
    #[test]
    fn mesh_collider_deserializes_from_any_shape() {
        assert!(serde_json::from_str::<MeshCollider>("{}").is_ok());
        assert!(serde_json::from_str::<MeshCollider>("null").is_ok());
    }
}

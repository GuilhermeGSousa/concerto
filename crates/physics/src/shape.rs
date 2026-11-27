use std::{collections::HashMap, sync::Arc};

use concerto_ecs::{
    events::event_reader::EventReader,
    reflect::{ReflectComponent, ReflectSceneComponent},
    CommandQueue, Component, Entity, Query, Res, ResMut, Resource, With, Without,
};
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetLifetimeEvent, AssetId};
use concerto_mesh::{Mesh, MeshComponent};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;
use log::warn;

use crate::{
    aabb::Aabb,
    backend::{MeshShapeCreationError, PhysicsBackend},
    collider::{BoxCollider, Collider},
    ActiveBackend,
};

/// What a collision shape is, as plain data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    Sphere {
        radius: f32,
    },
    Cuboid {
        half_extents: Vec3,
    },
    /// A capsule along the local Y axis.
    Capsule {
        half_height: f32,
        radius: f32,
    },
    /// A triangle mesh; its bounds are the shape's `local_aabb`.
    Mesh,
}

pub struct PhysicsShape(
    pub(crate) <ActiveBackend as PhysicsBackend>::ShapeHandle,
    ShapeKind,
);

impl PhysicsShape {
    /// A triangle mesh shape built from the positions and indices of `mesh`.
    ///
    /// # Errors
    ///
    /// Returns [`MeshShapeCreationError`] when the active backend cannot build a triangle mesh
    /// shape from `mesh`.
    pub fn from_mesh(mesh: &Mesh) -> Result<Self, MeshShapeCreationError> {
        Ok(Self(
            ActiveBackend::create_shape_from_mesh(mesh)?,
            ShapeKind::Mesh,
        ))
    }

    pub fn create_sphere_shape(radius: f32) -> Self {
        Self(
            ActiveBackend::create_sphere_shape(radius),
            ShapeKind::Sphere { radius },
        )
    }

    pub fn create_cuboid_shape(width: f32, height: f32, length: f32) -> Self {
        Self(
            ActiveBackend::create_cuboid_shape(width, height, length),
            ShapeKind::Cuboid {
                half_extents: Vec3::new(width, height, length),
            },
        )
    }

    pub fn create_capsule_shape(half_height: f32, radius: f32) -> Self {
        Self(
            ActiveBackend::create_capsule_shape(half_height, radius),
            ShapeKind::Capsule {
                half_height,
                radius,
            },
        )
    }

    /// What the shape is, for drawing it.
    pub fn kind(&self) -> ShapeKind {
        self.1
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

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default, SceneComponent)]
pub struct MeshCollider;

pub(crate) fn generate_mesh_shapes(
    meshes_to_generate: Query<
        (Entity, &MeshComponent),
        (With<MeshCollider>, Without<Collider>, Without<BoxCollider>),
    >,
    meshes: Res<AssetStore<Mesh>>,
    mut mesh_shapes: ResMut<PhysicsMeshShapes>,
    mut cmd: CommandQueue,
) {
    // TODO: move this logic to component lifecycles
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

use concerto_ecs::component::scene::{SceneComponent, SceneSpawnContext};
use concerto_ecs::{Component, Entity};
use concerto_foundation::assets::{
    asset_server::AssetServer, handle::AssetHandle, Asset, LoadableAsset,
};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::primitive::Primitive;
use crate::vertex::Vertex;

#[derive(Asset, Default, serde::Serialize, serde::Deserialize)]
pub struct Mesh {
    pub primitives: Vec<Primitive>,
}

/// An axis-aligned bounding box in mesh-local or world space.
#[derive(Debug, Clone, Copy, PartialEq, Component)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn extent(self) -> Vec3 {
        self.max - self.min
    }

    /// Returns the world-space AABB containing all eight transformed corners.
    /// This remains correct for rotation and non-uniform scale.
    pub fn transformed(self, transform: Mat4) -> Self {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for x in [self.min.x, self.max.x] {
            for y in [self.min.y, self.max.y] {
                for z in [self.min.z, self.max.z] {
                    let point = transform.transform_point3(Vec3::new(x, y, z));
                    min = min.min(point);
                    max = max.max(point);
                }
            }
        }
        Self { min, max }
    }
}

impl LoadableAsset for Mesh {}

impl Mesh {
    /// A mesh with exactly one primitive, for procedurally built geometry.
    pub fn single(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self {
            primitives: vec![Primitive { vertices, indices }],
        }
    }

    /// The union of every primitive's local-space bounds, or `None` when no
    /// primitive has vertices.
    ///
    /// Callers that inspect the same loaded asset repeatedly should cache this
    /// value by asset id.
    pub fn local_aabb(&self) -> Option<Aabb> {
        self.primitives
            .iter()
            .filter_map(Primitive::local_aabb)
            .reduce(|a, b| Aabb {
                min: a.min.min(b.min),
                max: a.max.max(b.max),
            })
    }

    /// Every primitive concatenated into one vertex and index list, with each
    /// primitive's indices offset by the vertices preceding it.
    pub fn merged_geometry(&self) -> (Vec<Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for primitive in &self.primitives {
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&primitive.vertices);
            indices.extend(primitive.indices.iter().map(|index| index + base));
        }
        (vertices, indices)
    }

    pub fn compute_normals(&mut self) -> &mut Self {
        for primitive in &mut self.primitives {
            primitive.compute_normals();
        }
        self
    }

    pub fn compute_tangents(&mut self) -> &mut Self {
        for primitive in &mut self.primitives {
            primitive.compute_tangents();
        }
        self
    }
}

#[derive(Component, Serialize, Deserialize)]
pub struct MeshComponent {
    pub handle: AssetHandle<Mesh>,
}

impl SceneComponent for MeshComponent {
    fn apply(mut self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        if let Some(server) = ctx.get_resource::<AssetServer>() {
            self.handle = server.load(self.handle.id());
        }
        ctx.insert(self, entity);
    }
}

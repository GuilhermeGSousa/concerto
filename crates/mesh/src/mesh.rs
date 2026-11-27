use concerto_ecs::{
    reflect::{ReflectComponent, ReflectSceneComponent},
    Component,
};
use concerto_foundation::assets::{handle::AssetHandle, Asset, LoadableAsset};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Mat4, Vec2, Vec3};

use crate::vertex::Vertex;

/// An indexed triangle mesh.
#[derive(Asset, serde::Serialize, serde::Deserialize)]
pub struct Mesh {
    /// The vertices, in mesh-local space.
    pub vertices: Vec<Vertex>,
    /// Three indices into `vertices` per triangle.
    pub indices: Vec<u32>,
}

/// An axis-aligned bounding box in mesh-local or world space.
#[derive(Debug, Clone, Copy, PartialEq, Component)]
pub struct Aabb {
    /// The corner with the smallest coordinate on each axis.
    pub min: Vec3,
    /// The corner with the largest coordinate on each axis.
    pub max: Vec3,
}

impl Aabb {
    /// Returns the point midway between the corners.
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Returns the full size of the box on each axis.
    pub fn extent(self) -> Vec3 {
        self.max - self.min
    }

    /// The world-space box containing all eight corners transformed by `transform`.
    ///
    /// It remains correct for rotation and non-uniform scale.
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
    /// Computes this mesh's local-space bounds from its vertex positions.
    ///
    /// Returns `None` for an empty mesh. Callers that inspect the same loaded
    /// asset repeatedly should cache this value by asset id.
    pub fn local_aabb(&self) -> Option<Aabb> {
        let first = self.vertices.first()?;
        let mut min = Vec3::from(first.pos_coords);
        let mut max = min;
        for vertex in &self.vertices[1..] {
            let point = Vec3::from(vertex.pos_coords);
            min = min.min(point);
            max = max.max(point);
        }
        Some(Aabb { min, max })
    }

    /// Adds each triangle's face normal to its vertices' normals and normalizes the average.
    ///
    /// The normals should start at zero. Returns `self` for chaining.
    ///
    /// # Panics
    ///
    /// Panics if an index is out of range of `vertices`.
    pub fn compute_normals(&mut self) -> &mut Self {
        let mut triangles_included = vec![0u32; self.vertices.len()];

        self.indices.chunks(3).for_each(|chunk| {
            let i0 = chunk[0] as usize;
            let i1 = chunk[1] as usize;
            let i2 = chunk[2] as usize;

            let pos0 = Vec3::from(self.vertices[i0].pos_coords);
            let pos1 = Vec3::from(self.vertices[i1].pos_coords);
            let pos2 = Vec3::from(self.vertices[i2].pos_coords);

            let normal = (pos1 - pos0).cross(pos2 - pos0).normalize();

            for i in [i0, i1, i2] {
                self.vertices[i].normal = (normal + Vec3::from(self.vertices[i].normal)).into();
                triangles_included[i] += 1;
            }
        });

        for (i, n) in triangles_included.into_iter().enumerate() {
            if n > 0 {
                let denom = 1.0 / n as f32;
                self.vertices[i].normal = (Vec3::from(self.vertices[i].normal) * denom)
                    .normalize()
                    .into();
            }
        }

        self
    }

    /// Sets each vertex's tangent and bitangent from the triangles' texture coordinates.
    ///
    /// When every UV is zero, it picks any tangent perpendicular to the normal instead. Otherwise
    /// the tangents should start at zero; each vertex gets the average over its triangles.
    /// Returns `self` for chaining.
    ///
    /// # Panics
    ///
    /// Panics if an index is out of range of `vertices`.
    pub fn compute_tangents(&mut self) -> &mut Self {
        let has_uvs = self.vertices.iter().any(|v| v.uv_coords != [0.0, 0.0]);

        if !has_uvs {
            for v in self.vertices.iter_mut() {
                let normal = Vec3::from(v.normal);
                let t = if normal.x.abs() > normal.y.abs() {
                    Vec3::new(normal.z, 0.0, -normal.x).normalize()
                } else {
                    Vec3::new(0.0, -normal.z, normal.y).normalize()
                };
                v.tangent = t.into();
                v.bitangent = normal.cross(t).into();
            }
            return self;
        }

        let mut triangles_included = vec![0u32; self.vertices.len()];

        self.indices.chunks(3).for_each(|chunk| {
            let i0 = chunk[0] as usize;
            let i1 = chunk[1] as usize;
            let i2 = chunk[2] as usize;

            let pos0 = Vec3::from(self.vertices[i0].pos_coords);
            let pos1 = Vec3::from(self.vertices[i1].pos_coords);
            let pos2 = Vec3::from(self.vertices[i2].pos_coords);

            let uv0 = Vec2::from(self.vertices[i0].uv_coords);
            let uv1 = Vec2::from(self.vertices[i1].uv_coords);
            let uv2 = Vec2::from(self.vertices[i2].uv_coords);

            let delta_pos1 = pos1 - pos0;
            let delta_pos2 = pos2 - pos0;
            let delta_uv1 = uv1 - uv0;
            let delta_uv2 = uv2 - uv0;

            let r = 1.0 / (delta_uv1.x * delta_uv2.y - delta_uv1.y * delta_uv2.x);
            let tangent = (delta_pos1 * delta_uv2.y - delta_pos2 * delta_uv1.y) * r;
            let bitangent = (delta_pos2 * delta_uv1.x - delta_pos1 * delta_uv2.x) * -r;

            for i in [i0, i1, i2] {
                self.vertices[i].tangent = (tangent + Vec3::from(self.vertices[i].tangent)).into();
                self.vertices[i].bitangent =
                    (bitangent + Vec3::from(self.vertices[i].bitangent)).into();
                triangles_included[i] += 1;
            }
        });

        for (i, n) in triangles_included.into_iter().enumerate() {
            if n > 0 {
                let denom = 1.0 / n as f32;
                let v = &mut self.vertices[i];
                v.tangent = (Vec3::from(v.tangent) * denom).into();
                v.bitangent = (Vec3::from(v.bitangent) * denom).into();
            }
        }

        self
    }
}

/// Draws a [`Mesh`] at the entity's transform.
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default, SceneComponent)]
pub struct MeshComponent {
    /// The mesh to draw.
    pub handle: AssetHandle<Mesh>,
}

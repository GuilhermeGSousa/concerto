//! Meshes, skeletons and the geometry around them: vertices, bounding boxes, rays and primitive
//! shapes.

/// Local-space bounding boxes kept on entities that draw a mesh.
pub mod bounds;
/// The mesh asset, its component and its bounding box.
pub mod mesh;
mod primitives;
/// Rays and their intersection with boxes and meshes.
pub mod ray;
/// Skeletons, their poses and the component that binds one to scene entities.
pub mod skeleton;
/// The vertex layout shared by meshes and the renderer.
pub mod vertex;

pub use bounds::{update_mesh_bounds, AabbSource};
pub use mesh::{Aabb, Mesh, MeshComponent};
pub use ray::Ray;
pub use skeleton::{
    JointName, JointPose, ReflectSkeletonSource, Skeleton, SkeletonComponent, SkeletonError,
    SkeletonSource,
};
pub use vertex::Vertex;

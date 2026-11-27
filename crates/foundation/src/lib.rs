//! The engine's foundation: assets and their loading, transforms, time, blending, springs and other
//! shared maths.

/// Content assets: the [`Asset`](assets::Asset) trait, handles, the
/// [`AssetServer`](assets::asset_server::AssetServer) that loads them, and the stores that hold
/// them.
pub mod assets;
/// [`Blendable`](blend::Blendable), values that can be interpolated, and its derive.
pub mod blend;
pub use concerto_tasks as tasks;
/// 2D geometry, such as Delaunay triangulation.
pub mod geometry;
/// Hashing whose output is stable across builds and platforms.
pub mod hash;
/// Springs, damping and other smoothing maths.
pub mod math;
/// [`Time`](time::Time), the frame and fixed-step clock, and frame statistics.
pub mod time;
/// [`Transform`](transform::Transform) and [`GlobalTransform`](transform::GlobalTransform), and the
/// systems that propagate one into the other down the hierarchy.
pub mod transform;
/// [`AsAny`](utils::AsAny), for downcasting trait objects.
pub mod utils;

/// Cameras, what they render to, and their render-world form.
pub mod camera;
/// Fog and ambient light around a camera.
pub mod camera_environment;
/// Directional, point and spot lights.
pub mod light;
/// The component that picks a mesh's material.
pub mod material;
/// The links between main-world entities and their render-world mirrors.
pub mod render_entity;
/// Opting meshes out of frustum culling and shadow casting.
pub mod visibility;

pub(crate) mod mesh;
pub(crate) mod shadows;
pub(crate) mod skeleton;
pub(crate) mod transform;

pub use camera::Camera;
pub use camera_environment::CameraEnvironment;
pub use light::Light;
pub use material::MaterialComponent;
pub use render_entity::RenderEntity;
pub use visibility::{NoFrustumCulling, NotShadowCaster};

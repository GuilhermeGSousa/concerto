// Allow the `concerto_render_macros`-generated code (which uses `concerto_render::` paths) to
// work when the derive is applied inside this crate itself.
extern crate self as concerto_render;

pub mod assets;
pub mod components;
pub mod device;
pub mod importers;
pub mod layouts;
pub mod material_plugin;
pub mod plugin;
pub mod queue;
pub mod render_asset;
pub mod resources;
pub mod shadow_pipeline;
pub mod systems;
pub mod wgpu_wrapper;

/// Re-export wgpu so downstream crates that use `#[derive(AsBindGroup)]` can
/// bring `wgpu` into scope via `use concerto_render::wgpu` without a direct dependency.
pub use wgpu;

/// Re-export the `AsBindGroup` derive macro so crates that depend on `render`
/// do not need to add `render-macros` as a separate dependency.
pub use concerto_render_macros::AsBindGroup;

/// Re-export the `Material` trait and plugin types for convenience.
pub use assets::material::Material;
pub use components::material::MaterialComponent;
pub use material_plugin::{MaterialPipeline, MaterialPlugin, RenderMaterial};

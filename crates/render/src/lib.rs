//! The wgpu renderer of Concerto.
//!
//! [`RenderPlugin`](plugin::RenderPlugin) creates the GPU device and the render
//! world's schedules, extracts cameras, meshes, lights and skeletons from the main
//! world every frame, and presents the result. Materials are added with
//! [`MaterialPlugin`], shadows with
//! [`ShadowPipelinePlugin`](shadow_pipeline::ShadowPipelinePlugin), and systems are
//! ordered against the renderer through [`RenderSet`](sets::RenderSet).
//!
//! # Examples
//!
//! ```no_run
//! use concerto_app::App;
//! use concerto_render::{plugin::RenderPlugin, shadow_pipeline::ShadowPipelinePlugin};
//!
//! let mut app = App::new();
//! app.register_plugin(RenderPlugin)
//!     .register_plugin(ShadowPipelinePlugin);
//! app.run();
//! ```

// Allow the `concerto_render_macros`-generated code (which uses `concerto_render::` paths) to
// work when the derive is applied inside this crate itself.
extern crate self as concerto_render;

/// Assets the renderer consumes: meshes, textures, materials and skeletons.
pub mod assets;
/// Components describing what to render and their render-world counterparts.
pub mod components;
/// The GPU device resource.
pub mod device;
/// Importers turning source files into render assets.
pub mod importers;
/// Bind group layouts shared by render pipelines.
pub mod layouts;
/// Rendering meshes with a [`Material`].
pub mod material_plugin;
/// The core [`RenderPlugin`](plugin::RenderPlugin).
pub mod plugin;
/// The GPU queue resource.
pub mod queue;
/// GPU-side copies of assets, prepared from their main-world versions.
pub mod render_asset;
/// Render-world resources such as the surface configuration.
pub mod resources;
/// System sets for ordering against the renderer.
pub mod sets;
/// Shadow map rendering.
pub mod shadow_pipeline;
/// Systems that finish, present and resize frames.
pub mod systems;
/// A wrapper making wgpu handles `Send` and `Sync`.
pub mod wgpu_wrapper;

pub use wgpu;

pub use concerto_render_macros::AsBindGroup;

pub use assets::material::Material;
pub use components::material::MaterialComponent;
pub use material_plugin::{MaterialPipeline, MaterialPlugin, RenderMaterial};

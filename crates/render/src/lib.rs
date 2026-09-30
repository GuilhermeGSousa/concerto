//! The wgpu renderer of Concerto.
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

pub mod assets;
pub mod capabilities;
pub mod components;
pub mod device;
pub mod importers;
pub mod layouts;
pub mod material_plugin;
pub mod plugin;
pub mod queue;
pub mod render_asset;
pub mod resources;
pub mod sets;
pub mod shader;
pub mod shadow_pipeline;
pub mod systems;
pub mod wgpu_wrapper;

pub use wgpu;

pub use concerto_render_macros::AsBindGroup;

pub use assets::material::Material;
pub use components::material::MaterialComponent;
pub use material_plugin::{MaterialPipeline, MaterialPlugin, RenderMaterial};

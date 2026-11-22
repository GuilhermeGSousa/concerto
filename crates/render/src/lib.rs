//! The wgpu renderer of Concerto.
//!
//! # Examples
//!
//! ```no_run
//! use concerto_app::App;
//! use concerto_render::{plugin::RenderPlugin, shadow_pipeline::ShadowPipelinePlugin};
//!
//! fn game(app: &mut App) {
//!     app.register_plugin(RenderPlugin)
//!         .register_plugin(ShadowPipelinePlugin);
//! }
//! ```

// Allow the `concerto_render_macros`-generated code (which uses `concerto_render::` paths) to
// work when the derive is applied inside this crate itself.
extern crate self as concerto_render;

pub mod ambient_occlusion;
pub mod assets;
pub mod auto_exposure;
pub mod batching;
pub mod bloom;
pub mod capabilities;
pub mod components;
pub mod device;
pub mod draw_target;
pub(crate) mod frustum;
pub mod gpu;
pub mod importers;
#[cfg(test)]
pub(crate) mod layout_test;
pub mod layouts;
pub mod material_plugin;
pub mod plugin;
pub mod queue;
pub mod readback;
pub mod render_asset;
pub mod resources;
pub mod sets;
pub mod shader;
pub mod shadow_pipeline;
pub mod sky_light;
pub mod systems;
pub mod tonemapping;
pub mod view_phase;
pub mod view_target;
pub mod wgpu_wrapper;

#[doc(hidden)]
pub use bytemuck;
pub use wgpu;

pub use concerto_render_macros::AsBindGroup;

pub use ambient_occlusion::{AmbientOcclusionPlugin, ScreenSpaceAmbientOcclusion};
pub use assets::material::Material;
pub use auto_exposure::{AutoExposure, AutoExposurePlugin};
pub use bloom::{Bloom, BloomPlugin};
pub use components::camera::ExclusiveCamera;
pub use components::material::MaterialComponent;
pub use material_plugin::{MaterialPipeline, MaterialPlugin, RenderMaterial};
pub use sky_light::{
    CameraSkyLight, LightTint, RadianceSample, SkyRadianceMap, SphericalHarmonics,
    SKY_RADIANCE_SCALE,
};
pub use tonemapping::{Exposure, Tonemapping};
pub use view_target::{Msaa, ViewPipelines, ViewTarget};

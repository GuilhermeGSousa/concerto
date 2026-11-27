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

/// Screen-space ambient occlusion, which darkens creases and contact points.
pub mod ambient_occlusion;
/// The render asset types: materials, meshes, skeletons, textures and vertex layouts.
pub mod assets;
/// Automatic exposure, which adapts a camera's brightness to the scene as an eye does.
pub mod auto_exposure;
/// Grouping draws that share a mesh and material into instanced batches, and their stats.
pub mod batching;
/// Bloom, the glow that spreads from bright parts of the image.
pub mod bloom;
/// The GPU features the renderer adapts to.
pub mod capabilities;
/// The components that put cameras, lights and meshes into the render world.
pub mod components;
/// Depth of field, which blurs what is out of a camera's focus.
pub mod depth_of_field;
pub(crate) mod depth_pass;
/// The wgpu device as a render-world resource.
pub mod device;
/// Where an `App`'s window cameras and UI draw: a window surface or a host's texture.
pub mod draw_target;
pub(crate) mod frustum;
/// Requesting the GPU an `App` renders with.
pub mod gpu;
/// Importers that turn source images into texture assets.
pub mod importers;
#[cfg(test)]
pub(crate) mod layout_test;
/// The engine bind group layouts that material pipelines share.
pub mod layouts;
pub mod material_plugin;
/// The plugin that sets up the renderer and its render world.
pub mod plugin;
/// The wgpu queue as a render-world resource.
pub mod queue;
/// Copying textures from the GPU back to the CPU.
pub mod readback;
/// GPU copies of assets, prepared from the main world's assets within a per-frame budget.
pub mod render_asset;
/// Render-world resources describing the surface being drawn to.
pub mod resources;
/// The system sets that order the render world's work within a frame.
pub mod sets;
pub mod shader;
/// Shadow maps for directional, point and spot lights.
pub mod shadow_pipeline;
/// Sky lighting: irradiance as spherical harmonics and a radiance map for reflections.
pub mod sky_light;
/// Render-world systems that draw the frame and follow the window.
pub mod systems;
/// Exposure and tonemapping, which map HDR colour to the display.
pub mod tonemapping;
/// The draws queued for each camera, sorted and issued by phase.
pub mod view_phase;
/// The textures each camera's scene is drawn into, and multisampling.
pub mod view_target;
/// A wrapper that makes wgpu types `Send` and `Sync` on every target.
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
pub use depth_of_field::{DepthOfField, DepthOfFieldPlugin};
pub use material_plugin::{MaterialPipeline, MaterialPlugin, RenderMaterial};
pub use sky_light::{
    CameraSkyLight, LightTint, RadianceSample, SkyRadianceMap, SphericalHarmonics,
    SKY_RADIANCE_SCALE,
};
pub use tonemapping::{Exposure, Tonemapping};
pub use view_target::{Msaa, ViewPipelines, ViewTarget};

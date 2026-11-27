//! Tests of the renderer's public behaviour: materials, textures, cameras, shadows, post-processing
//! and the encodings of render assets.

mod alpha_to_coverage;
mod ambient_occlusion;
mod blended_materials;
mod bloom;
mod camera_environment_edit;
mod camera_render_target;
mod camera_viewport;
mod compressed_textures;
mod cutout_shadows;
mod depth_of_field;
mod exclusive_camera;
mod hdr;
mod hosted_target;
mod material_serialization;
mod msaa;
mod point_shadows;
mod readback;
mod required_sync;
mod scene_component_impls;
mod shadow_cascades;
mod texture_derive;
mod texture_pipeline_e2e;
mod unlit_view;
mod webgl2_shaders;

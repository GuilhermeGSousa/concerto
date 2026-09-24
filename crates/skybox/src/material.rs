use concerto_foundation::assets::{Asset, handle::AssetHandle};

use concerto_render::{
    AsBindGroup,
    assets::{
        texture::Texture,
        vertex::{Vertex, VertexBufferLayout},
    },
};

/// Material used by the skybox render pass.
///
/// Wraps an optional cube-map texture handle and implements [`AsBindGroup`]
/// via the derive macro, making the bind-group layout and bind-group creation
/// fully macro-generated.
///
/// The material is placed at `@group(0)` in the skybox shader, and the
/// standard camera uniform lives at `@group(1)`.
///
/// # Differences from regular mesh materials
///
/// * `cull_mode = "front"` – renders the inside faces of the skybox cube.
/// * `depth_stencil = "read_only"` – depth test without write, `LessEqual`
///   compare so the skybox fills only sky-colored pixels (furthest possible).
/// * `vertex_layouts` uses only [`Vertex`] – no per-instance transform buffer.
/// * This material is intended to be registered via
///   [`crate::material_plugin::MaterialPlugin::pipeline_only`] which creates
///   the [`crate::material_plugin::MaterialPipeline`] resource without adding
///   the generic mesh rendering systems.
#[derive(Asset, AsBindGroup, serde::Serialize, serde::Deserialize)]
#[material(
    vertex_shader = include_str!("shaders/skybox.wgsl"),
    fragment_shader = include_str!("shaders/skybox.wgsl"),
    cull_mode = "front",
    depth_stencil = "read_only",
    vertex_layouts = vec![Vertex::describe()],
)]
pub struct SkyboxMaterial {
    /// The cube-map texture (binding 0) and its sampler (binding 1).
    #[texture(0, dimension = "cube")]
    #[sampler(1)]
    pub texture: Option<AssetHandle<Texture>>,
}

use bytemuck::{Pod, Zeroable};
use concerto_color::LinearRgba;
use concerto_core::assets::Asset;
use concerto_render::AsBindGroup;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct WorldGridUniform {
    pub line_color: LinearRgba,
    pub cell_size: f32,
    pub coarse_cells: f32,
    pub fade_start: f32,
    pub fade_end: f32,
    pub surface_color: LinearRgba,
}

#[derive(Asset, AsBindGroup, serde::Serialize, serde::Deserialize)]
#[material(
    vertex_shader   = include_str!("shaders/world_grid.wgsl"),
    fragment_shader = include_str!("shaders/world_grid.wgsl"),
    lighting        = true,
    cull_mode       = "none",
    clear_depth     = false,
    blend           = "alpha",
)]
pub struct WorldGridMaterial {
    #[uniform(0)]
    pub uniform: WorldGridUniform,
}

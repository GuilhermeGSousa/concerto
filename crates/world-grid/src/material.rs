use bytemuck::{Pod, Zeroable};
use concerto_color::LinearRgba;
use concerto_foundation::assets::{Asset, LoadableAsset};
use concerto_render::AsBindGroup;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
/// The grid's settings as the shader reads them, copied from a [`WorldGrid`](crate::WorldGrid).
pub struct WorldGridUniform {
    /// The colour of the lines, alpha included.
    pub line_color: LinearRgba,
    /// The side of a fine cell, in world units.
    pub cell_size: f32,
    /// How many fine cells span a coarse cell, whose lines are brighter.
    pub coarse_cells: f32,
    /// The horizontal distance from the camera, in world units, where the grid starts to fade.
    pub fade_start: f32,
    /// The horizontal distance from the camera, in world units, where the grid has faded out.
    pub fade_end: f32,
    /// The colour of the plane between the lines, alpha included.
    pub surface_color: LinearRgba,
    /// 1 to shade the grid with the scene's lights, 0 to draw it at full brightness.
    pub lit: f32,
    /// Pads the uniform to a multiple of 16 bytes; always zero.
    pub _padding: [f32; 3],
}

#[derive(Asset, AsBindGroup, serde::Serialize, serde::Deserialize)]
#[material(
    vertex_shader   = include_str!("shaders/world_grid.wgsl"),
    fragment_shader = include_str!("shaders/world_grid.wgsl"),
    lighting        = true,
    cull_mode       = "none",
    blend           = "alpha",
)]
/// The material of a [`WorldGrid`](crate::WorldGrid), which draws the grid on a full-screen
/// triangle and writes the depth of the ground plane.
pub struct WorldGridMaterial {
    /// The grid's settings, bound at binding 0.
    #[uniform(0)]
    pub uniform: WorldGridUniform,
}

impl LoadableAsset for WorldGridMaterial {}

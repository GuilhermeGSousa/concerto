//! An infinite reference grid on the world's ground plane, `y = 0`, drawn by a [`WorldGrid`]
//! entity.

/// [`WorldGridMaterial`](material::WorldGridMaterial), the material the grid draws with.
pub mod material;
/// [`WorldGridPlugin`](plugin::WorldGridPlugin), which draws [`WorldGrid`] entities.
pub mod plugin;
/// The [`WorldGrid`] component.
pub mod world_grid;

pub use world_grid::WorldGrid;

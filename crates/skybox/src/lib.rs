//! A physically based sky for Concerto: an atmosphere lit by a sun light, drawn behind a camera's
//! scene and used to light it.

/// The atmosphere model: air density, optical depth and the lookup tables built from them.
pub mod atmosphere;
/// The lookup tables of the active atmosphere and the sun's tint and sky lighting they give.
pub mod lighting;
/// The plugins that add the sky.
pub mod plugin;
/// Drawing the sky in the render world.
pub mod render;
/// The [`Sun`], [`Atmosphere`] and [`Sky`] components and settings.
pub mod sun;

pub use lighting::AtmosphereLuts;
pub use plugin::{SkyPlugin, SkyboxPlugin};
pub use render::SkyRenderPlugin;
pub use sun::{Atmosphere, Sky, Sun};

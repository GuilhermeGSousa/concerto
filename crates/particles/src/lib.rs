//! Particles for Concerto: emitters simulated on the CPU and drawn as billboards in a camera's transparent phase.

mod emitter;
mod plugin;
mod render;
mod simulation;

pub use emitter::ParticleEmitter;
pub use plugin::ParticlesPlugin;
pub use simulation::{Particle, Particles};

/// The WGSL source of the particle shader, for tools that check shaders.
pub const PARTICLE_SHADER: &str = include_str!("shaders/particles.wgsl");

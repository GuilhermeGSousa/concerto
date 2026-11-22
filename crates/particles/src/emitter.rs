use concerto_color::Color;
use concerto_ecs::{
    component::Component,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;

use crate::simulation::Particles;

/// Emits particles from its entity's transform: soft round billboards that move, grow and fade over their lifetime.
#[derive(Component, Clone, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
#[require(Particles)]
pub struct ParticleEmitter {
    /// Particles emitted each second; 0 emits none and lets the living ones die out.
    pub rate: f32,
    /// The most particles alive at once; emission waits while it is reached.
    pub capacity: u32,
    /// Seconds a particle lives.
    pub lifetime: f32,
    /// Half the size of the box particles appear in, in the emitter's space.
    pub extent: Vec3,
    /// The velocity a particle starts with, in the emitter's space.
    pub velocity: Vec3,
    /// How far a particle's starting velocity may differ from `velocity` on each axis.
    pub velocity_spread: f32,
    /// Acceleration in world space: gravity, or buoyancy for smoke and steam.
    pub acceleration: Vec3,
    /// How quickly a particle loses its velocity to the air, per second.
    pub drag: f32,
    /// A particle's diameter when it appears, in world units.
    pub start_size: f32,
    /// A particle's diameter when it dies.
    pub end_size: f32,
    /// A particle's colour when it appears; its alpha is how opaque the particle is, and values above 1 glow under bloom.
    pub start_color: Color,
    /// A particle's colour when it dies.
    pub end_color: Color,
    /// Whether the scene's lights and ambient light colour the particles; unlit particles show their colour as it is.
    pub lit: bool,
    /// Whether particles add their light to what is behind them, as fire and sparks do, instead of covering it.
    pub additive: bool,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            rate: 20.0,
            capacity: 256,
            lifetime: 2.0,
            extent: Vec3::ZERO,
            velocity: Vec3::Y,
            velocity_spread: 0.2,
            acceleration: Vec3::ZERO,
            drag: 0.0,
            start_size: 0.1,
            end_size: 0.1,
            start_color: Color::rgba(1.0, 1.0, 1.0, 1.0),
            end_color: Color::rgba(1.0, 1.0, 1.0, 0.0),
            lit: true,
            additive: false,
        }
    }
}

impl ParticleEmitter {
    /// Snow falling through a box of half-size `extent` around the emitter.
    pub fn snow(extent: Vec3) -> Self {
        Self {
            rate: 400.0,
            capacity: 2048,
            lifetime: 5.0,
            extent,
            velocity: Vec3::new(0.3, -1.2, 0.0),
            velocity_spread: 0.3,
            start_size: 0.035,
            end_size: 0.035,
            start_color: Color::rgba(1.0, 1.0, 1.0, 0.9),
            end_color: Color::rgba(1.0, 1.0, 1.0, 0.9),
            ..Self::default()
        }
    }

    /// Steam or breath rising and thinning out; scale `rate` and the sizes to the source.
    pub fn steam() -> Self {
        Self {
            rate: 30.0,
            capacity: 128,
            lifetime: 2.5,
            extent: Vec3::splat(0.05),
            velocity: Vec3::new(0.0, 0.5, 0.0),
            velocity_spread: 0.15,
            acceleration: Vec3::new(0.0, 0.15, 0.0),
            drag: 0.6,
            start_size: 0.15,
            end_size: 0.9,
            start_color: Color::rgba(1.0, 1.0, 1.0, 0.35),
            end_color: Color::rgba(1.0, 1.0, 1.0, 0.0),
            ..Self::default()
        }
    }

    /// Embers thrown up from a fire, glowing and falling back.
    pub fn embers() -> Self {
        Self {
            rate: 40.0,
            capacity: 256,
            lifetime: 1.8,
            extent: Vec3::new(0.15, 0.02, 0.15),
            velocity: Vec3::new(0.0, 1.6, 0.0),
            velocity_spread: 0.6,
            acceleration: Vec3::new(0.0, -0.8, 0.0),
            drag: 0.4,
            start_size: 0.03,
            end_size: 0.01,
            start_color: Color::rgba(24.0, 9.0, 1.5, 1.0),
            end_color: Color::rgba(6.0, 0.6, 0.0, 0.0),
            lit: false,
            additive: true,
        }
    }

    /// Sets the particles emitted each second.
    pub fn with_rate(mut self, rate: f32) -> Self {
        self.rate = rate;
        self
    }

    /// Sets the most particles alive at once.
    pub fn with_capacity(mut self, capacity: u32) -> Self {
        self.capacity = capacity;
        self
    }

    /// Sets the seconds a particle lives.
    pub fn with_lifetime(mut self, lifetime: f32) -> Self {
        self.lifetime = lifetime;
        self
    }

    /// Sets half the size of the box particles appear in.
    pub fn with_extent(mut self, extent: Vec3) -> Self {
        self.extent = extent;
        self
    }

    /// Sets the starting velocity and how far it may vary on each axis.
    pub fn with_velocity(mut self, velocity: Vec3, spread: f32) -> Self {
        self.velocity = velocity;
        self.velocity_spread = spread;
        self
    }

    /// Sets the world-space acceleration.
    pub fn with_acceleration(mut self, acceleration: Vec3) -> Self {
        self.acceleration = acceleration;
        self
    }

    /// Sets how quickly a particle loses its velocity.
    pub fn with_drag(mut self, drag: f32) -> Self {
        self.drag = drag;
        self
    }

    /// Sets a particle's diameter when it appears and when it dies.
    pub fn with_size(mut self, start: f32, end: f32) -> Self {
        self.start_size = start;
        self.end_size = end;
        self
    }

    /// Sets a particle's colour when it appears and when it dies.
    pub fn with_color(mut self, start: Color, end: Color) -> Self {
        self.start_color = start;
        self.end_color = end;
        self
    }

    /// Sets whether the scene's light colours the particles.
    pub fn with_lit(mut self, lit: bool) -> Self {
        self.lit = lit;
        self
    }

    /// Sets whether particles add their light to what is behind them.
    pub fn with_additive(mut self, additive: bool) -> Self {
        self.additive = additive;
        self
    }
}

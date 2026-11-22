use concerto_ecs::{component::Component, query::Query, resource::Res};
use concerto_foundation::{time::Time, transform::GlobalTransform};
use concerto_noise::{mix_seed, unit_from_seed};
use glam::{Mat4, Vec3};

use crate::emitter::ParticleEmitter;

/// One living particle, in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Seconds since it appeared.
    pub age: f32,
}

/// The living particles of a [`ParticleEmitter`] on the same entity; not saved.
#[derive(Component, Default)]
pub struct Particles {
    alive: Vec<Particle>,
    pending: f32,
    emitted: u64,
}

impl Particles {
    /// The living particles, oldest first.
    pub fn alive(&self) -> &[Particle] {
        &self.alive
    }

    /// Moves and ages the particles by `seconds`, drops the dead, and emits what `emitter` is due from `world_from_emitter`.
    pub fn advance(&mut self, emitter: &ParticleEmitter, world_from_emitter: Mat4, seconds: f32) {
        let slowed = (-emitter.drag.max(0.0) * seconds).exp();
        for particle in &mut self.alive {
            particle.age += seconds;
            particle.velocity = (particle.velocity + emitter.acceleration * seconds) * slowed;
            particle.position += particle.velocity * seconds;
        }
        self.alive
            .retain(|particle| particle.age < emitter.lifetime);

        if emitter.rate <= 0.0 || emitter.lifetime <= 0.0 {
            self.pending = 0.0;
            return;
        }
        let capacity = emitter.capacity as usize;
        if self.alive.capacity() < capacity {
            self.alive.reserve_exact(capacity - self.alive.len());
        }
        self.pending += emitter.rate * seconds;
        while self.pending >= 1.0 {
            if self.alive.len() >= capacity {
                self.pending = 1.0;
                break;
            }
            self.pending -= 1.0;
            let seed = self.emitted;
            self.emitted += 1;
            let signed = |stream: u64| unit_from_seed(mix_seed(seed, stream)) * 2.0 - 1.0;
            let place = emitter.extent * Vec3::new(signed(0), signed(1), signed(2));
            let push = emitter.velocity
                + emitter.velocity_spread * Vec3::new(signed(3), signed(4), signed(5));
            self.alive.push(Particle {
                position: world_from_emitter.transform_point3(place),
                velocity: world_from_emitter.transform_vector3(push),
                age: 0.0,
            });
        }
    }
}

pub(crate) fn simulate_particles(
    time: Res<Time>,
    mut emitters: Query<(&ParticleEmitter, &GlobalTransform, &mut Particles)>,
) {
    let seconds = time.delta().as_secs_f32();
    for (emitter, transform, mut particles) in emitters.iter_mut() {
        particles.advance(emitter, transform.matrix(), seconds);
    }
}

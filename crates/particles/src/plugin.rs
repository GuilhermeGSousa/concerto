use concerto_app::{
    edit_time::AtEditTime,
    plugins::Plugin,
    schedule_groups::{Extract, Render, Update},
    App,
};
use concerto_ecs::IntoSystemConfig;
use concerto_render::{sets::RenderSet, view_phase::clear_view_phases};

use crate::{
    emitter::ParticleEmitter,
    render::{extract_particles, queue_particles, ExtractedParticles, ParticleDraw},
    simulation::simulate_particles,
};

/// Simulates every [`ParticleEmitter`] and draws its particles in each camera's transparent phase.
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](concerto_render::plugin::RenderPlugin)
/// was not registered before it.
pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<ParticleEmitter>();
        app.add_system(Update, simulate_particles.at_edit_time());
        app.render_mut()
            .insert_resource(ExtractedParticles::default());
        app.add_render_system(Extract, extract_particles)
            .add_render_system(
                Render,
                queue_particles
                    .in_set(RenderSet::Queue)
                    .after(clear_view_phases),
            );
    }

    fn finish(&self, app: &mut App) {
        let draw = ParticleDraw::new(app.render());
        app.render_mut().insert_resource(draw);
    }
}

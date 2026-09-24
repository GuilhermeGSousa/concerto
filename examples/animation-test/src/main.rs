use concerto::{
    app::{
        schedule_groups::{Startup, Update},
        App,
    },
    ecs::command::CommandQueue,
    render::components::light::{Light, LightType},
    DefaultPlugins,
};
use concerto_color::Color;
use concerto_gameplay::{movement::first_person_player_fly, player::spawn_first_person_player};
use glam::Vec3;

use concerto_debug_gizmos::DebugGizmosPlugin;
use concerto_world_grid::WorldGrid;

use crate::demo_overlay::{draw_entity_gizmos, spawn_overlay, update_overlay};
use crate::movement_animation::{setup_animations, spawn_character, update_movement};

mod demo_overlay;
mod movement_animation;

fn main() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Debug).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }

    let mut app = App::new();
    app.register_plugin(DefaultPlugins::default());
    app.register_plugin(DebugGizmosPlugin);

    // Startup: camera + light, the animated character, and the debug overlay.
    app.add_system(Startup, spawn_camera)
        .add_system(Startup, spawn_character)
        .add_system(Startup, spawn_overlay);

    // Update: fly camera, blend-space graph setup, movement input, and the
    // debug overlay / gizmos.
    app.add_system(Update, first_person_player_fly)
        .add_system(Update, setup_animations)
        .add_system(Update, update_movement)
        .add_system(Update, update_overlay)
        .add_system(Update, draw_entity_gizmos);

    app.run();
}

fn spawn_camera(mut cmd: CommandQueue) {
    cmd.spawn(WorldGrid::default());
    // First-person fly camera with a headlight so the character is lit wherever you look.
    spawn_first_person_player(
        &mut cmd,
        Vec3::new(0.0, 1.0, 0.0),
        Light {
            color: Color::rgba(1.0, 1.0, 1.0, 1.0),
            intensity: 20.0,
            light_type: LightType::Point,
            shadowmaps_enabled: false,
        },
    );
}

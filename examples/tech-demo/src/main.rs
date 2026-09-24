use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    ui::frame_stats_overlay::FrameStatsOverlayPlugin,
};

use crate::{
    character::{
        PlayerSpawner, face_camera_direction, setup_character_animations, spawn_character,
        update_movement,
    },
    scene::spawn_scene,
    ui::{spawn_grounded_overlay, update_grounded_overlay},
};

mod character;
mod scene;
mod ui;

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
    app.register_plugin(DefaultPlugins::default())
        .register_plugin(FrameStatsOverlayPlugin);

    app.register_scene_component::<PlayerSpawner>();

    app.add_system(Update, spawn_character)
        .add_system(Startup, spawn_scene)
        .add_system(Startup, spawn_grounded_overlay)
        .add_system(Update, setup_character_animations)
        .add_system(Update, update_movement)
        .add_system(Update, face_camera_direction)
        .add_system(Update, update_grounded_overlay);

    app.run();
}

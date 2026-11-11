use concerto::{DefaultPlugins, app::App, asset_id};

pub mod book;
pub mod book_ui;
pub mod casefile;
pub mod controls;
pub mod house;
pub mod level;
pub mod players;
pub mod rules;
pub mod save;
pub mod schedule;
pub mod session;
pub mod world;

use crate::clock::NightClockPlugin;

/// The case schema and its loader.
pub use one_more_night_case as case;

pub mod clock;

/// Builds the game; the standalone binary and the editor both call it.
pub fn game(app: &mut App) {
    players::install(app);
    app.register_plugin(DefaultPlugins);
    players::enable_joining(app);
    app.set_startup_scene(asset_id!("content/level/test_level.gasset"));
    app.register_plugin(NightClockPlugin::default());
    world::install(app, world::default_saves());
}

/// The game as tests build it: the same, saving nowhere.
pub fn game_without_saves(app: &mut App) {
    players::install(app);
    app.register_plugin(DefaultPlugins);
    players::enable_joining(app);
    app.set_startup_scene(asset_id!("content/level/test_level.gasset"));
    app.register_plugin(NightClockPlugin::default());
    world::install(app, None);
}

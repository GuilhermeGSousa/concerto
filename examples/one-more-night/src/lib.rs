use concerto::{DefaultPlugins, app::App, asset_id};

pub mod players;

use crate::clock::NightClockPlugin;

/// The case schema and its loader.
pub use one_more_night_case as case;

pub mod clock;

/// Builds the game; the standalone binary and the editor both call it.
pub fn game(app: &mut App) {
    players::install(app);
    app.register_plugin(DefaultPlugins);
    players::enable_joining(app);
    app.set_startup_scene(asset_id!("content/room/room.gasset"));
    app.register_plugin(NightClockPlugin::default());
}

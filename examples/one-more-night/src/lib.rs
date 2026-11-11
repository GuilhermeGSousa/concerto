use concerto::{DefaultPlugins, app::App};

use crate::clock::NightClockPlugin;

/// The case schema and its loader.
pub use one_more_night_case as case;

pub mod clock;

/// Builds the game; the standalone binary and the editor both call it.
pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(NightClockPlugin::default());
}

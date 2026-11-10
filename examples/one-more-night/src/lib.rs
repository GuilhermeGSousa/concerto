use concerto::{DefaultPlugins, app::App};

use crate::clock::NightClockPlugin;

pub mod clock;

/// Builds the game; the standalone binary and the editor both call it.
pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(NightClockPlugin);
}

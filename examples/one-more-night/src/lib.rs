use concerto::{DefaultPlugins, app::App};

/// Builds the game; the standalone binary and the editor both call it.
pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins);
}

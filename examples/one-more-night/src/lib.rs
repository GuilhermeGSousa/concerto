use concerto::{DefaultPlugins, app::App};

/// The case schema and its loader.
pub use one_more_night_case as case;

/// Builds the game; the standalone binary and the editor both call it.
pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins);
}

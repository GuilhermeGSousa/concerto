use concerto_app::App;
use concerto_editor::clip_editor::preview::clip_preview;

use crate::{document::DatabaseSettings, playback::database_playback};

/// What a definition's preview adds to the game's recipe.
pub fn database_preview(app: &mut App) {
    clip_preview(app);
    database_playback(app);
    app.register_type::<DatabaseSettings>();
}

mod document;
mod preview;
mod save;
#[cfg(test)]
mod tests;

use concerto_app::{App, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;
use concerto_editor::asset_editor::AssetEditorAppExt;
use concerto_motion_matching::MotionDatabaseDefinition;

pub use document::{DefinitionDocument, MotionDatabaseEditor, read_definition};
pub use preview::{ChannelOrder, DatabaseSettings, database_preview};

use document::{follow_definition_edits, forget_discarded_edits, load_definition};
use save::{request_definition_saves, save_definition};

/// Adds the motion database editor to the editor's `App`.
pub fn install(app: &mut App) {
    app.register_creatable_asset_editor::<MotionDatabaseDefinition, MotionDatabaseEditor>()
        .expect("the editor's App holds the asset-editor registry")
        .add_system(
            Update,
            (
                follow_definition_edits,
                request_definition_saves,
                load_definition,
            )
                .chain(),
        )
        .add_listener(save_definition)
        .add_listener(forget_discarded_edits);
}

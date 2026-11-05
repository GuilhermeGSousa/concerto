mod derive;
mod document;
mod facts;
mod joint;
mod menu;
mod panels;
mod playback;
mod preview;
mod save;
#[cfg(test)]
mod tests;

use concerto_app::{
    App,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::IntoSystemConfig;
use concerto_editor::{asset_editor::AssetEditorAppExt, inspector::EditableApp};
use concerto_mesh::JointName;
use concerto_motion_matching::MotionDatabase;

pub use derive::{BakedStats, ChannelStats, EntryStats};
pub use document::{
    ChannelId, DatabaseSettings, DefinitionDocument, HeldChannel, MotionDatabaseEditor,
    read_definition,
};
pub use facts::{EntryFacts, read_skeleton};
pub use joint::JointNameEditor;
pub use playback::{BakedPlayback, DatabaseProgress, SeekDatabase, ShowDatabase};
pub use preview::database_preview;

use derive::{derive_definitions, follow_written_inputs, mark_background_work};
use document::{
    clear_status_on_edit, follow_definition_edits, load_definition, restore_discarded_edits,
};
use facts::{follow_skeleton, read_entry_facts};
use joint::refresh_joint_rows;
use menu::close_name_menu;
use panels::{
    build_definition_panels, sync_channel_rows, sync_definition_panels, sync_panel_scroll,
};
use playback::{follow_playback, follow_preview_time};
use save::save_definitions;

/// Adds the motion database editor to the editor's `App`.
pub fn install(app: &mut App) {
    app.register_creatable_asset_editor::<MotionDatabase, MotionDatabaseEditor>()
        .expect("the editor's App holds the asset-editor registry")
        .register_property_editor::<JointName, JointNameEditor>()
        .add_system(
            Update,
            (
                (
                    follow_definition_edits,
                    save_definitions,
                    load_definition,
                    follow_skeleton,
                    read_entry_facts,
                    follow_written_inputs,
                    derive_definitions,
                    follow_playback,
                    follow_preview_time,
                )
                    .chain(),
                (
                    mark_background_work,
                    build_definition_panels,
                    sync_definition_panels,
                    sync_channel_rows,
                    close_name_menu,
                )
                    .chain(),
            )
                .chain(),
        )
        .add_system(LateUpdate, (refresh_joint_rows, sync_panel_scroll))
        .add_listener(clear_status_on_edit)
        .add_listener(restore_discarded_edits);
}

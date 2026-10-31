mod build;
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
use concerto_motion_matching::MotionDatabaseDefinition;

pub use build::{BakedState, BakedStats, BuildState, ChannelStats, EntryStats};
pub use document::{DefinitionDocument, MotionDatabaseEditor, read_definition};
pub use facts::{EntryFacts, read_skeleton};
pub use joint::JointNameEditor;
pub use playback::{BakedPlayback, DatabaseProgress, SeekDatabase, ShowDatabase};
pub use preview::{ChannelKind, ChannelOrder, DatabaseSettings, SkeletonBones, database_preview};

use build::{check_baked, finish_build, mark_background_work, request_builds, start_build};
use document::{
    clear_status_on_edit, follow_definition_edits, forget_discarded_edits, load_definition,
};
use facts::{follow_skeleton, read_entry_facts};
use joint::refresh_joint_rows;
use menu::close_name_menu;
use panels::{
    build_definition_panels, sync_channel_rows, sync_definition_panels, sync_panel_scroll,
};
use playback::{follow_playback, follow_preview_time};
use save::{request_definition_saves, save_definition};

/// Adds the motion database editor to the editor's `App`.
pub fn install(app: &mut App) {
    app.register_creatable_asset_editor::<MotionDatabaseDefinition, MotionDatabaseEditor>()
        .expect("the editor's App holds the asset-editor registry")
        .register_property_editor::<JointName, JointNameEditor>()
        .add_system(
            Update,
            (
                (
                    follow_definition_edits,
                    request_definition_saves,
                    request_builds,
                    load_definition,
                    follow_skeleton,
                    read_entry_facts,
                    finish_build,
                    check_baked,
                    follow_playback,
                    follow_preview_time,
                )
                    .chain(),
                (
                    mark_background_work,
                    build_definition_panels,
                    sync_definition_panels,
                    sync_channel_rows,
                )
                    .chain(),
            )
                .chain(),
        )
        .add_system(LateUpdate, (refresh_joint_rows, sync_panel_scroll))
        .add_listener(save_definition)
        .add_listener(start_build)
        .add_listener(clear_status_on_edit)
        .add_listener(forget_discarded_edits)
        .add_listener(close_name_menu);
}

mod derive;
mod document;
mod save;

use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;
use concerto_editor::asset_editor::AssetEditorAppExt;
use concerto_render::assets::texture::Texture;

pub use document::{TextureDocument, TextureEditor, TextureTab};

use derive::{build_previews, derive_textures, mark_background_work, show_previews};
use document::{
    clear_status_on_edit, follow_texture_edits, load_textures, restore_discarded_edits,
};
use save::save_textures;

/// Opens textures in a tab that previews their derived levels and edits their settings.
pub struct TextureEditorPlugin;

impl Plugin for TextureEditorPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<TextureDocument>();
        app.register_asset_editor::<Texture, TextureEditor>()
            .expect("the editor's App holds the asset-editor registry")
            .add_system(
                Update,
                (
                    follow_texture_edits,
                    save_textures,
                    load_textures,
                    derive_textures,
                    mark_background_work,
                    build_previews,
                    show_previews,
                )
                    .chain(),
            )
            .add_listener(clear_status_on_edit)
            .add_listener(restore_discarded_edits);
    }
}

/// Adds the texture tab to the editor's `App`.
pub fn install(app: &mut App) {
    app.register_plugin(TextureEditorPlugin);
}

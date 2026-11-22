use std::path::Path;

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, With, command::CommandQueue, events::event_writer::EventWriter,
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, report_save},
    import::ImportQueue,
    project::ProjectState,
    scene::{SaveFailed, SceneSaved},
};
use concerto_foundation::assets::{
    content::resave_content_asset,
    registry::{AssetEntry, AssetRegistry},
};

use crate::document::{TextureTab, hold_document, read_texture_payload};

fn save_document(
    document: &mut EditorDocument,
    tab: &mut TextureTab,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<AssetEntry> {
    let asset = document.current.clone().context("No texture is open")?;
    let value = tab.value.context("No texture is open")?;
    if let Some(error) = &tab.derive_error {
        anyhow::bail!("{error}; change the settings until the texture derives, then save");
    }
    let mut texture = read_texture_payload(root, &asset)?;
    texture.settings = value.settings;
    let id = resave_content_asset(&texture, root, &asset.address, assets)?;
    let saved = assets
        .get(id)
        .cloned()
        .context("The saved texture was not registered")?;
    hold_document(tab, &texture);
    document.title = saved.name().into_owned();
    document.current = Some(saved.clone());
    Ok(saved)
}

pub(crate) fn save_textures(
    mut requested: Query<(Entity, &mut EditorDocument, &mut TextureTab), With<SaveRequested>>,
    project: Res<ProjectState>,
    imports: Option<Res<ImportQueue>>,
    mut assets: ResMut<AssetRegistry>,
    mut saved: EventWriter<SceneSaved>,
    mut failed: EventWriter<SaveFailed>,
    mut cmd: CommandQueue,
) {
    if imports
        .as_deref()
        .is_some_and(|queue| queue.is_running() || queue.remaining() > 0)
    {
        return;
    }
    let root = project
        .project
        .as_ref()
        .map(|project| project.root.as_path());
    for (entity, mut document, mut tab) in requested.iter_mut() {
        if tab.derive_wanted || tab.job.is_some() {
            continue;
        }
        let written = root
            .context("Open a project before saving")
            .and_then(|root| save_document(&mut document, &mut tab, root, &mut assets));
        report_save(
            entity,
            &mut document,
            written.map(Into::into),
            &mut saved,
            &mut failed,
            &mut cmd,
        );
    }
}

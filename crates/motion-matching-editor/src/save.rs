use std::path::Path;

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, With, command::CommandQueue, events::event_writer::EventWriter,
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, report_save},
    project::ProjectState,
    scene::{SaveFailed, SceneSaved},
};
use concerto_foundation::assets::{
    content::save_content_asset_into,
    registry::{AssetEntry, AssetRegistry},
};

use crate::document::DefinitionDocument;

pub(crate) fn save_document(
    document: &mut EditorDocument,
    state: &mut DefinitionDocument,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<AssetEntry> {
    let address = document
        .current
        .as_ref()
        .context("No definition is open")?
        .address
        .clone();
    let definition = state.definition()?;
    let id = save_content_asset_into(&definition, root, &address, assets)?;
    let saved = assets
        .get(id)
        .cloned()
        .context("The saved definition was not registered")?;
    document.title = saved.name().into_owned();
    document.current = Some(saved.clone());
    state.loaded = Some(definition);
    state.derive_wanted = true;
    Ok(saved)
}

pub(crate) fn save_definitions(
    mut requested: Query<
        (Entity, &mut EditorDocument, &mut DefinitionDocument),
        With<SaveRequested>,
    >,
    project: Res<ProjectState>,
    mut assets: ResMut<AssetRegistry>,
    mut saved: EventWriter<SceneSaved>,
    mut failed: EventWriter<SaveFailed>,
    mut cmd: CommandQueue,
) {
    let root = project
        .project
        .as_ref()
        .map(|project| project.root.as_path());
    for (entity, mut document, mut state) in requested.iter_mut() {
        let written = root
            .context("Open a project before saving")
            .and_then(|root| save_document(&mut document, &mut state, root, &mut assets));
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

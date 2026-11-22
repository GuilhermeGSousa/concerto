use std::path::Path;

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, With, command::CommandQueue, events::event_writer::EventWriter,
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, SavedFiles, report_save},
    project::ProjectState,
    scene::{SaveFailed, SceneSaved},
};
use concerto_foundation::assets::{content::save_content_asset_into, registry::AssetRegistry};

use super::document::GraphDocument;

fn save_document(
    document: &mut EditorDocument,
    state: &mut GraphDocument,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<SavedFiles> {
    let asset = document.current.clone().context("No graph is open")?;
    let mut written = SavedFiles {
        asset,
        written: Vec::new(),
    };
    for held in state.graphs_mut() {
        if !held.changed {
            continue;
        }
        let graph = held.copy.build()?.graph;
        let id = save_content_asset_into(&graph, root, &held.asset.address, assets)?;
        let saved = assets
            .get(id)
            .cloned()
            .context("The saved graph was not registered")?;
        held.changed = false;
        written.written.push(saved.clone());
        if id == written.asset.id {
            document.title = saved.name().into_owned();
            document.current = Some(saved.clone());
            written.asset = saved.clone();
        }
        held.asset = saved;
    }
    Ok(written)
}

pub(crate) fn save_graphs(
    mut requested: Query<(Entity, &mut EditorDocument, &mut GraphDocument), With<SaveRequested>>,
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
            written,
            &mut saved,
            &mut failed,
            &mut cmd,
        );
    }
}

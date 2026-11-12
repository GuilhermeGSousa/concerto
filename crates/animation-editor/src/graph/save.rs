use std::path::Path;

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, With, command::CommandQueue, events::event_writer::EventWriter,
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, Unsaved},
    project::ProjectState,
    scene::{SaveFailed, SceneSaved},
};
use concerto_foundation::assets::{
    content::save_content_asset_into,
    registry::{AssetEntry, AssetRegistry},
};

use super::document::GraphDocument;

fn save_document(
    document: &mut EditorDocument,
    state: &GraphDocument,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<AssetEntry> {
    let address = document
        .current
        .as_ref()
        .context("No graph is open")?
        .address
        .clone();
    let graph = state.graph.as_ref().context("No graph is open")?.build()?;
    let id = save_content_asset_into(&graph, root, &address, assets)?;
    let saved = assets
        .get(id)
        .cloned()
        .context("The saved graph was not registered")?;
    document.title = saved.name().to_owned();
    document.current = Some(saved.clone());
    Ok(saved)
}

pub(crate) fn save_graphs(
    requested: Query<(Entity, &mut EditorDocument, &GraphDocument), With<SaveRequested>>,
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
    for (entity, mut document, state) in requested.iter() {
        cmd.remove::<SaveRequested>(entity);
        let written = root
            .context("Open a project before saving")
            .and_then(|root| save_document(&mut document, state, root, &mut assets));
        match written {
            Ok(asset) => {
                document.status = format!("Saved {}", asset.address);
                saved.write(SceneSaved {
                    document: entity,
                    asset,
                });
                cmd.remove::<Unsaved>(entity);
            }
            Err(error) => {
                let message = format!("Save failed: {error:#}");
                document.status = message.clone();
                failed.write(SaveFailed {
                    document: entity,
                    message,
                });
            }
        }
    }
}

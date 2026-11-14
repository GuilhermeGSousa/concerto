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

struct Written {
    tab: AssetEntry,
    files: Vec<AssetEntry>,
}

fn save_document(
    document: &mut EditorDocument,
    state: &mut GraphDocument,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<Written> {
    let tab = document.current.clone().context("No graph is open")?;
    let mut written = Written {
        tab,
        files: Vec::new(),
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
        written.files.push(saved.clone());
        if id == written.tab.id {
            document.title = saved.name().to_owned();
            document.current = Some(saved.clone());
            written.tab = saved.clone();
        }
        held.asset = saved;
    }
    Ok(written)
}

pub(crate) fn save_graphs(
    requested: Query<(Entity, &mut EditorDocument, &mut GraphDocument), With<SaveRequested>>,
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
    for (entity, mut document, mut state) in requested.iter() {
        cmd.remove::<SaveRequested>(entity);
        let written = root
            .context("Open a project before saving")
            .and_then(|root| save_document(&mut document, &mut state, root, &mut assets));
        match written {
            Ok(written) => {
                let addresses: Vec<&str> = written
                    .files
                    .iter()
                    .map(|asset| asset.address.as_str())
                    .collect();
                document.status = format!("Saved {}", addresses.join(", "));
                saved.write(SceneSaved {
                    document: entity,
                    asset: written.tab,
                    written: written.files,
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

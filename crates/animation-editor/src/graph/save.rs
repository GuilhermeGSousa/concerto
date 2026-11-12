use std::path::Path;

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, With, command::CommandQueue, events::event_writer::EventWriter,
};
use concerto_editor::{
    asset_editor::{EditorDocument, SaveRequested, Unsaved},
    project::{AssetsWritten, ProjectState},
    scene::{SaveFailed, SceneSaved},
};
use concerto_foundation::assets::{
    AssetId,
    content::save_content_asset_into,
    registry::{AssetEntry, AssetRegistry},
};

use super::document::GraphDocument;

struct Written {
    tab: AssetEntry,
    addresses: Vec<String>,
    nested: Vec<AssetId>,
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
        addresses: Vec::new(),
        nested: Vec::new(),
    };
    for held in state.graphs_mut() {
        if !held.changed {
            continue;
        }
        let graph = held.copy.build()?;
        let id = save_content_asset_into(&graph, root, &held.asset.address, assets)?;
        let saved = assets
            .get(id)
            .cloned()
            .context("The saved graph was not registered")?;
        held.changed = false;
        written.addresses.push(saved.address.clone());
        if id == written.tab.id {
            document.title = saved.name().to_owned();
            document.current = Some(saved.clone());
            written.tab = saved.clone();
        } else {
            written.nested.push(id);
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
    mut nested: EventWriter<AssetsWritten>,
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
                document.status = format!("Saved {}", written.addresses.join(", "));
                if !written.nested.is_empty() {
                    nested.write(AssetsWritten {
                        ids: written.nested,
                    });
                }
                saved.write(SceneSaved {
                    document: entity,
                    asset: written.tab,
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

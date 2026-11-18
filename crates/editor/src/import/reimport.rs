use std::path::{Path, PathBuf};

use concerto_ecs::{
    Component, Query, Res, ResMut, command::CommandQueue, events::event_reader::EventReader,
    query::filter::With, reflect::AppTypeRegistry, signal::On, signal::Signal,
};
use concerto_foundation::assets::{AssetId, registry::AssetRegistry};
use concerto_import::importer_for;
use concerto_mcp::{PendingCall, ToolCalls, ToolError};
use concerto_reflect::Reflect;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    ImportFinished, ImportQueue, ImportStaging, RowState, StagedImport, extension_of, scan_content,
    stage_row, validate_rows,
};
use crate::{
    mcp::{busy, find_asset, imported_answer, patched, project_opening},
    project::{EditorCommands, ProjectState},
};

/// Opens the Import dialog with one row that imports again the import `asset` belongs to, staged from its import record.
pub struct StageReimport {
    pub asset: AssetId,
}
impl Signal for StageReimport {}

/// Queues a re-import of the import `asset` belongs to, from `source` when given, which becomes the import's source; it ends with an `ImportFinished` for the recorded source.
pub struct ReimportAsset {
    pub asset: AssetId,
    pub source: Option<PathBuf>,
    /// The importer's settings, written whole into the import record; `None` keeps the record's.
    pub settings: Option<Box<dyn Reflect>>,
    /// The settings of the source's main asset; `None` keeps the asset's own.
    pub asset_settings: Option<Box<dyn Reflect>>,
}
impl Signal for ReimportAsset {}

/// Imports again the source of the import an asset belongs to, as the Asset Browser's Re-import… does, keeping every asset's id and address, and answers the assets it wrote.
#[derive(Deserialize, JsonSchema)]
pub struct ReimportArgs {
    /// Any asset of the import: an address, such as content/hero/scene.gasset or content/hero/scene.gasset#mesh/0, or an id, as list_assets shows them.
    pub asset: String,
    /// The file to import instead of the recorded source, which becomes the import's source; a relative path is read from the editor's working directory, and a file outside the project is copied beside the recorded source.
    pub source: Option<String>,
    /// Changes to the importer's settings, as an object of the fields to change, such as {"scale": 0.5} for a .bvh; the others keep the import record's values.
    pub settings: Option<serde_json::Value>,
    /// Changes to the settings of the asset an image source makes, such as {"compression": "Bc7"}; the others keep the asset's current values.
    pub asset_settings: Option<serde_json::Value>,
}

#[derive(Component)]
pub(super) struct Reimporting;

fn reimport_row(
    registry: &AssetRegistry,
    project_root: &Path,
    asset: AssetId,
    source: Option<PathBuf>,
) -> Option<StagedImport> {
    let recorded = registry.get(asset)?.provenance.as_ref()?.source.clone();
    Some(StagedImport {
        source: source.unwrap_or_else(|| project_root.join(&recorded)),
        destination: recorded,
        state: RowState::New,
        staged: None,
        settings: None,
        asset_settings: None,
        reimport: Some(asset),
    })
}

pub(super) fn stage_reimport(
    on: On<StageReimport>,
    registry: Res<AssetRegistry>,
    project: Res<ProjectState>,
    mut staging: ResMut<ImportStaging>,
) {
    let Some(root) = project.project.as_ref().map(|project| &project.root) else {
        return;
    };
    if staging.visible || project.busy() {
        return;
    }
    let Some(row) = reimport_row(&registry, root, on.signal().asset, None) else {
        return;
    };
    let scan = scan_content(root);
    let mut rows = vec![row];
    stage_row(&mut rows[0], &scan);
    validate_rows(&mut rows, root);
    staging.rows = rows;
    staging.visible = true;
    staging.scan = Some(scan);
}

pub(super) fn queue_reimport(
    on: On<ReimportAsset>,
    registry: Res<AssetRegistry>,
    mut project: ResMut<ProjectState>,
    mut queue: ResMut<ImportQueue>,
) {
    let request = on.signal();
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    let Some(mut row) = reimport_row(&registry, &root, request.asset, request.source.clone())
    else {
        project.status = format!(
            "{} was not imported; it has no source to import again.",
            request.asset.simple_hex()
        );
        return;
    };
    stage_row(&mut row, &scan_content(&root));
    if let Some(importer) = importer_for(&extension_of(Path::new(&row.destination))) {
        row.settings = request
            .settings
            .as_deref()
            .map(|settings| importer.clone_settings(settings));
        row.asset_settings = request
            .asset_settings
            .as_deref()
            .map(|settings| importer.clone_settings(settings));
    }
    queue.enqueue(vec![row]);
}

pub(super) fn call_reimport(
    mut calls: ToolCalls<ReimportArgs>,
    state: Res<ProjectState>,
    commands: Res<EditorCommands>,
    registry: Res<AssetRegistry>,
    types: Res<AppTypeRegistry>,
    mut queue: ResMut<ImportQueue>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if project_opening(&state, &commands) {
            reply.err(busy());
            continue;
        }
        let Some(root) = state.project.as_ref().map(|project| project.root.clone()) else {
            continue;
        };
        let entry = match find_asset(&registry, &arguments.asset) {
            Ok(entry) => entry,
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        let Some(provenance) = &entry.provenance else {
            reply.err(ToolError::new(
                "not_imported",
                format!(
                    "'{}' was not imported; it has no source to import again. Re-import an asset list_assets shows as an import or a part of one.",
                    entry.location()
                ),
            ));
            continue;
        };
        let source = arguments.source.as_ref().map(PathBuf::from);
        match &source {
            Some(source) if !source.is_file() => {
                reply.err(ToolError::new(
                    "invalid_arguments",
                    format!(
                        "No file at {}; give a path to the source file, absolute or from the editor's working directory.",
                        source.display()
                    ),
                ));
                continue;
            }
            None if !root.join(&provenance.source).is_file() => {
                reply.err(ToolError::new(
                    "source_missing",
                    format!(
                        "'{}', the source of '{}', is missing; pass source with the path of the file to import instead.",
                        provenance.source,
                        entry.location()
                    ),
                ));
                continue;
            }
            _ => {}
        }
        let Some(mut row) = reimport_row(&registry, &root, entry.id, source) else {
            continue;
        };
        stage_row(&mut row, &scan_content(&root));
        let staged = match &row.staged {
            Some(Ok(staged)) => staged,
            Some(Err(reason)) => {
                reply.err(ToolError::new(
                    "import_failed",
                    format!("{} was not re-imported: {reason}", row.destination),
                ));
                continue;
            }
            None => continue,
        };
        let Some(importer) = importer_for(&extension_of(Path::new(&row.destination))) else {
            continue;
        };
        let types = types.read();
        let settings = patched(
            "settings",
            arguments.settings.as_ref(),
            Some(importer.clone_settings(staged.settings.as_ref())),
            &types,
        );
        let asset_settings = patched(
            "asset_settings",
            arguments.asset_settings.as_ref(),
            staged
                .asset_settings
                .as_deref()
                .map(|value| importer.clone_settings(value)),
            &types,
        );
        match (settings, asset_settings) {
            (Ok(settings), Ok(asset_settings)) => {
                row.settings = settings;
                row.asset_settings = asset_settings;
            }
            (Err(reason), _) | (_, Err(reason)) => {
                reply.err(ToolError::new(
                    "invalid_arguments",
                    format!("{}: {reason}", arguments.asset),
                ));
                continue;
            }
        }
        let waiter = cmd
            .spawn((
                PendingCall::new(reply, "the re-import to finish"),
                Reimporting,
            ))
            .entity();
        queue.enqueue_for(row, waiter);
    }
}

pub(super) fn answer_reimports(
    mut finished: EventReader<ImportFinished>,
    waiting: Query<&PendingCall, With<Reimporting>>,
    mut cmd: CommandQueue,
) {
    for event in finished.read() {
        let Some((entity, pending)) = event
            .waiter
            .and_then(|waiter| Some((waiter, waiting.get_entity(waiter)?)))
        else {
            continue;
        };
        match &event.result {
            Ok(imported) => {
                let source = imported
                    .assets
                    .iter()
                    .find_map(|asset| asset.provenance.as_ref())
                    .map_or(event.destination.as_str(), |provenance| {
                        provenance.source.as_str()
                    });
                pending.reply.ok(imported_answer(
                    &format!("Re-imported {source}"),
                    imported,
                    &event.settings,
                ));
            }
            Err(reason) => pending.reply.err(ToolError::new(
                "import_failed",
                format!("{} was not re-imported: {reason}", event.destination),
            )),
        }
        cmd.despawn(entity);
    }
}

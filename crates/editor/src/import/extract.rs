use std::path::Path;

use concerto_ecs::{
    Component, Entity, Event, Query, Res, ResMut,
    command::CommandQueue,
    events::{event_reader::EventReader, event_writer::EventWriter},
    signal::{On, Signal},
};
use concerto_foundation::assets::{
    AssetId,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_import::config::ContentConfig;
use concerto_mcp::{PendingCall, ToolCalls, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;

use super::ImportQueue;
use crate::{
    mcp::{busy, find_asset, project_opening},
    project::{AssetsWritten, EditorCommands, ProjectState},
};

/// Moves the embedded asset `asset` out of its import's scene file into a file of its own beside it, as the Asset Browser's Extract to file does; it ends with an `AssetExtracted` or an `ExtractionFailed`.
pub struct ExtractAsset {
    pub asset: AssetId,
}
impl Signal for ExtractAsset {}

/// An embedded asset became the loose file `entry`, and the file of `host`, which held it, was rewritten without it.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct AssetExtracted {
    pub entry: AssetEntry,
    pub host: AssetId,
}

/// An `ExtractAsset` of `asset` wrote nothing, for the reason `message` gives.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ExtractionFailed {
    pub asset: AssetId,
    pub message: String,
}

/// Moves an asset embedded in its import's scene file into a file of its own at its usual address, keeping its id, as the Asset Browser's Extract to file does; a re-import then keeps it there, and nothing embeds it again.
#[derive(Deserialize, JsonSchema)]
pub struct ExtractAssetArgs {
    /// The embedded asset: an address with a part, such as content/hero/scene.gasset#material/0, or an id, as list_assets shows them.
    pub asset: String,
}

#[derive(Component)]
pub(super) struct Extracting(AssetId);

fn extract(
    asset: AssetId,
    project_root: &Path,
    registry: &mut AssetRegistry,
    queue: &ImportQueue,
) -> Result<AssetExtracted, String> {
    let Some(host) = registry
        .get(asset)
        .and_then(|entry| entry.embedded_in.as_ref())
        .map(|held| held.host)
    else {
        return Err(format!(
            "{} is not embedded in a scene file",
            asset.simple_hex()
        ));
    };
    if queue.is_running() || queue.remaining() > 0 {
        return Err("an import is running; extract once it has finished".into());
    }
    ContentConfig::load_or_default(project_root)
        .and_then(|config| {
            concerto_import::extract_embedded(asset, project_root, &config, registry)
        })
        .map(|entry| AssetExtracted { entry, host })
        .map_err(|error| format!("{error:#}"))
}

pub(super) fn extract_asset(
    on: On<ExtractAsset>,
    mut project: ResMut<ProjectState>,
    queue: Res<ImportQueue>,
    mut registry: ResMut<AssetRegistry>,
    mut extracted: EventWriter<AssetExtracted>,
    mut failed: EventWriter<ExtractionFailed>,
    mut written: EventWriter<AssetsWritten>,
) {
    let asset = on.signal().asset;
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    match extract(asset, &root, &mut registry, &queue) {
        Ok(done) => {
            project.status = format!("Extracted {} to {}", done.entry.name(), done.entry.address);
            written.write(AssetsWritten {
                ids: vec![done.entry.id, done.host],
            });
            extracted.write(done);
        }
        Err(message) => {
            project.status = format!("Not extracted: {message}");
            log::warn!("extracting {} failed: {message}", asset.simple_hex());
            failed.write(ExtractionFailed { asset, message });
        }
    }
}

pub(super) fn call_extract_asset(
    mut calls: ToolCalls<ExtractAssetArgs>,
    state: Res<ProjectState>,
    commands: Res<EditorCommands>,
    registry: Res<AssetRegistry>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if project_opening(&state, &commands) {
            reply.err(busy());
            continue;
        }
        if state.project.is_none() {
            continue;
        }
        let entry = match find_asset(&registry, &arguments.asset) {
            Ok(entry) => entry,
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        if entry.embedded_in.is_none() {
            reply.err(ToolError::new(
                "not_embedded",
                format!(
                    "'{}' is a file of its own already; extract an asset list_assets shows with a # in its address.",
                    entry.location()
                ),
            ));
            continue;
        }
        cmd.trigger(ExtractAsset { asset: entry.id });
        cmd.spawn((
            PendingCall::new(reply, "the asset to be extracted"),
            Extracting(entry.id),
        ));
    }
}

pub(super) fn answer_extractions(
    mut extracted: EventReader<AssetExtracted>,
    mut failed: EventReader<ExtractionFailed>,
    waiting: Query<(Entity, &PendingCall, &Extracting)>,
    mut cmd: CommandQueue,
) {
    let waiting_for = |asset: AssetId| {
        waiting
            .iter()
            .find(|(_, _, extracting)| extracting.0 == asset)
    };
    for done in extracted.read() {
        if let Some((entity, pending, _)) = waiting_for(done.entry.id) {
            pending.reply.ok(format!(
                "Extracted {} to {}.",
                done.entry.id.simple_hex(),
                done.entry.address
            ));
            cmd.despawn(entity);
        }
    }
    for failure in failed.read() {
        if let Some((entity, pending, _)) = waiting_for(failure.asset) {
            pending.reply.err(ToolError::new(
                "extract_failed",
                format!(
                    "{} was not extracted: {}",
                    failure.asset.simple_hex(),
                    failure.message
                ),
            ));
            cmd.despawn(entity);
        }
    }
}

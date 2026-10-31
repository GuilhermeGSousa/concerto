use std::path::Path;

use anyhow::Context;
use concerto_ecs::{
    Component, Entity, Query, Res, With,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    reflect::AppTypeRegistry,
    signal::On,
};
use concerto_editor::{
    asset_editor::{AssetEditor, EditorDocument, PreviewSpec, Unsaved, finish_asset_request},
    guard::DiscardEdits,
    hosted::{Hosted, HostedApp},
    preview::{PreviewFailed, PreviewSignals},
    project::ProjectState,
    tools::edits::{ComponentEdited, PropertyCommitted},
};
use concerto_foundation::assets::{
    Asset, AssetId, content::read_content_asset, handle::AssetHandle, registry::AssetEntry,
};
use concerto_mesh::Skeleton;
use concerto_motion_matching::{MotionDatabaseDefinition, MotionEntry};

use crate::{
    facts::{EntryFacts, FactsJob, ShownSkeleton},
    preview::{DatabaseSettings, ShowDefinition, database_preview, live_channel},
};

/// What a definition's tab holds; its other settings and its channels live in the preview's world.
#[derive(Component, Default)]
pub struct DefinitionDocument {
    pub skeleton: AssetHandle<Skeleton>,
    pub entries: Vec<MotionEntry>,
    pub selected_entry: Option<usize>,
    pub loaded: Option<MotionDatabaseDefinition>,
    pub entry_facts: Vec<EntryFacts>,
    pub(crate) shown_skeleton: Option<ShownSkeleton>,
    pub(crate) facts_job: Option<FactsJob>,
}

impl DefinitionDocument {
    /// What was read of `clip` against the definition's skeleton, once it has been.
    pub fn facts_of(&self, clip: AssetId) -> Option<&EntryFacts> {
        let skeleton = self.skeleton.id();
        self.entry_facts
            .iter()
            .find(|facts| facts.clip == clip && facts.skeleton == skeleton)
    }
}

/// Opens a `MotionDatabaseDefinition` in a tab whose preview `App` holds what the inspector edits.
#[derive(Default)]
pub struct MotionDatabaseEditor;

impl AssetEditor for MotionDatabaseEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        editor.insert(DefinitionDocument::default());
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: database_preview,
        })
    }

    fn bottom_host(&self) -> bool {
        true
    }

    fn edits_in_preview(&self) -> bool {
        true
    }
}

/// Reads the definition `asset` names under `root`, checking its id and its kind.
pub fn read_definition(
    root: &Path,
    asset: &AssetEntry,
) -> anyhow::Result<MotionDatabaseDefinition> {
    let bytes = std::fs::read(root.join(&asset.address))?;
    let (header, payload) = read_content_asset(&bytes)?;
    anyhow::ensure!(
        header.asset_id == asset.id && header.kind == MotionDatabaseDefinition::name(),
        "Asset identity or type mismatch"
    );
    Ok(bincode::deserialize(payload)?)
}

fn check_channels(definition: &MotionDatabaseDefinition, hosted: &HostedApp) -> anyhow::Result<()> {
    let registry = hosted
        .world()
        .get_resource::<AppTypeRegistry>()
        .context("The preview has no type registry")?
        .read();
    for channel in &definition.channels {
        live_channel(channel, &registry).map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

pub(crate) fn load_definition(
    documents: Query<(
        Entity,
        &mut EditorDocument,
        &mut DefinitionDocument,
        Option<&HostedApp>,
        Option<&PreviewFailed>,
    )>,
    project: Res<ProjectState>,
    mut previews: PreviewSignals,
) {
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    for (entity, mut document, mut state, hosted, failed) in documents.iter() {
        if hosted.is_none() && failed.is_none() {
            continue;
        }
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        let definition = read_definition(&root, &asset).and_then(|definition| {
            if let Some(hosted) = hosted {
                check_channels(&definition, hosted)?;
            }
            Ok(definition)
        });
        let definition = match definition {
            Ok(definition) => definition,
            Err(error) => {
                finish_asset_request(&mut document, &asset, Err(error));
                continue;
            }
        };
        state.skeleton = definition.skeleton.clone();
        state.entries = definition.entries.clone();
        state.selected_entry = (!definition.entries.is_empty()).then_some(0);
        state.entry_facts.clear();
        state.shown_skeleton = None;
        previews.trigger(
            entity,
            ShowDefinition {
                settings: DatabaseSettings::of(&definition),
                channels: definition.channels.clone(),
            },
        );
        state.loaded = Some(definition);
        finish_asset_request(&mut document, &asset, Ok(()));
    }
}

pub(crate) fn follow_definition_edits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    mut edited: EventReader<Hosted<ComponentEdited>>,
    documents: Query<Entity, With<DefinitionDocument>>,
    mut cmd: CommandQueue,
) {
    let committed = committed
        .read()
        .filter(|hosted| hosted.event.result.is_ok())
        .map(|hosted| hosted.document);
    let edited = edited
        .read()
        .filter(|hosted| hosted.event.result.is_ok())
        .map(|hosted| hosted.document);
    for document in committed.chain(edited) {
        if documents.get_entity(document).is_some() {
            cmd.insert(Unsaved, document);
        }
    }
}

pub(crate) fn forget_discarded_edits(
    on: On<DiscardEdits>,
    documents: Query<&mut HostedApp, With<DefinitionDocument>>,
) {
    if let Some(mut hosted) = documents.get_entity(on.signal().0) {
        hosted.mark_saved();
    }
}

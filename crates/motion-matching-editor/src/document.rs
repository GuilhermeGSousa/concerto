use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Context;
use concerto_ecs::{
    Component, Entity, Query, Res, With,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    signal::{On, lifecycle::Add},
};
use concerto_editor::{
    asset_editor::{AssetEditor, EditorDocument, PreviewSpec, Unsaved, finish_asset_request},
    guard::DiscardEdits,
    hosted::{Hosted, HostedApp},
    inspector::{DocumentValue, InspectedValues, PropertyTarget, RefreshInspector, ShownValue},
    joint_picker::DocumentBones,
    preview::{PreviewFailed, PreviewSignals},
    project::ProjectState,
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{decode_content_asset, read_content_asset},
    handle::AssetHandle,
    registry::AssetEntry,
};
use concerto_mesh::Skeleton;
use concerto_motion_matching::{
    BakeError, FeatureChannel, FeatureChannelData, MirrorAxis, MotionDatabase, MotionEntry,
    SimulationBoneSettings, TrajectorySettings,
};
use concerto_reflect::{Reflect, ReflectedValue, TypeRegistry};
use uuid::Uuid;

use crate::{
    derive::{BakedStats, DeriveJob},
    facts::{EntryFacts, FactsJob, ShownSkeleton},
    playback::{BakedPlayback, ClipJob, ShowDatabase},
    preview::database_preview,
};

const SETTINGS: &str = "Settings";

/// A definition's settings that belong to no channel, as the inspector edits them.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct DatabaseSettings {
    pub rate: f32,
    pub speed: f32,
    pub simulation: SimulationBoneSettings,
    pub trajectory: TrajectorySettings,
    pub mirror_axis: MirrorAxis,
}

impl DatabaseSettings {
    /// The settings `definition` holds.
    pub fn of(definition: &MotionDatabase) -> Self {
        Self {
            rate: definition.rate,
            speed: definition.speed,
            simulation: definition.simulation.clone(),
            trajectory: definition.trajectory,
            mirror_axis: definition.mirror_axis,
        }
    }
}

/// Names one held channel for as long as the document holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelId(Uuid);

/// A live channel of a definition, under the type name a save writes back.
pub struct HeldChannel {
    pub id: ChannelId,
    pub type_name: String,
    pub value: Box<dyn FeatureChannel>,
}

impl HeldChannel {
    pub(crate) fn of(data: &FeatureChannelData, registry: &TypeRegistry) -> Result<Self, String> {
        let name = data.type_name();
        let mut loaded = data.clone();
        loaded.load(registry).map_err(|error| match error {
            BakeError::UnregisteredChannel(_) if registry.get_by_name(name).is_some() => {
                format!("{name} is not a feature channel")
            }
            BakeError::UnregisteredChannel(_) => {
                format!("Channel {name} is not registered in the game")
            }
            BakeError::ChannelValue { reason, .. } => {
                format!("Channel {name} cannot be built: {reason}")
            }
            error => error.to_string(),
        })?;
        let value = loaded.channel().map_err(|error| error.to_string())?;
        Ok(Self {
            id: ChannelId(Uuid::new_v4()),
            type_name: name.to_owned(),
            value: value.dyn_clone(),
        })
    }

    pub(crate) fn empty(name: &str, registry: &TypeRegistry) -> Result<Self, String> {
        let stored = FeatureChannelData::Stored {
            type_name: name.to_owned(),
            value: ReflectedValue::Struct(Vec::new()),
        };
        Self::of(&stored, registry)
    }

    fn data(&self) -> FeatureChannelData {
        FeatureChannelData::Live {
            type_name: self.type_name.clone(),
            channel: self.value.dyn_clone(),
        }
    }
}

/// Everything a definition's tab holds, its settings and channels included.
#[derive(Component, Default)]
pub struct DefinitionDocument {
    pub skeleton: AssetHandle<Skeleton>,
    pub entries: Vec<MotionEntry>,
    pub selected_entry: Option<usize>,
    pub settings: Option<DatabaseSettings>,
    pub channels: Vec<HeldChannel>,
    pub loaded: Option<MotionDatabase>,
    pub stats: Option<BakedStats>,
    pub entry_facts: Vec<EntryFacts>,
    pub time: f32,
    pub(crate) shown_skeleton: Option<ShownSkeleton>,
    pub(crate) facts_job: Option<FactsJob>,
    pub(crate) derive_wanted: bool,
    pub(crate) derive_job: Option<DeriveJob>,
    pub(crate) playback: Option<BakedPlayback>,
    pub(crate) sought_entry: Option<usize>,
    pub(crate) picked_channel: Option<usize>,
    pub(crate) shown_clip: Option<AssetId>,
    pub(crate) clip_job: Option<ClipJob>,
}

impl DefinitionDocument {
    /// What was read of `clip` against the definition's skeleton, once it has been.
    pub fn facts_of(&self, clip: AssetId) -> Option<&EntryFacts> {
        let skeleton = self.skeleton.id();
        self.entry_facts
            .iter()
            .find(|facts| facts.clip == clip && facts.skeleton == skeleton)
    }

    pub(crate) fn definition(&self) -> anyhow::Result<MotionDatabase> {
        let settings = self
            .settings
            .clone()
            .context("The preview is not running, so the channels cannot be written")?;
        Ok(MotionDatabase {
            skeleton: self.skeleton.clone(),
            entries: self.entries.clone(),
            rate: settings.rate,
            speed: settings.speed,
            simulation: settings.simulation,
            trajectory: settings.trajectory,
            channels: self.channels.iter().map(HeldChannel::data).collect(),
            mirror_axis: settings.mirror_axis,
            baked: None,
        })
    }

    fn channel(&self, id: ChannelId) -> Option<&HeldChannel> {
        self.channels.iter().find(|channel| channel.id == id)
    }

    fn channel_mut(&mut self, id: ChannelId) -> Option<&mut HeldChannel> {
        self.channels.iter_mut().find(|channel| channel.id == id)
    }
}

/// Opens a `MotionDatabase` in a tab that holds it, with a preview that plays it.
#[derive(Default)]
pub struct MotionDatabaseEditor;

impl AssetEditor for MotionDatabaseEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        editor.insert((
            DefinitionDocument::default(),
            InspectedValues::default(),
            DocumentBones(None),
        ));
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: database_preview,
        })
    }

    fn bottom_host(&self) -> bool {
        true
    }
}

fn settings_value(document: Entity) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<DefinitionDocument> {
        name: SETTINGS.to_owned(),
        document,
        read: Box::new(|state| state.settings.as_ref().map(|settings| settings as _)),
        edit: Box::new(|state| state.settings.as_mut().map(|settings| settings as _)),
    })
}

fn channel_value(document: Entity, id: ChannelId, name: String) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<DefinitionDocument> {
        name,
        document,
        read: Box::new(move |state| {
            state
                .channel(id)
                .map(|channel| channel.value.as_ref() as &dyn Reflect)
        }),
        edit: Box::new(move |state| {
            state
                .channel_mut(id)
                .map(|channel| channel.value.as_mut() as &mut dyn Reflect)
        }),
    })
}

fn channel_names(channels: &[HeldChannel], registry: &TypeRegistry) -> Vec<String> {
    let mut seen = HashMap::from([(SETTINGS, 1)]);
    channels
        .iter()
        .map(|channel| {
            let name = registry
                .get_by_name(&channel.type_name)
                .map_or(channel.type_name.as_str(), |registration| {
                    registration.short_name()
                });
            let count = seen.entry(name).or_insert(0);
            *count += 1;
            match *count {
                1 => name.to_owned(),
                count => format!("{name} {count}"),
            }
        })
        .collect()
}

pub(crate) fn publish_values(
    document: Entity,
    state: &DefinitionDocument,
    registry: &TypeRegistry,
    shown: Option<ChannelId>,
    cmd: &mut CommandQueue,
) {
    let names = channel_names(&state.channels, registry);
    let channels = state.channels.iter().zip(names);
    let values: Vec<_> = std::iter::once(settings_value(document))
        .chain(channels.map(|(channel, name)| channel_value(document, channel.id, name)))
        .collect();
    let index = state
        .channels
        .iter()
        .position(|channel| Some(channel.id) == shown)
        .map_or(0, |index| index + 1);
    let shown = ShownValue(values[index].clone());
    cmd.insert((InspectedValues(values), shown), document);
    cmd.trigger(RefreshInspector);
}

/// Reads the definition `asset` names under `root`, checking its id and its kind.
pub fn read_definition(root: &Path, asset: &AssetEntry) -> anyhow::Result<MotionDatabase> {
    let bytes = std::fs::read(root.join(&asset.address))?;
    let (header, _) = read_content_asset(&bytes)?;
    anyhow::ensure!(
        header.asset_id == asset.id && header.kind == MotionDatabase::name(),
        "Asset identity or type mismatch"
    );
    Ok(decode_content_asset::<MotionDatabase>(&bytes)?.1)
}

fn hold_definition(
    state: &mut DefinitionDocument,
    mut definition: MotionDatabase,
    channels: Option<Vec<HeldChannel>>,
) {
    if let Some(channels) = &channels {
        definition.channels = channels.iter().map(HeldChannel::data).collect();
    }
    state.settings = channels
        .is_some()
        .then(|| DatabaseSettings::of(&definition));
    state.channels = channels.unwrap_or_default();
    state.skeleton = definition.skeleton.clone();
    state.entries = definition.entries.clone();
    state.selected_entry = (!definition.entries.is_empty()).then_some(0);
    state.entry_facts.clear();
    state.shown_skeleton = None;
    state.time = 0.0;
    state.sought_entry = None;
    state.loaded = Some(definition);
}

fn live_channels(
    definition: &MotionDatabase,
    registry: &TypeRegistry,
) -> anyhow::Result<Vec<HeldChannel>> {
    definition
        .channels
        .iter()
        .map(|channel| HeldChannel::of(channel, registry).map_err(anyhow::Error::msg))
        .collect()
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
    mut cmd: CommandQueue,
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
        let types = hosted.and_then(HostedApp::type_registry);
        let read = read_definition(&root, &asset).and_then(|definition| {
            let channels = types
                .as_ref()
                .map(|types| live_channels(&definition, &types.read()))
                .transpose()?;
            Ok((definition, channels))
        });
        let (definition, channels) = match read {
            Ok(read) => read,
            Err(error) => {
                finish_asset_request(&mut document, &asset, Err(error));
                continue;
            }
        };
        hold_definition(&mut state, definition, channels);
        state.stats = None;
        state.playback = None;
        state.derive_job = None;
        state.derive_wanted = true;
        previews.trigger(entity, ShowDatabase(None));
        if let Some(types) = types {
            publish_values(entity, &state, &types.read(), None, &mut cmd);
        }
        finish_asset_request(&mut document, &asset, Ok(()));
    }
}

pub(crate) fn restore_discarded_edits(
    on: On<DiscardEdits>,
    documents: Query<(&mut DefinitionDocument, Option<&HostedApp>)>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let Some((mut state, hosted)) = documents.get_entity(document) else {
        return;
    };
    let Some(definition) = state.loaded.clone() else {
        return;
    };
    let types = hosted.and_then(HostedApp::type_registry);
    let channels = types
        .as_ref()
        .map(|types| live_channels(&definition, &types.read()))
        .transpose();
    let channels = match channels {
        Ok(channels) => channels,
        Err(error) => {
            log::warn!("Discarded edits stay: {error}");
            cmd.insert(Unsaved, document);
            return;
        }
    };
    hold_definition(&mut state, definition, channels);
    state.derive_wanted = true;
    if let Some(types) = types {
        publish_values(document, &state, &types.read(), None, &mut cmd);
    }
}

pub(crate) fn follow_definition_edits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    documents: Query<Entity, With<DefinitionDocument>>,
    mut cmd: CommandQueue,
) {
    for hosted in committed.read() {
        if hosted.event.result.is_ok() && documents.get_entity(hosted.document).is_some() {
            cmd.insert(Unsaved, hosted.document);
        }
    }
}

pub(crate) fn clear_status_on_edit(
    on: On<Add<Unsaved>>,
    documents: Query<&mut EditorDocument, With<DefinitionDocument>>,
) {
    if let Some(mut document) = documents.get_entity(on.signal().entity)
        && !document.status.is_empty()
    {
        document.status.clear();
    }
}

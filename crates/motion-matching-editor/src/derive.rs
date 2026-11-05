use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use anyhow::anyhow;
use concerto_animation::clip::AnimationClip;
use concerto_ecs::{Entity, Query, Res, command::CommandQueue, events::event_reader::EventReader};
use concerto_editor::{
    asset_editor::{BackgroundWork, EditorDocument},
    preview::PreviewSignals,
    project::{AssetsWritten, ProjectState},
};
use concerto_foundation::assets::{
    Asset, AssetId, ContentAssetRoot,
    asset_server::AssetServer,
    derived::{DerivedData, DerivedDataCache, DerivedInputs},
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::Skeleton;
use concerto_motion_matching::{FeatureChannel, MotionDatabase};

use crate::{
    document::DefinitionDocument,
    playback::{BakedPlayback, ShowDatabase},
};

/// How many frames a baked entry holds, and whether it is shorter than the trajectory looks ahead.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryStats {
    pub clip: AssetId,
    pub frames: usize,
    pub short: bool,
}

/// One channel's mean over its dimensions and its group's standard deviation.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelStats {
    pub name: String,
    pub mean: f32,
    pub deviation: f32,
}

/// What the panel shows of a baked database.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedStats {
    pub entries: Vec<EntryStats>,
    pub frames: usize,
    pub bytes: usize,
    pub horizon: f32,
    pub channels: Vec<ChannelStats>,
}

impl BakedStats {
    /// The stats of `database`.
    pub fn of(database: &MotionDatabase) -> Self {
        let trajectory = &database.trajectory;
        let horizon = trajectory.sample_dt * trajectory.sample_count as f32;
        let entries = database
            .clips()
            .enumerate()
            .map(|(index, clip)| {
                let frames = database.frames_of_entry(index).unwrap_or(0);
                EntryStats {
                    clip: clip.id(),
                    frames,
                    short: (frames as f32) < horizon * database.rate,
                }
            })
            .collect();
        let channels = database
            .channels
            .iter()
            .enumerate()
            .zip(database.deviations().unwrap_or_default())
            .filter_map(|((index, channel), deviation)| {
                let range = database.channel_range(index)?;
                let means = database.offset()?.get(range.clone())?;
                Some(ChannelStats {
                    name: channel.type_name().to_owned(),
                    mean: means.iter().sum::<f32>() / range.len().max(1) as f32,
                    deviation: *deviation,
                })
            })
            .collect();
        Self {
            entries,
            frames: database.frame_count().unwrap_or(0),
            bytes: database.size_in_bytes().unwrap_or(0),
            horizon,
            channels,
        }
    }
}

pub(crate) type DeriveProgress = Arc<Mutex<Option<(usize, usize)>>>;

pub(crate) struct Derived {
    playback: BakedPlayback,
    stats: BakedStats,
}

pub(crate) struct DeriveJob {
    pub(crate) handle: JoinHandle<anyhow::Result<Derived>>,
    pub(crate) asset: AssetEntry,
    pub(crate) progress: DeriveProgress,
    pub(crate) line: String,
}

impl DeriveJob {
    fn progress(&self) -> Option<(usize, usize)> {
        *self.progress.lock().unwrap()
    }
}

pub(crate) fn progress_line(finished: usize, total: usize) -> String {
    format!("Deriving {} of {total}", (finished + 1).min(total))
}

struct DeriveInput {
    definition: MotionDatabase,
    root: PathBuf,
    assets: AssetRegistry,
    cache: DerivedDataCache,
    progress: DeriveProgress,
}

fn derived_database(input: &DeriveInput, inputs: &DerivedInputs) -> anyhow::Result<MotionDatabase> {
    let (name, version) = (MotionDatabase::name(), MotionDatabase::DERIVED_VERSION);
    let mut database = input.definition.clone();
    let key = database.derived_key(inputs)?;
    if let Some(blob) = input.cache.get(name, version, key) {
        match database.apply_derived(&blob) {
            Ok(()) => return Ok(database),
            Err(error) => log::warn!("discarding the cached {name} blob {key:016x}: {error:#}"),
        }
    }
    *input.progress.lock().unwrap() = Some((0, database.entries.len()));
    let blob = database.derive(inputs, &mut |finished, total| {
        *input.progress.lock().unwrap() = Some((finished, total));
    })?;
    input.cache.put(name, version, key, &blob);
    database.apply_derived(&blob)?;
    Ok(database)
}

fn run_derive(input: DeriveInput) -> anyhow::Result<Derived> {
    let root = ContentAssetRoot::Directory(input.root.clone());
    let ids = input.definition.derived_inputs();
    let inputs = pollster::block_on(DerivedInputs::read(&root, &input.assets, &ids))?;
    let database = derived_database(&input, &inputs)?;
    database.check_weights()?;
    let skeleton: Skeleton = inputs.decode(database.skeleton.id())?;
    let clips = database
        .clips()
        .map(|clip| inputs.decode::<AnimationClip>(clip.id()).map(Arc::new))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let channels: Vec<Box<dyn FeatureChannel>> = database
        .live_channels()?
        .into_iter()
        .map(|channel| channel.dyn_clone())
        .collect();
    Ok(Derived {
        stats: BakedStats::of(&database),
        playback: BakedPlayback {
            database: Arc::new(database),
            skeleton: Arc::new(skeleton),
            clips,
            channels: channels.into(),
        },
    })
}

fn derivable(state: &DefinitionDocument) -> Option<MotionDatabase> {
    state.settings.as_ref()?;
    let definition = state.loaded.as_ref()?;
    (!definition.skeleton.id().is_nil() && !definition.entries.is_empty())
        .then(|| definition.clone())
}

pub(crate) fn follow_written_inputs(
    mut written: EventReader<AssetsWritten>,
    documents: Query<&mut DefinitionDocument>,
) {
    let ids: Vec<AssetId> = written
        .read()
        .flat_map(|written| written.ids.iter().copied())
        .collect();
    if ids.is_empty() {
        return;
    }
    for mut state in documents.iter() {
        let reads = state.loaded.as_ref().is_some_and(|definition| {
            definition
                .derived_inputs()
                .iter()
                .any(|input| ids.contains(input))
        });
        if reads {
            state.derive_wanted = true;
        }
    }
}

fn finish_derive(
    document: Entity,
    editor: &mut EditorDocument,
    state: &mut DefinitionDocument,
    job: DeriveJob,
    previews: &mut PreviewSignals,
) {
    if editor.current.as_ref() != Some(&job.asset)
        || editor.pending.is_some()
        || state.derive_wanted
    {
        return;
    }
    let counted = job.progress().is_some();
    let derived = job
        .handle
        .join()
        .map_err(|_| anyhow!("the worker panicked"))
        .and_then(|derived| derived);
    let mut job_line = job.line;
    match derived {
        Ok(derived) => {
            if counted && editor.status == job_line {
                editor.status.clear();
            }
            state.stats = Some(derived.stats);
            state.playback = Some(derived.playback.clone());
            previews.trigger(document, ShowDatabase(Some(derived.playback)));
        }
        Err(error) => {
            show_derive_line(editor, &mut job_line, format!("Derive failed: {error:#}"));
            forget_derived(document, state, previews);
        }
    }
}

fn derive_may_write(editor: &EditorDocument, held: &str) -> bool {
    editor.status.is_empty() || editor.status == held
}

fn show_derive_line(editor: &mut EditorDocument, held: &mut String, line: String) {
    if !derive_may_write(editor, held) {
        return;
    }
    if editor.status != line {
        editor.status = line.clone();
    }
    *held = line;
}

fn forget_derived(document: Entity, state: &mut DefinitionDocument, previews: &mut PreviewSignals) {
    state.stats = None;
    if state.playback.take().is_some() {
        previews.trigger(document, ShowDatabase(None));
    }
}

pub(crate) fn derive_definitions(
    documents: Query<(Entity, &mut EditorDocument, &mut DefinitionDocument)>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    server: Res<AssetServer>,
    mut previews: PreviewSignals,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for (document, mut editor, mut state) in documents.iter() {
        if state
            .derive_job
            .as_ref()
            .is_some_and(|job| job.handle.is_finished())
            && let Some(job) = state.derive_job.take()
        {
            finish_derive(document, &mut editor, &mut state, job, &mut previews);
        }
        let shown = state.derive_job.as_ref().and_then(|job| {
            let (finished, total) = job.progress()?;
            let line = progress_line(finished, total);
            let stale = job.line != line || editor.status != line;
            (stale && derive_may_write(&editor, &job.line)).then_some(line)
        });
        if let Some(line) = shown
            && let Some(job) = state.derive_job.as_mut()
        {
            show_derive_line(&mut editor, &mut job.line, line);
        }
        if !state.derive_wanted || state.derive_job.is_some() || editor.pending.is_some() {
            continue;
        }
        state.derive_wanted = false;
        let (Some(asset), Some(definition)) = (editor.current.clone(), derivable(&state)) else {
            forget_derived(document, &mut state, &mut previews);
            continue;
        };
        let progress = DeriveProgress::default();
        let input = DeriveInput {
            definition,
            root: project.root.clone(),
            assets: assets.clone(),
            cache: server.derived_cache(),
            progress: progress.clone(),
        };
        state.derive_job = Some(DeriveJob {
            handle: std::thread::spawn(move || run_derive(input)),
            asset,
            progress,
            line: editor.status.clone(),
        });
    }
}

pub(crate) fn mark_background_work(
    documents: Query<(Entity, &DefinitionDocument, Option<&BackgroundWork>)>,
    mut cmd: CommandQueue,
) {
    for (document, state, marked) in documents.iter() {
        let working =
            state.facts_job.is_some() || state.derive_job.is_some() || state.clip_job.is_some();
        match (working, marked.is_some()) {
            (true, false) => cmd.insert(BackgroundWork, document),
            (false, true) => cmd.remove::<BackgroundWork>(document),
            _ => {}
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;

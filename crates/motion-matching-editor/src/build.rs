use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};

use anyhow::{Context, anyhow, ensure};
use concerto_animation::clip::AnimationClip;
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, With, World,
    command::CommandQueue,
    reflect::AppTypeRegistry,
    signal::{On, Signal},
};
use concerto_editor::{
    asset_editor::{BackgroundWork, EditorDocument, Unsaved},
    hosted::HostedApp,
    inspector::{ComponentEdits, PropertyCommits},
    preview::PreviewSignals,
    project::ProjectState,
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{
        CONTENT_FORMAT_VERSION, ContentAssetHeader, read_content_asset, read_content_asset_header,
        write_content_asset,
    },
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::Skeleton;
use concerto_motion_matching::{
    MotionDatabase, MotionDatabaseDefinition, bake_with_progress, is_stale, weightless_hash,
    weights_of,
};

use crate::{
    document::DefinitionDocument,
    facts::{decode, read_bytes},
    playback::{BakedPlayback, ShowDatabase},
    save::{captured, save_and_report, set_status},
};

/// How a definition's baked file compares with the definition as last saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BakedState {
    #[default]
    Unknown,
    NotBuilt,
    UpToDate,
    OutOfDate,
    CantRead(String),
}

impl BakedState {
    /// What the panel shows, empty while nothing is known yet.
    pub fn line(&self) -> String {
        match self {
            Self::Unknown => String::new(),
            Self::NotBuilt => "Not built".into(),
            Self::UpToDate => "Up to date".into(),
            Self::OutOfDate => "Out of date".into(),
            Self::CantRead(asset) => format!("Can't read {asset}"),
        }
    }
}

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
        let trajectory = database.trajectory();
        let horizon = trajectory.sample_dt * trajectory.sample_count as f32;
        let entries = database
            .clips()
            .iter()
            .enumerate()
            .map(|(index, clip)| {
                let frames = database.frames_of_entry(index).unwrap_or(0);
                EntryStats {
                    clip: clip.id(),
                    frames,
                    short: (frames as f32) < horizon * database.rate(),
                }
            })
            .collect();
        let channels = database
            .channels()
            .iter()
            .zip(database.deviations())
            .map(|(span, deviation)| {
                let means = &database.offset()[span.offset..span.offset + span.dims];
                ChannelStats {
                    name: span.type_name.clone(),
                    mean: means.iter().sum::<f32>() / span.dims.max(1) as f32,
                    deviation: *deviation,
                }
            })
            .collect();
        Self {
            entries,
            frames: database.frame_count(),
            bytes: database.size_in_bytes(),
            horizon,
            channels,
        }
    }
}

struct Built {
    bytes: Vec<u8>,
    stats: BakedStats,
    weights_only: bool,
}

struct BuildJob {
    handle: JoinHandle<anyhow::Result<Built>>,
    asset: AssetEntry,
    progress: Arc<AtomicUsize>,
    total: usize,
}

/// The bake a document has running, if any.
#[derive(Default)]
pub struct BuildState {
    job: Option<BuildJob>,
}

impl BuildState {
    /// Whether a bake is running.
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// The entries the running bake has finished, and how many it has.
    pub fn progress(&self) -> Option<(usize, usize)> {
        let job = self.job.as_ref()?;
        Some((job.progress.load(Ordering::Relaxed), job.total))
    }

    fn is_finished(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|job| job.handle.is_finished())
    }
}

pub(crate) struct Checked {
    pub(crate) baked: BakedState,
    pub(crate) stats: Option<BakedStats>,
    pub(crate) playback: Option<BakedPlayback>,
}

pub(crate) type BakedJob = JoinHandle<Checked>;

pub(crate) fn progress_line(finished: usize, total: usize) -> String {
    format!("Baking {} of {total}", (finished + 1).min(total))
}

pub(crate) fn baked_address(definition: &AssetEntry) -> String {
    let name = format!("{}_baked.gasset", definition.name());
    match definition.folder() {
        "" => name,
        folder => format!("{folder}/{name}"),
    }
}

pub(crate) fn build_refusal(state: &DefinitionDocument, hosted: bool) -> Option<&'static str> {
    if !hosted {
        Some("The preview is not running")
    } else if state.loaded.is_none() {
        Some("No definition is open")
    } else if state.skeleton.id().is_nil() {
        Some("Pick a skeleton")
    } else if state.entries.is_empty() {
        Some("Add an entry")
    } else {
        None
    }
}

#[derive(Component)]
pub(crate) struct BuildRequested;

pub(crate) struct StartBuild {
    document: Entity,
}

impl Signal for StartBuild {}

struct Source {
    label: String,
    asset: Option<AssetEntry>,
}

impl Source {
    fn of(id: AssetId, fallback: String, assets: &AssetRegistry) -> Self {
        let asset = assets.get(id).cloned();
        let label = asset
            .as_ref()
            .map_or(fallback, |asset| asset.name().to_owned());
        Self { label, asset }
    }

    fn read<A: Asset>(&self, root: &Path) -> Result<Vec<u8>, String> {
        self.asset
            .as_ref()
            .and_then(|asset| read_bytes::<A>(root, asset).ok())
            .ok_or_else(|| self.label.clone())
    }
}

struct Sources {
    root: PathBuf,
    skeleton: Source,
    clips: Vec<Source>,
}

impl Sources {
    fn of(definition: &MotionDatabaseDefinition, root: PathBuf, assets: &AssetRegistry) -> Self {
        Self {
            root,
            skeleton: Source::of(definition.skeleton.id(), "the skeleton".into(), assets),
            clips: definition
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let fallback = format!("the clip of entry {}", index + 1);
                    Source::of(entry.clip.id(), fallback, assets)
                })
                .collect(),
        }
    }

    fn read(&self) -> Result<SourceBytes, String> {
        Ok(SourceBytes {
            skeleton: self.skeleton.read::<Skeleton>(&self.root)?,
            clips: self
                .clips
                .iter()
                .map(|clip| clip.read::<AnimationClip>(&self.root))
                .collect::<Result<_, _>>()?,
        })
    }
}

struct SourceBytes {
    skeleton: Vec<u8>,
    clips: Vec<Vec<u8>>,
}

impl SourceBytes {
    fn hash(&self, definition: &MotionDatabaseDefinition) -> u64 {
        let clips: Vec<&[u8]> = self.clips.iter().map(Vec::as_slice).collect();
        weightless_hash(definition, &self.skeleton, &clips)
    }
}

fn read_baked(root: &Path, address: &str) -> Option<MotionDatabase> {
    let bytes = std::fs::read(root.join(address)).ok()?;
    let (header, payload) = read_content_asset(&bytes).ok()?;
    if header.kind != MotionDatabase::name() {
        return None;
    }
    bincode::deserialize(payload).ok()
}

fn packed(database: &MotionDatabase, address: &str, weights_only: bool) -> anyhow::Result<Built> {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::from_path(address),
        references: database.referenced_sub_assets(),
        kind: MotionDatabase::name().to_string(),
        provenance: None,
    };
    let payload = bincode::serialize(database).context("failed to serialize the database")?;
    Ok(Built {
        bytes: write_content_asset(&header, &payload)?,
        stats: BakedStats::of(database),
        weights_only,
    })
}

struct BuildInput {
    definition: MotionDatabaseDefinition,
    sources: Sources,
    address: String,
    types: AppTypeRegistry,
    progress: Arc<AtomicUsize>,
}

fn run_build(input: BuildInput) -> anyhow::Result<Built> {
    let BuildInput {
        definition,
        sources,
        address,
        types,
        progress,
    } = input;
    let read = sources
        .read()
        .map_err(|asset| anyhow!("Can't read {asset}"))?;
    refuse_another_asset(&sources.root.join(&address), &address)?;
    let hash = read.hash(&definition);
    let registry = types.read();
    if let Some(mut database) = read_baked(&sources.root, &address)
        && database.source_hash() == hash
    {
        database.set_weights(&weights_of(&definition, &registry)?)?;
        return packed(&database, &address, true);
    }
    let skeleton: Skeleton = decode(&read.skeleton)?;
    let clips = read
        .clips
        .iter()
        .map(|clip| decode(clip))
        .collect::<anyhow::Result<Vec<AnimationClip>>>()?;
    let clips: Vec<&AnimationClip> = clips.iter().collect();
    let database = bake_with_progress(&definition, &skeleton, &clips, &registry, |finished, _| {
        progress.store(finished, Ordering::Relaxed);
    })?
    .with_source_hash(hash);
    packed(&database, &address, false)
}

pub(crate) fn request_builds(
    requested: Query<Entity, (With<BuildRequested>, With<DefinitionDocument>)>,
    commits: Res<PropertyCommits>,
    edits: Res<ComponentEdits>,
    mut cmd: CommandQueue,
) {
    if !commits.0.is_empty() || !edits.0.is_empty() {
        return;
    }
    for document in requested.iter() {
        cmd.remove::<BuildRequested>(document);
        cmd.trigger(StartBuild { document });
    }
}

fn is_edited(world: &World, document: Entity) -> bool {
    world
        .get_component_for_entity::<Unsaved>(document)
        .is_some()
        || world
            .get_component_for_entity::<HostedApp>(document)
            .is_some_and(HostedApp::scene_dirty)
}

fn can_start(world: &World, document: Entity) -> bool {
    let hosted = world
        .get_component_for_entity::<HostedApp>(document)
        .is_some();
    world
        .get_component_for_entity::<DefinitionDocument>(document)
        .is_some_and(|state| !state.build.is_running() && build_refusal(state, hosted).is_none())
}

fn begin(world: &mut World, document: Entity) -> anyhow::Result<BuildJob> {
    let definition = captured(world, document)?;
    let asset = world
        .get_component_for_entity::<EditorDocument>(document)
        .and_then(|document| document.current.clone())
        .context("No definition is open")?;
    let root = world
        .get_resource::<ProjectState>()
        .and_then(|state| state.project.as_ref())
        .context("Open a project before building")?
        .root
        .clone();
    let types = world
        .get_component_for_entity::<HostedApp>(document)
        .and_then(|hosted| hosted.world().get_resource::<AppTypeRegistry>())
        .context("The preview has no type registry")?
        .clone();
    let assets = world
        .get_resource::<AssetRegistry>()
        .context("The editor has no asset registry")?;
    let sources = Sources::of(&definition, root, assets);
    let total = definition.entries.len();
    let progress = Arc::new(AtomicUsize::new(0));
    let input = BuildInput {
        definition,
        sources,
        address: baked_address(&asset),
        types,
        progress: progress.clone(),
    };
    Ok(BuildJob {
        handle: std::thread::spawn(move || run_build(input)),
        asset,
        progress,
        total,
    })
}

pub(crate) fn start_build(on: On<StartBuild>, world: &mut World) {
    let document = on.signal().document;
    if !can_start(world, document) {
        return;
    }
    if is_edited(world, document) && !save_and_report(world, document) {
        return;
    }
    match begin(world, document) {
        Ok(job) => {
            set_status(world, document, String::new());
            if let Some(state) = world.get_component_for_entity_mut::<DefinitionDocument>(document)
            {
                state.build.job = Some(job);
            }
        }
        Err(error) => set_status(world, document, format!("Build failed: {error:#}")),
    }
}

fn refuse_another_asset(path: &Path, address: &str) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let header = read_content_asset_header(path)
        .map_err(|_| anyhow!("Can't tell what {address} holds, so Build left it alone"))?;
    ensure!(
        header.kind == MotionDatabase::name(),
        "{address} holds a {}, so Build left it alone",
        header.kind
    );
    Ok(())
}

fn install(
    built: &Built,
    asset: &AssetEntry,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<String> {
    let address = baked_address(asset);
    let path = root.join(&address);
    refuse_another_asset(&path, &address)?;
    std::fs::write(&path, &built.bytes)
        .with_context(|| format!("failed to write '{}'", path.display()))?;
    assets.insert(AssetEntry {
        id: AssetId::from_path(&address),
        address: address.clone(),
        kind: MotionDatabase::name().to_string(),
        provenance: None,
    });
    Ok(address)
}

fn built_status(built: &Built, address: &str) -> String {
    if built.weights_only {
        format!("Updated the weights in {address}")
    } else {
        format!("Built {address}")
    }
}

pub(crate) fn finish_build(
    documents: Query<(&mut EditorDocument, &mut DefinitionDocument)>,
    project: Res<ProjectState>,
    mut assets: ResMut<AssetRegistry>,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for (mut document, mut state) in documents.iter() {
        if !state.build.is_finished() {
            continue;
        }
        let Some(job) = state.build.job.take() else {
            continue;
        };
        if document.current.as_ref() != Some(&job.asset) || document.pending.is_some() {
            continue;
        }
        let built = job
            .handle
            .join()
            .map_err(|_| anyhow!("the worker panicked"))
            .and_then(|built| built)
            .and_then(|built| {
                let address = install(&built, &job.asset, &project.root, &mut assets)?;
                Ok((built, address))
            });
        match built {
            Ok((built, address)) => {
                document.status = built_status(&built, &address);
                state.baked = BakedState::UpToDate;
                state.stats = Some(built.stats);
                state.baked_wanted = true;
            }
            Err(error) => document.status = format!("Build failed: {error:#}"),
        }
    }
}

struct CheckInput {
    definition: MotionDatabaseDefinition,
    sources: Sources,
    address: String,
    types: AppTypeRegistry,
    assets: AssetRegistry,
}

fn baked_state(database: &MotionDatabase, input: &CheckInput) -> BakedState {
    let read = match input.sources.read() {
        Ok(read) => read,
        Err(asset) => return BakedState::CantRead(asset),
    };
    let hash = read.hash(&input.definition);
    if is_stale(database, &input.definition, hash, &input.types.read()) {
        BakedState::OutOfDate
    } else {
        BakedState::UpToDate
    }
}

fn read_playback(database: MotionDatabase, input: &CheckInput) -> Option<BakedPlayback> {
    if database.skeleton().id() != input.definition.skeleton.id() {
        return None;
    }
    let root = &input.sources.root;
    let skeleton: Skeleton = decode(&input.sources.skeleton.read::<Skeleton>(root).ok()?).ok()?;
    let clips = database
        .clips()
        .iter()
        .map(|clip| {
            let source = Source::of(clip.id(), String::new(), &input.assets);
            let bytes = source.read::<AnimationClip>(root).ok()?;
            Some(Arc::new(decode::<AnimationClip>(&bytes).ok()?))
        })
        .collect::<Option<Vec<_>>>()?;
    let sampled: Vec<&AnimationClip> = clips.iter().map(Arc::as_ref).collect();
    database.pose(0, &skeleton, &sampled).ok()?;
    Some(BakedPlayback {
        database: Arc::new(database),
        skeleton: Arc::new(skeleton),
        clips,
    })
}

fn run_check(input: CheckInput) -> Checked {
    let Some(database) = read_baked(&input.sources.root, &input.address) else {
        return Checked {
            baked: BakedState::NotBuilt,
            stats: None,
            playback: None,
        };
    };
    Checked {
        baked: baked_state(&database, &input),
        stats: Some(BakedStats::of(&database)),
        playback: read_playback(database, &input),
    }
}

pub(crate) fn check_baked(
    documents: Query<(
        Entity,
        &EditorDocument,
        &mut DefinitionDocument,
        Option<&HostedApp>,
    )>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for (entity, document, mut state, hosted) in documents.iter() {
        if state
            .baked_job
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
            && let Some(Ok(checked)) = state.baked_job.take().map(JoinHandle::join)
        {
            state.baked = checked.baked;
            state.stats = checked.stats;
            state.playback = checked.playback.clone();
            previews.trigger(entity, ShowDatabase(checked.playback));
        }
        if !state.baked_wanted || state.baked_job.is_some() {
            continue;
        }
        let types = hosted.and_then(|hosted| hosted.world().get_resource::<AppTypeRegistry>());
        let (Some(asset), Some(definition), Some(types)) =
            (&document.current, &state.loaded, types)
        else {
            continue;
        };
        let input = CheckInput {
            definition: definition.clone(),
            sources: Sources::of(definition, project.root.clone(), &assets),
            address: baked_address(asset),
            types: types.clone(),
            assets: assets.clone(),
        };
        state.baked_wanted = false;
        state.baked_job = Some(std::thread::spawn(move || run_check(input)));
    }
}

pub(crate) fn mark_background_work(
    documents: Query<(Entity, &DefinitionDocument, Option<&BackgroundWork>)>,
    mut cmd: CommandQueue,
) {
    for (document, state, marked) in documents.iter() {
        let working = state.facts_job.is_some()
            || state.baked_job.is_some()
            || state.clip_job.is_some()
            || state.build.is_running();
        match (working, marked.is_some()) {
            (true, false) => cmd.insert(BackgroundWork, document),
            (false, true) => cmd.remove::<BackgroundWork>(document),
            _ => {}
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;

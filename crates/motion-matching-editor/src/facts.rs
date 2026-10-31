use std::{path::Path, sync::Arc, thread::JoinHandle};

use anyhow::Context;
use concerto_ecs::{Entity, Query, Res};
use concerto_editor::{
    clip_editor::{ClipFacts, read_clip},
    preview::PreviewSignals,
    project::ProjectState,
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::read_content_asset,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::Skeleton;
use serde::de::DeserializeOwned;

use crate::{document::DefinitionDocument, preview::ShowSkeletonBones};

/// What an entry's clip holds and how much of `skeleton` it drives, or why it could not be read.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryFacts {
    pub clip: AssetId,
    pub skeleton: AssetId,
    pub read: Result<ClipFacts, String>,
}

pub(crate) type FactsJob = JoinHandle<Vec<EntryFacts>>;

pub(crate) struct ShownSkeleton {
    pub(crate) id: AssetId,
    pub(crate) skeleton: Option<Arc<Skeleton>>,
    pub(crate) error: Option<String>,
}

pub(crate) fn read_bytes<A: Asset>(root: &Path, asset: &AssetEntry) -> anyhow::Result<Vec<u8>> {
    let bytes = std::fs::read(root.join(&asset.address))?;
    let (header, _) = read_content_asset(&bytes)?;
    anyhow::ensure!(
        header.asset_id == asset.id && header.kind == A::name(),
        "Asset identity or type mismatch"
    );
    Ok(bytes)
}

pub(crate) fn decode<A: DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<A> {
    let (_, payload) = read_content_asset(bytes)?;
    Ok(bincode::deserialize(payload)?)
}

/// Reads the skeleton `asset` names under `root`, checking its id and its kind.
pub fn read_skeleton(root: &Path, asset: &AssetEntry) -> anyhow::Result<Skeleton> {
    decode(&read_bytes::<Skeleton>(root, asset)?)
}

fn read_shown(id: AssetId, root: &Path, assets: &AssetRegistry) -> ShownSkeleton {
    let mut shown = ShownSkeleton {
        id,
        skeleton: None,
        error: None,
    };
    if id.is_nil() {
        return shown;
    }
    let read = assets
        .get(id)
        .context("The skeleton is not in the project")
        .and_then(|asset| read_skeleton(root, asset));
    match read {
        Ok(skeleton) => shown.skeleton = Some(Arc::new(skeleton)),
        Err(error) => shown.error = Some(format!("{error:#}")),
    }
    shown
}

pub(crate) fn follow_skeleton(
    documents: Query<(Entity, &mut DefinitionDocument)>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for (document, mut state) in documents.iter() {
        let id = state.skeleton.id();
        if state
            .shown_skeleton
            .as_ref()
            .is_some_and(|shown| shown.id == id)
        {
            continue;
        }
        let shown = read_shown(id, &project.root, &assets);
        let bones = shown
            .skeleton
            .as_ref()
            .map(|skeleton| skeleton.names.to_vec());
        previews.trigger(document, ShowSkeletonBones(bones));
        state.shown_skeleton = Some(shown);
    }
}

fn read_facts(
    root: &Path,
    asset: Option<AssetEntry>,
    skeleton: Option<&Skeleton>,
) -> Result<ClipFacts, String> {
    let asset = asset.ok_or("The clip is not in the project")?;
    let clip = read_clip(root, &asset).map_err(|error| format!("{error:#}"))?;
    let (skeleton_bones, driven) = skeleton.map_or((0, 0), |skeleton| {
        let driven = skeleton
            .bone_ids
            .iter()
            .filter(|id| clip.get_channels(id).is_some())
            .count();
        (skeleton.bone_ids.len(), driven)
    });
    Ok(ClipFacts {
        skeleton_bones,
        driven,
        ..ClipFacts::of(&clip)
    })
}

fn unread_clips(state: &DefinitionDocument, skeleton: AssetId) -> Vec<AssetId> {
    let mut unread = Vec::new();
    for entry in &state.entries {
        let clip = entry.clip.id();
        let read = state
            .entry_facts
            .iter()
            .any(|facts| facts.clip == clip && facts.skeleton == skeleton);
        if !read && !unread.contains(&clip) {
            unread.push(clip);
        }
    }
    unread
}

fn start_facts_job(
    clips: Vec<(AssetId, Option<AssetEntry>)>,
    skeleton: AssetId,
    shown: Option<Arc<Skeleton>>,
    root: std::path::PathBuf,
) -> FactsJob {
    std::thread::spawn(move || {
        clips
            .into_iter()
            .map(|(clip, asset)| EntryFacts {
                clip,
                skeleton,
                read: read_facts(&root, asset, shown.as_deref()),
            })
            .collect()
    })
}

pub(crate) fn read_entry_facts(
    documents: Query<&mut DefinitionDocument>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for mut state in documents.iter() {
        if state
            .facts_job
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
            && let Some(Ok(read)) = state.facts_job.take().map(JoinHandle::join)
        {
            state.entry_facts.extend(read);
        }
        let skeleton = state.skeleton.id();
        if state.facts_job.is_some() {
            continue;
        }
        if state
            .entry_facts
            .iter()
            .any(|facts| facts.skeleton != skeleton)
        {
            state.entry_facts.retain(|facts| facts.skeleton == skeleton);
        }
        let Some(shown) = state
            .shown_skeleton
            .as_ref()
            .filter(|shown| shown.id == skeleton)
            .map(|shown| shown.skeleton.clone())
        else {
            continue;
        };
        let unread = unread_clips(&state, skeleton);
        if unread.is_empty() {
            continue;
        }
        let clips = unread
            .into_iter()
            .map(|clip| (clip, assets.get(clip).cloned()))
            .collect();
        state.facts_job = Some(start_facts_job(
            clips,
            skeleton,
            shown,
            project.root.clone(),
        ));
    }
}

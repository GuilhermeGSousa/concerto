use std::{ops::Range, path::PathBuf, sync::Arc, thread::JoinHandle};

use concerto_animation::clip::AnimationClip;
use concerto_animation_editor::{
    preview::{ClearClip, ClipPreviewProgress, SeekClip, ShowClip},
    read_clip,
};
use concerto_app::{App, edit_time::AtPreviewTime, schedule_groups::LateUpdate};
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
use concerto_ecs::{
    Entity, Event, IntoSystemConfig, Query, Res, ResMut, Resource,
    entity::hierarchy::ChildOf,
    events::event_writer::EventWriter,
    signal::{
        On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    hosted::HostedAppEvents,
    inspector::{InspectedValues, ShownValue},
    preview::PreviewSignals,
    project::ProjectState,
    timeline::TimelineScrubbed,
};
use concerto_foundation::{
    assets::{
        AssetId,
        asset_store::AssetStore,
        registry::{AssetEntry, AssetRegistry},
    },
    time::Time,
    transform::{GlobalTransform, Transform, systems::propagate_global_transforms},
};
use concerto_mesh::{JointPose, Skeleton, SkeletonComponent};
use concerto_motion_matching::{FeatureChannel, FeatureShape, MotionDatabase, preview_features};
use concerto_motion_matching_gizmos::{
    BakedFrame, SIMULATION_WIDTH, channel_color, draw_shape, draw_simulation, local_pose,
    placed_pose,
};

use crate::{document::DefinitionDocument, panels::shown_index};

/// A baked database with the skeleton and the clips it samples, as a preview plays it.
#[derive(Clone)]
pub struct BakedPlayback {
    pub database: Arc<MotionDatabase>,
    pub skeleton: Arc<Skeleton>,
    pub clips: Vec<Arc<AnimationClip>>,
    pub channels: Arc<[Box<dyn FeatureChannel>]>,
}

/// Replaces the baked database the preview plays; `None` stops it.
pub struct ShowDatabase(pub Option<BakedPlayback>);

impl Signal for ShowDatabase {}

/// Moves the preview to the baked frame of `entry` nearest to `time` in its clip, in seconds.
pub struct SeekDatabase {
    pub entry: usize,
    pub time: f32,
}

impl Signal for SeekDatabase {}

/// Names the baked channel the preview draws at full colour; the others are dimmed.
pub struct ShowChannel(pub Option<usize>);

impl Signal for ShowChannel {}

/// Which entry the shown baked frame belongs to and where it sits in the clip; written when the frame changes.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct DatabaseProgress {
    pub entry: usize,
    pub time: f32,
}

#[derive(Resource, Default)]
pub(crate) struct DatabasePlayback {
    pub(crate) shown: Option<BakedPlayback>,
    pub(crate) picked: Option<usize>,
    looped: Option<usize>,
    position: f32,
    reported: Option<usize>,
}

impl DatabasePlayback {
    pub(crate) fn frame(&self) -> usize {
        self.position as usize
    }
}

pub(crate) struct ClipJob {
    handle: JoinHandle<anyhow::Result<AnimationClip>>,
}

fn stepped(position: f32, delta: f32, rate: f32, range: Range<usize>) -> f32 {
    let start = range.start as f32;
    if !range.contains(&(position as usize)) {
        return start;
    }
    start + (position - start + delta * rate) % range.len() as f32
}

fn looped_range(database: &MotionDatabase, entry: Option<usize>) -> Range<usize> {
    entry
        .and_then(|entry| database.ranges()?.nth(entry))
        .unwrap_or(0..database.frame_count().unwrap_or(0))
}

fn frame_in_entry(database: &MotionDatabase, entry: usize, time: f32) -> Option<usize> {
    let range = database.ranges()?.nth(entry)?;
    let (_, start) = database.clip_time(range.start)?;
    let along = (time - start) * database.rate / database.speed;
    let last = range.len().checked_sub(1)?;
    Some(range.start + (along.round().max(0.0) as usize).min(last))
}

fn sampled_clips(shown: &BakedPlayback) -> Vec<&AnimationClip> {
    shown.clips.iter().map(Arc::as_ref).collect()
}

fn write_pose(
    pose: &[JointPose],
    played: &Skeleton,
    component: &SkeletonComponent,
    skeleton: &Skeleton,
    transforms: &Query<&mut Transform>,
) {
    for (bone, id) in component.bones().iter().zip(&skeleton.bone_ids) {
        let joint = played.bone_ids.iter().position(|played| played == id);
        let target = bone.entity().and_then(|bone| transforms.get_entity(bone));
        if let (Some(joint), Some(mut transform)) = (joint, target) {
            transform.translation = pose[joint].translation;
            transform.rotation = pose[joint].rotation;
            transform.scale = pose[joint].scale;
        }
    }
}

fn play_database(
    mut playback: ResMut<DatabasePlayback>,
    time: Res<Time>,
    stores: Option<Res<AssetStore<Skeleton>>>,
    skeletons: Query<&SkeletonComponent>,
    transforms: Query<&mut Transform>,
    mut progress: EventWriter<DatabaseProgress>,
) {
    let playback = &mut *playback;
    let Some(shown) = &playback.shown else {
        return;
    };
    playback.position = stepped(
        playback.position,
        time.delta().as_secs_f32(),
        shown.database.rate,
        looped_range(&shown.database, playback.looped),
    );
    let frame = playback.position as usize;
    let clips = sampled_clips(shown);
    let Some(simulation) = shown.database.simulation_bone(frame) else {
        return;
    };
    let Some(pose) = local_pose(&shown.database, &shown.skeleton, &clips, frame)
        .map(|pose| placed_pose(&shown.skeleton, &simulation, pose))
    else {
        return;
    };
    if let Some(stores) = stores {
        for component in skeletons.iter() {
            if component.root().is_none() {
                continue;
            }
            if let Some(skeleton) = stores.get(component.skeleton()) {
                write_pose(&pose, &shown.skeleton, component, skeleton, &transforms);
            }
        }
    }
    if playback.reported != Some(frame) {
        playback.reported = Some(frame);
        if let Some((entry, time)) = shown.database.clip_time(frame) {
            progress.write(DatabaseProgress { entry, time });
        }
    }
}

fn frame_shapes(shown: &BakedPlayback, frame: usize) -> Option<Vec<Vec<FeatureShape>>> {
    let clips = sampled_clips(shown);
    let baked = BakedFrame::new(&shown.database, &shown.skeleton, &clips, frame)?;
    Some(preview_features(
        &shown.channels,
        &baked.context(&shown.skeleton),
        shown.database.features(frame)?,
    ))
}

fn stage_of(
    skeletons: &Query<&SkeletonComponent>,
    parents: &Query<&ChildOf>,
    globals: &Query<&GlobalTransform>,
) -> Transform {
    skeletons
        .iter()
        .find_map(SkeletonComponent::root)
        .and_then(|root| parents.get_entity(root))
        .and_then(|parent| globals.get_entity(parent.parent()))
        .map_or(Transform::IDENTITY, GlobalTransform::to_transform)
}

fn draw_simulation_bone(
    playback: Res<DatabasePlayback>,
    skeletons: Query<&SkeletonComponent>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    mut gizmos: DebugGizmos,
) {
    let Some(shown) = &playback.shown else {
        return;
    };
    let Some(simulation) = shown.database.simulation_bone(playback.frame()) else {
        return;
    };
    let stage = stage_of(&skeletons, &parents, &globals);
    let bone = stage.mul_transform(simulation);
    gizmos.set_line_width(SIMULATION_WIDTH);
    draw_simulation(&mut gizmos, &bone);
}

fn draw_feature_channels(
    playback: Res<DatabasePlayback>,
    skeletons: Query<&SkeletonComponent>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    mut gizmos: DebugGizmos,
) {
    let Some(shown) = playback
        .shown
        .as_ref()
        .filter(|shown| !shown.channels.is_empty())
    else {
        return;
    };
    let frame = playback.frame();
    let Some(channels) = frame_shapes(shown, frame) else {
        return;
    };
    let Some(simulation) = shown.database.simulation_bone(frame) else {
        return;
    };
    let stage = stage_of(&skeletons, &parents, &globals);
    let bone = stage.mul_transform(simulation);
    gizmos.set_line_width(SIMULATION_WIDTH);
    for (channel, shapes) in channels.iter().enumerate() {
        let color = channel_color(channel, playback.picked);
        for shape in shapes {
            draw_shape(&mut gizmos, &bone, *shape, color);
        }
    }
}

fn show_channel(on: On<ShowChannel>, mut playback: ResMut<DatabasePlayback>) {
    playback.picked = on.signal().0;
}

fn show_database(on: On<ShowDatabase>, mut playback: ResMut<DatabasePlayback>) {
    let shown = on.signal().0.clone();
    let frames = shown
        .as_ref()
        .and_then(|shown| shown.database.frame_count())
        .unwrap_or(0);
    if playback.frame() >= frames {
        playback.position = 0.0;
    }
    playback.shown = shown;
    playback.reported = None;
}

fn seek_database(on: On<SeekDatabase>, mut playback: ResMut<DatabasePlayback>) {
    let SeekDatabase { entry, time } = *on.signal();
    let frame = playback
        .shown
        .as_ref()
        .and_then(|shown| frame_in_entry(&shown.database, entry, time));
    if let Some(frame) = frame {
        playback.position = frame as f32;
        playback.looped = Some(entry);
    }
}

pub(crate) fn baked_entry(state: &DefinitionDocument) -> Option<usize> {
    let index = state.selected_entry?;
    let database = &state.playback.as_ref()?.database;
    database.frames_of_entry(index)?;
    let baked = database.entries.get(index)?;
    let entry = state.entries.get(index)?;
    (entry.clip.id() == baked.clip.id() && entry.mirrored == baked.mirrored).then_some(index)
}

pub(crate) fn scrub_entry(document: Entity) -> Listener<TimelineScrubbed> {
    (move |on: On<TimelineScrubbed>,
           documents: Query<&mut DefinitionDocument>,
           mut previews: PreviewSignals| {
        let Some(mut state) = documents.get_entity(document) else {
            return;
        };
        let time = on.signal().time;
        state.time = time;
        if state.playback.is_none() {
            previews.trigger(document, SeekClip(time));
        } else if let Some(entry) = baked_entry(&state) {
            previews.trigger(document, SeekDatabase { entry, time });
        }
    })
    .into_listener()
}

fn wanted_clip(state: &DefinitionDocument) -> Option<AssetId> {
    if state.playback.is_some() || state.derive_wanted || state.derive_job.is_some() {
        return None;
    }
    Some(state.entries.get(state.selected_entry?)?.clip.id())
}

fn deliver_clip(job: ClipJob, document: Entity, previews: &mut PreviewSignals) {
    match job.handle.join() {
        Ok(Ok(clip)) => previews.trigger(document, ShowClip(Arc::new(clip))),
        _ => previews.trigger(document, ClearClip),
    }
}

fn start_clip_job(asset: AssetEntry, root: PathBuf) -> ClipJob {
    ClipJob {
        handle: std::thread::spawn(move || read_clip(&root, &asset)),
    }
}

fn picked_channel(
    state: &DefinitionDocument,
    values: Option<&InspectedValues>,
    shown: Option<&ShownValue>,
) -> Option<usize> {
    let index = shown_index(values?, shown?)?.checked_sub(1)?;
    let held = state.channels.get(index)?;
    let baked = state.playback.as_ref()?.database.channels.get(index)?;
    (held.type_name == baked.type_name()).then_some(index)
}

pub(crate) fn follow_playback(
    documents: Query<(
        Entity,
        &mut DefinitionDocument,
        Option<&InspectedValues>,
        Option<&ShownValue>,
    )>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for (document, mut state, values, shown) in documents.iter() {
        let skeleton = state.skeleton.id();
        if state
            .playback
            .as_ref()
            .is_some_and(|shown| shown.database.skeleton.id() != skeleton)
        {
            state.playback = None;
            previews.trigger(document, ShowDatabase(None));
        }

        let picked = picked_channel(&state, values, shown);
        if state.picked_channel != picked {
            state.picked_channel = picked;
            previews.trigger(document, ShowChannel(picked));
        }

        let sought = baked_entry(&state);
        if state.sought_entry != sought {
            state.sought_entry = sought;
            if let Some(entry) = sought {
                previews.trigger(document, SeekDatabase { entry, time: 0.0 });
            }
        }

        if state
            .clip_job
            .as_ref()
            .is_some_and(|job| job.handle.is_finished())
            && let Some(job) = state.clip_job.take()
        {
            deliver_clip(job, document, &mut previews);
        }

        let wanted = wanted_clip(&state);
        if state.shown_clip == wanted {
            continue;
        }
        state.shown_clip = wanted;
        state.clip_job = None;
        match wanted.and_then(|clip| assets.get(clip)) {
            Some(asset) => {
                state.clip_job = Some(start_clip_job(asset.clone(), project.root.clone()));
            }
            None => previews.trigger(document, ClearClip),
        }
    }
}

pub(crate) fn follow_preview_time(
    mut clips: HostedAppEvents<ClipPreviewProgress>,
    mut baked: HostedAppEvents<DatabaseProgress>,
    documents: Query<&mut DefinitionDocument>,
) {
    for (document, progress) in clips.read() {
        if let Some(mut state) = documents.get_entity(document)
            && state.playback.is_none()
        {
            state.time = progress.time;
        }
    }
    for (document, progress) in baked.read() {
        if let Some(mut state) = documents.get_entity(document)
            && baked_entry(&state) == Some(progress.entry)
        {
            state.time = progress.time;
        }
    }
}

pub(crate) fn database_playback(app: &mut App) {
    app.insert_resource(DatabasePlayback::default())
        .register_event::<DatabaseProgress>()
        .add_system(
            LateUpdate,
            play_database
                .before(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_listener(show_database.at_preview_time())
        .add_listener(seek_database.at_preview_time())
        .add_listener(show_channel.at_preview_time());
    if app.is_plugin_added::<DebugGizmosPlugin>() {
        app.add_system(
            LateUpdate,
            draw_simulation_bone
                .after(propagate_global_transforms)
                .at_preview_time(),
        );
        app.add_system(
            LateUpdate,
            draw_feature_channels
                .after(propagate_global_transforms)
                .at_preview_time(),
        );
    }
}

#[cfg(test)]
pub(crate) mod tests;

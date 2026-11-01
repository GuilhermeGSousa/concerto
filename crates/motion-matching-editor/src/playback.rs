use std::{path::PathBuf, sync::Arc, thread::JoinHandle};

use concerto_animation::clip::AnimationClip;
use concerto_app::{App, edit_time::AtPreviewTime, schedule_groups::LateUpdate};
use concerto_color::Color;
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
    clip_editor::{
        preview::{ClearClip, ClipPreviewProgress, SeekClip, ShowClip},
        read_clip,
    },
    hosted::HostedAppEvents,
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
use concerto_motion_matching::MotionDatabase;

use crate::{build::BakedState, document::DefinitionDocument};

const SIMULATION_COLOR: Color = Color::rgba(1.0, 0.85, 0.1, 1.0);
const SIMULATION_RADIUS: f32 = 0.3;
const SIMULATION_ARROW: f32 = 0.6;
const SIMULATION_WIDTH: f32 = 2.0;

/// A baked database with the skeleton and the clips it samples, as a preview plays it.
#[derive(Clone)]
pub struct BakedPlayback {
    pub database: Arc<MotionDatabase>,
    pub skeleton: Arc<Skeleton>,
    pub clips: Vec<Arc<AnimationClip>>,
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

/// Which entry the shown baked frame belongs to and where it sits in the clip; written when the frame changes.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct DatabaseProgress {
    pub entry: usize,
    pub time: f32,
}

#[derive(Resource, Default)]
pub(crate) struct DatabasePlayback {
    pub(crate) shown: Option<BakedPlayback>,
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

fn stepped(position: f32, delta: f32, rate: f32, frames: usize) -> f32 {
    (position + delta * rate) % frames.max(1) as f32
}

fn frame_in_entry(database: &MotionDatabase, entry: usize, time: f32) -> Option<usize> {
    let range = database.ranges().nth(entry)?;
    let (_, start) = database.clip_time(range.start)?;
    let along = (time - start) * database.rate() / database.speed();
    let last = range.len().checked_sub(1)?;
    Some(range.start + (along.round().max(0.0) as usize).min(last))
}

fn placed_pose(shown: &BakedPlayback, frame: usize) -> Option<Vec<JointPose>> {
    let clips: Vec<&AnimationClip> = shown.clips.iter().map(Arc::as_ref).collect();
    let mut pose = shown.database.pose(frame, &shown.skeleton, &clips).ok()?;
    let simulation = shown.database.simulation(frame);
    for (joint, parent) in pose.iter_mut().zip(shown.skeleton.parents.iter()) {
        if parent.is_none() {
            joint.translation = simulation.rotation * joint.translation + simulation.translation;
            joint.rotation = simulation.rotation * joint.rotation;
        }
    }
    Some(pose)
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
        shown.database.rate(),
        shown.database.frame_count(),
    );
    let frame = playback.position as usize;
    let Some(pose) = placed_pose(shown, frame) else {
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
    let stage = skeletons
        .iter()
        .find_map(SkeletonComponent::root)
        .and_then(|root| parents.get_entity(root))
        .and_then(|parent| globals.get_entity(parent.parent()))
        .map_or(Transform::IDENTITY, GlobalTransform::to_transform);
    let bone = stage.mul_transform(shown.database.simulation(playback.frame()));
    let tip = bone.translation + bone.local_z() * SIMULATION_ARROW;
    gizmos.set_line_width(SIMULATION_WIDTH);
    gizmos.circle(
        bone.translation,
        bone.local_y(),
        SIMULATION_RADIUS,
        SIMULATION_COLOR,
    );
    gizmos.arrow(bone.translation, tip, SIMULATION_COLOR);
}

fn show_database(on: On<ShowDatabase>, mut playback: ResMut<DatabasePlayback>) {
    let shown = on.signal().0.clone();
    let frames = shown
        .as_ref()
        .map_or(0, |shown| shown.database.frame_count());
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
    }
}

pub(crate) fn baked_entry(state: &DefinitionDocument) -> Option<usize> {
    let index = state.selected_entry?;
    let baked = state.playback.as_ref()?.database.clips().get(index)?;
    (state.entries.get(index)?.clip.id() == baked.id()).then_some(index)
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
    if state.playback.is_some() || state.baked == BakedState::Unknown {
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

pub(crate) fn follow_playback(
    documents: Query<(Entity, &mut DefinitionDocument)>,
    project: Res<ProjectState>,
    assets: Res<AssetRegistry>,
    mut previews: PreviewSignals,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for (document, mut state) in documents.iter() {
        let skeleton = state.skeleton.id();
        if state
            .playback
            .as_ref()
            .is_some_and(|shown| shown.database.skeleton().id() != skeleton)
        {
            state.playback = None;
            previews.trigger(document, ShowDatabase(None));
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
        .add_listener(seek_database.at_preview_time());
    if app.is_plugin_added::<DebugGizmosPlugin>() {
        app.add_system(
            LateUpdate,
            draw_simulation_bone
                .after(propagate_global_transforms)
                .at_preview_time(),
        );
    }
}

#[cfg(test)]
pub(crate) mod tests;

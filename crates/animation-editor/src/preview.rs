//! Everything that runs in a clip editor's preview `App`: playing a clip on a skeleton and reporting back.
use std::sync::Arc;

use concerto_animation::clip::AnimationClip;
use concerto_app::{
    App,
    edit_time::AtPreviewTime,
    schedule_groups::{LateUpdate, Update},
};
use concerto_debug_gizmos::DebugGizmosPlugin;
use concerto_ecs::{
    Added, Component, Entity, Event, IntoSystemConfig, Query, Res, ResMut, Resource, With, World,
    command::CommandQueue,
    entity::hierarchy::ChildOf,
    events::event_writer::EventWriter,
    signal::{On, Signal},
    system::input::SystemLocal,
    world::FromWorld,
};
use concerto_editor::preview::OrbitCamera;
use concerto_foundation::{
    assets::asset_store::AssetStore,
    time::Time,
    transform::{GlobalTransform, Transform, systems::propagate_global_transforms},
};
use concerto_mesh::{
    SkeletonGizmo,
    skeleton::{Skeleton, SkeletonComponent},
};
use concerto_name::Name;
use concerto_render::plugin::RenderPlugin;
use concerto_scene::{
    scene::Scene,
    spawner::{SpawnFilter, spawn_scene},
};
use glam::Vec3;
use uuid::Uuid;

#[derive(Resource)]
pub(crate) struct ClipPlayback {
    pub(crate) clip: Option<Arc<AnimationClip>>,
    pub(crate) time: f32,
    pub(crate) rate: f32,
    pub(crate) playing: bool,
    pub(crate) looping: bool,
}

impl Default for ClipPlayback {
    fn default() -> Self {
        Self {
            clip: None,
            time: 0.0,
            rate: 1.0,
            playing: true,
            looping: true,
        }
    }
}

#[derive(Resource)]
pub(super) struct ClipPosing {
    pub(super) root_motion: bool,
    pub(super) skeleton_gizmo: bool,
}

impl Default for ClipPosing {
    fn default() -> Self {
        Self {
            root_motion: false,
            skeleton_gizmo: true,
        }
    }
}

/// The entity a preview's scene is spawned under, which root motion moves.
#[derive(Component)]
pub(crate) struct ClipPreviewRoot;

/// Replaces the clip the preview plays and goes back to its start.
pub struct ShowClip(pub Arc<AnimationClip>);
impl Signal for ShowClip {}

/// Leaves the preview with no clip, so it no longer writes the bones.
pub struct ClearClip;
impl Signal for ClearClip {}

/// Replaces the scene the preview shows; `None` leaves the stage empty.
pub struct ShowClipScene(pub Option<Scene>);
impl Signal for ShowClipScene {}

/// Sets how the preview plays the clip.
pub struct SetClipTransport {
    pub playing: bool,
    pub looping: bool,
    pub rate: f32,
}
impl Signal for SetClipTransport {}

/// Moves the preview to a time in the clip, in seconds.
pub struct SeekClip(pub f32);
impl Signal for SeekClip {}

/// Sets whether the root bone's movement moves the character.
pub struct SetRootMotion(pub bool);
impl Signal for SetRootMotion {}

/// Sets whether the preview draws its skeletons' bones.
pub struct SetSkeletonGizmo(pub bool);
impl Signal for SetSkeletonGizmo {}

/// Where the preview is in the clip; written when either field changes.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ClipPreviewProgress {
    pub time: f32,
    pub playing: bool,
}

/// A bone of the shown skeleton: the id a clip's channels target and the bone's name.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownBone {
    pub id: Uuid,
    pub name: Name,
}

/// How many bones the shown skeleton has, how many of them the clip drives, and its bones in order; written when any changes.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ClipPreviewBones {
    pub bones: usize,
    pub driven: usize,
    pub skeleton: Vec<ShownBone>,
}

fn shows_skeleton(reported: &[ShownBone], skeleton: Option<&Skeleton>) -> bool {
    let Some(skeleton) = skeleton else {
        return reported.is_empty();
    };
    reported.len() == skeleton.bone_ids.len()
        && reported
            .iter()
            .zip(skeleton.bone_ids.iter().zip(skeleton.names.iter()))
            .all(|(bone, (id, name))| bone.id == *id && bone.name == *name)
}

fn shown_bones(skeleton: Option<&Skeleton>) -> Vec<ShownBone> {
    skeleton.map_or_else(Vec::new, |skeleton| {
        skeleton
            .bone_ids
            .iter()
            .zip(skeleton.names.iter())
            .map(|(id, name)| ShownBone {
                id: *id,
                name: *name,
            })
            .collect()
    })
}

fn advanced(time: f32, delta: f32, rate: f32, duration: f32, looping: bool) -> (f32, bool) {
    if duration <= 0.0 {
        return (time, true);
    }
    let next = time + delta * rate;
    if next < duration {
        (next, true)
    } else if looping {
        (next % duration, true)
    } else {
        (duration, false)
    }
}

fn advance_clip(mut preview: ResMut<ClipPlayback>, time: Res<Time>) {
    if !preview.playing {
        return;
    }
    let duration = preview.clip.as_ref().map_or(0.0, |clip| clip.duration());
    let (next, playing) = advanced(
        preview.time,
        time.delta().as_secs_f32(),
        preview.rate,
        duration,
        preview.looping,
    );
    preview.time = next;
    preview.playing = playing;
}

fn pose_clip(
    preview: Res<ClipPlayback>,
    posing: Res<ClipPosing>,
    stores: Option<Res<AssetStore<Skeleton>>>,
    skeletons: Query<&SkeletonComponent>,
    transforms: Query<&mut Transform>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
) {
    let (Some(clip), Some(stores)) = (&preview.clip, stores) else {
        return;
    };
    for component in skeletons.iter() {
        let Some(root) = component.root() else {
            continue;
        };
        let Some(skeleton) = stores.get(component.skeleton()) else {
            continue;
        };
        let bones = component
            .bones()
            .iter()
            .zip(&skeleton.bone_ids)
            .zip(&skeleton.rest_pose);
        for ((bone, id), rest) in bones {
            let Some(channels) = clip.get_channels(id) else {
                continue;
            };
            let Some(entity) = bone.entity() else {
                continue;
            };
            let Some(mut transform) = transforms.get_entity(entity) else {
                continue;
            };
            let mut pose = rest.clone();
            for channel in channels {
                channel.sample_transform(preview.time, &mut pose);
            }
            if entity == root && !posing.root_motion {
                let up = parents
                    .get_entity(root)
                    .and_then(|parent| globals.get_entity(parent.parent()))
                    .map_or(Vec3::Y, |parent| parent.rotation().inverse() * Vec3::Y);
                let lifted = (pose.translation - rest.translation).dot(up);
                pose.translation = rest.translation + up * lifted;
            }
            transform.translation = pose.translation;
            transform.rotation = pose.rotation;
            transform.scale = pose.scale;
        }
    }
}

pub(crate) fn first_rooted<'a>(
    skeletons: impl Iterator<Item = (Entity, &'a SkeletonComponent)>,
) -> Option<&'a SkeletonComponent> {
    skeletons
        .filter(|(_, skeleton)| skeleton.root().is_some())
        .min_by_key(|(entity, _)| entity.index())
        .map(|(_, skeleton)| skeleton)
}

#[derive(Default)]
struct FollowedRoot {
    last: Option<(Entity, Vec3)>,
}

impl FromWorld for FollowedRoot {
    fn from_world(_world: &World) -> Self {
        Self::default()
    }
}

fn middle_of_bones(
    skeleton: &SkeletonComponent,
    globals: &Query<&GlobalTransform>,
) -> Option<Vec3> {
    let mut joints = skeleton
        .bones()
        .iter()
        .filter_map(|bone| bone.entity())
        .filter_map(|bone| Some(globals.get_entity(bone)?.translation()));
    let first = joints.next()?;
    let (min, max) = joints.fold((first, first), |(min, max), joint| {
        (min.min(joint), max.max(joint))
    });
    Some((min + max) * 0.5)
}

fn follow_root(
    skeletons: Query<(Entity, &SkeletonComponent)>,
    globals: Query<&GlobalTransform>,
    cameras: Query<&mut OrbitCamera>,
    mut followed: SystemLocal<FollowedRoot>,
) {
    let skeleton = first_rooted(skeletons.iter());
    let found = skeleton
        .and_then(SkeletonComponent::root)
        .and_then(|root| Some((root, globals.get_entity(root)?.translation())));
    let last = std::mem::replace(&mut followed.last, found);
    let Some((root, at)) = found else {
        return;
    };
    for mut orbit in cameras.iter() {
        match last {
            Some((followed_root, from)) if followed_root == root => {
                let moved = at - from;
                orbit.target += moved - Vec3::Y * moved.dot(Vec3::Y);
            }
            _ => {
                orbit.target = skeleton
                    .and_then(|skeleton| middle_of_bones(skeleton, &globals))
                    .unwrap_or(at)
            }
        }
    }
}

#[derive(Default)]
struct ReportedProgress(Option<ClipPreviewProgress>);

impl FromWorld for ReportedProgress {
    fn from_world(_world: &World) -> Self {
        Self::default()
    }
}

fn report_progress(
    preview: Res<ClipPlayback>,
    mut progress_events: EventWriter<ClipPreviewProgress>,
    mut reported: SystemLocal<ReportedProgress>,
) {
    let progress = ClipPreviewProgress {
        time: preview.time,
        playing: preview.playing,
    };
    if reported.0.as_ref() != Some(&progress) {
        reported.0 = Some(progress.clone());
        progress_events.write(progress);
    }
}

#[derive(Default)]
struct ReportedBones(Option<ClipPreviewBones>);

impl FromWorld for ReportedBones {
    fn from_world(_world: &World) -> Self {
        Self::default()
    }
}

fn report_bones(
    preview: Res<ClipPlayback>,
    stores: Option<Res<AssetStore<Skeleton>>>,
    skeletons: Query<(Entity, &SkeletonComponent)>,
    mut bones_events: EventWriter<ClipPreviewBones>,
    mut reported: SystemLocal<ReportedBones>,
) {
    let shown = first_rooted(skeletons.iter());
    let skeleton = shown
        .zip(stores.as_ref())
        .and_then(|(component, stores)| stores.get(component.skeleton()));
    let driven = preview
        .clip
        .as_ref()
        .zip(skeleton)
        .map_or(0, |(clip, skeleton)| {
            skeleton
                .bone_ids
                .iter()
                .filter(|id| clip.get_channels(id).is_some())
                .count()
        });
    let count = shown.map_or(0, |component| component.bones().len());
    let unchanged = reported.0.as_ref().is_some_and(|last| {
        last.bones == count && last.driven == driven && shows_skeleton(&last.skeleton, skeleton)
    });
    if !unchanged {
        let bones = ClipPreviewBones {
            bones: count,
            driven,
            skeleton: shown_bones(skeleton),
        };
        reported.0 = Some(bones.clone());
        bones_events.write(bones);
    }
}

fn apply_skeleton_gizmo(cmd: &mut CommandQueue, skeleton: Entity, shown: bool) {
    if shown {
        cmd.insert(SkeletonGizmo::default(), skeleton);
    } else {
        cmd.remove::<SkeletonGizmo>(skeleton);
    }
}

fn gizmo_new_skeletons(
    posing: Res<ClipPosing>,
    skeletons: Query<(Entity, &SkeletonComponent), Added<SkeletonComponent>>,
    mut cmd: CommandQueue,
) {
    for (entity, skeleton) in skeletons.iter() {
        if skeleton.root().is_some() {
            apply_skeleton_gizmo(&mut cmd, entity, posing.skeleton_gizmo);
        }
    }
}

fn show_clip(on: On<ShowClip>, mut preview: ResMut<ClipPlayback>) {
    preview.clip = Some(Arc::clone(&on.signal().0));
    preview.time = 0.0;
}

fn clear_clip(_: On<ClearClip>, mut preview: ResMut<ClipPlayback>) {
    preview.clip = None;
    preview.time = 0.0;
}

fn show_clip_scene(
    on: On<ShowClipScene>,
    roots: Query<Entity, With<ClipPreviewRoot>>,
    mut cmd: CommandQueue,
) {
    for root in roots.iter() {
        cmd.despawn(root);
    }
    if let Some(scene) = &on.signal().0 {
        let root = cmd.spawn((Transform::IDENTITY, ClipPreviewRoot)).entity();
        spawn_scene(&mut cmd, scene, root, &SpawnFilter::default());
    }
}

fn set_clip_transport(on: On<SetClipTransport>, mut preview: ResMut<ClipPlayback>) {
    let transport = on.signal();
    preview.playing = transport.playing;
    preview.looping = transport.looping;
    preview.rate = transport.rate;
}

fn seek_clip(on: On<SeekClip>, mut preview: ResMut<ClipPlayback>) {
    preview.time = on.signal().0;
}

fn set_root_motion(on: On<SetRootMotion>, mut posing: ResMut<ClipPosing>) {
    posing.root_motion = on.signal().0;
}

fn set_skeleton_gizmo(
    on: On<SetSkeletonGizmo>,
    mut posing: ResMut<ClipPosing>,
    skeletons: Query<(Entity, &SkeletonComponent)>,
    mut cmd: CommandQueue,
) {
    let shown = on.signal().0;
    posing.skeleton_gizmo = shown;
    for (entity, skeleton) in skeletons.iter() {
        if skeleton.root().is_some() {
            apply_skeleton_gizmo(&mut cmd, entity, shown);
        }
    }
}

/// Adds clip playback to `app`: it plays the clip [`ShowClip`] gives it and reports [`ClipPreviewProgress`], posing nothing.
pub fn clip_playback(app: &mut App) {
    app.insert_resource(ClipPlayback::default())
        .register_event::<ClipPreviewProgress>()
        .add_system(Update, advance_clip.at_preview_time())
        .add_system(LateUpdate, report_progress.at_preview_time())
        .add_listener(show_clip.at_preview_time())
        .add_listener(clear_clip.at_preview_time())
        .add_listener(set_clip_transport.at_preview_time())
        .add_listener(seek_clip.at_preview_time());
}

/// Adds the stage a preview plays on to `app`: the scene [`ShowClipScene`] gives it, the bones [`SetSkeletonGizmo`] draws, and the orbit camera following the first rooted skeleton.
pub fn preview_stage(app: &mut App) {
    if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
        app.register_plugin(DebugGizmosPlugin);
    }
    app.insert_resource(ClipPosing::default())
        .add_system(Update, gizmo_new_skeletons.at_preview_time())
        .add_system(
            LateUpdate,
            follow_root
                .after(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_listener(show_clip_scene.at_preview_time())
        .add_listener(set_skeleton_gizmo.at_preview_time());
}

/// Adds a clip preview to `app`: [`preview_stage`], and every rooted skeleton posed from the clip [`ShowClip`] gives it.
pub fn clip_preview(app: &mut App) {
    clip_playback(app);
    preview_stage(app);
    app.register_event::<ClipPreviewBones>()
        .add_system(
            LateUpdate,
            pose_clip
                .before(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_system(LateUpdate, report_bones.after(pose_clip).at_preview_time())
        .add_listener(set_root_motion.at_preview_time());
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel};
    use concerto_app::plugins::TransformPlugin;
    use concerto_ecs::{
        Entity, World,
        component::scene::SceneEntityRef,
        events::{event_channel::EventChannel, event_cursor::EventCursor},
    };
    use concerto_editor::{
        hosted::testing::plain_recipe,
        preview::{OrbitCamera, PreviewPlugin},
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        transform::Transform,
    };
    use concerto_mesh::{
        SkeletonGizmo,
        skeleton::{JointPose, Skeleton, SkeletonComponent},
    };
    use glam::{Mat4, Quat, Vec3};
    use std::f32::consts::FRAC_PI_2;
    use uuid::Uuid;

    const ROOT: Uuid = Uuid::from_u128(1);
    const DRIVEN: Uuid = Uuid::from_u128(2);
    const STILL: Uuid = Uuid::from_u128(3);
    const ROOT_REST: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    const ROOT_END: Vec3 = Vec3::new(4.0, 1.5, -2.0);
    const DRIVEN_REST: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    const DRIVEN_END: Vec3 = Vec3::new(2.0, 0.5, 0.0);
    const STILL_AT: Vec3 = Vec3::new(7.0, 8.0, 9.0);
    const NEAR: f32 = 1e-4;

    struct Advance {
        time: f32,
        delta: f32,
        rate: f32,
        duration: f32,
        looping: bool,
        next: f32,
        playing: bool,
    }

    #[test]
    fn advancing_scales_by_the_rate_wraps_while_looping_and_stops_at_the_end_otherwise() {
        let rows = [
            Advance {
                time: 1.0,
                delta: 0.5,
                rate: 2.0,
                duration: 10.0,
                looping: false,
                next: 2.0,
                playing: true,
            },
            Advance {
                time: 1.5,
                delta: 1.0,
                rate: 1.0,
                duration: 2.0,
                looping: true,
                next: 0.5,
                playing: true,
            },
            Advance {
                time: 1.5,
                delta: 1.0,
                rate: 1.0,
                duration: 2.0,
                looping: false,
                next: 2.0,
                playing: false,
            },
            Advance {
                time: 0.0,
                delta: 1.0,
                rate: 1.0,
                duration: 0.0,
                looping: true,
                next: 0.0,
                playing: true,
            },
        ];
        for row in rows {
            assert_eq!(
                advanced(row.time, row.delta, row.rate, row.duration, row.looping),
                (row.next, row.playing),
                "{} + {} at {}x of {}",
                row.time,
                row.delta,
                row.rate,
                row.duration
            );
        }
    }

    fn keys(from: Vec3, to: Vec3) -> AnimationChannel {
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::from_translation([from.to_array(), to.to_array()].into_iter()),
        )
    }

    fn turned() -> Quat {
        Quat::from_rotation_y(FRAC_PI_2)
    }

    fn clip() -> Arc<AnimationClip> {
        let mut clip = AnimationClip::default();
        clip.add_channel(ROOT, keys(ROOT_REST, ROOT_END));
        clip.add_channel(
            ROOT,
            AnimationChannel::new(
                vec![0.0, 2.0],
                AnimationChanelOutput::from_rotation(
                    [Quat::IDENTITY.to_array(), turned().to_array()].into_iter(),
                ),
            ),
        );
        clip.add_channel(DRIVEN, keys(DRIVEN_REST, DRIVEN_END));
        Arc::new(clip)
    }

    fn skeleton() -> Skeleton {
        let rest = |translation| JointPose {
            translation,
            ..JointPose::IDENTITY
        };
        Skeleton::new(
            vec![Mat4::IDENTITY; 3],
            vec![None, Some(0), Some(1)],
            vec!["root".into(), "driven".into(), "still".into()],
            vec![rest(ROOT_REST), rest(DRIVEN_REST), rest(Vec3::ZERO)],
            vec![ROOT, DRIVEN, STILL],
        )
        .unwrap()
    }

    struct Rig {
        owner: Entity,
        root: Entity,
        driven: Entity,
        still: Entity,
    }

    fn spawn_rig(world: &mut World) -> Rig {
        let id = AssetId::new();
        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(id, skeleton());
        let parent = world.spawn(Transform::IDENTITY);
        let root = world.spawn(Transform::from_translation(ROOT_REST));
        let driven = world.spawn(Transform::from_translation(DRIVEN_REST));
        let still = world.spawn(Transform::from_translation(STILL_AT));
        world.entity_mut(parent).add_child(root);
        world.entity_mut(root).add_child(driven);
        world.entity_mut(driven).add_child(still);
        let owner = world.spawn(SkeletonComponent::new(
            AssetHandle::weak(id),
            [root, driven, still].map(SceneEntityRef::Entity).to_vec(),
            Some(SceneEntityRef::Entity(root)),
        ));
        Rig {
            owner,
            root,
            driven,
            still,
        }
    }

    fn transform(app: &App, entity: Entity) -> Transform {
        app.main()
            .world()
            .get_component_for_entity::<Transform>(entity)
            .unwrap()
            .clone()
    }

    fn camera_target(app: &mut App) -> Vec3 {
        let world = app.main_mut().world_mut();
        world
            .query::<&OrbitCamera, ()>()
            .iter(world)
            .next()
            .unwrap()
            .target
    }

    fn written<E: Event + Clone + 'static>(app: &App) -> Vec<E> {
        app.get_resource::<EventChannel<E>>()
            .unwrap()
            .read_from(&mut EventCursor::default())
            .cloned()
            .collect()
    }

    #[test]
    fn a_seek_poses_the_clip_with_root_motion_off_and_on_and_the_camera_follows_the_root() {
        let mut app = App::for_previewing();
        plain_recipe(&mut app);
        app.register_plugin(TransformPlugin)
            .register_plugin(PreviewPlugin);
        clip_preview(&mut app);
        app.insert_resource(AssetStore::<Skeleton>::new());
        app.finish_plugin_build();
        app.update();
        let world = app.main_mut().world_mut();
        let rig = spawn_rig(world);
        world.trigger(ShowClip(clip()));
        world.trigger(SetClipTransport {
            playing: false,
            looping: true,
            rate: 1.0,
        });
        world.trigger(SeekClip(2.0));

        app.update();

        assert!(
            transform(&app, rig.driven)
                .translation
                .abs_diff_eq(DRIVEN_END, NEAR)
        );
        assert_eq!(transform(&app, rig.still).translation, STILL_AT);
        let held = transform(&app, rig.root);
        assert!(
            held.translation
                .abs_diff_eq(Vec3::new(ROOT_REST.x, ROOT_END.y, ROOT_REST.z), NEAR),
            "{}",
            held.translation
        );
        assert!(held.rotation.abs_diff_eq(turned(), NEAR));
        assert!(
            app.main()
                .world()
                .get_component_for_entity::<SkeletonGizmo>(rig.owner)
                .is_some()
        );
        let progress = written::<ClipPreviewProgress>(&app);
        assert_eq!(
            progress.last(),
            Some(&ClipPreviewProgress {
                time: 2.0,
                playing: false
            })
        );
        let bones = written::<ClipPreviewBones>(&app);
        let shown = bones.last().expect("the preview reports its bones");
        assert_eq!(shown.bones, 3);
        assert_eq!(shown.driven, 2);
        let before = camera_target(&mut app);

        app.main_mut().world_mut().trigger(SetRootMotion(true));
        app.update();

        assert!(
            transform(&app, rig.root)
                .translation
                .abs_diff_eq(ROOT_END, NEAR)
        );
        let moved = ROOT_END - ROOT_REST;
        assert!(
            camera_target(&mut app).abs_diff_eq(before + Vec3::new(moved.x, 0.0, moved.z), NEAR),
            "{} from {before}",
            camera_target(&mut app)
        );
    }
}

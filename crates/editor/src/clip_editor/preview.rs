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
use concerto_foundation::{
    assets::asset_store::AssetStore,
    time::Time,
    transform::{GlobalTransform, Transform, systems::propagate_global_transforms},
};
use concerto_mesh::{
    SkeletonGizmo,
    skeleton::{Skeleton, SkeletonComponent},
};
use concerto_render::plugin::RenderPlugin;
use concerto_scene::{scene::Scene, spawner::spawn_scene};
use glam::Vec3;

use crate::preview::OrbitCamera;

#[derive(Resource)]
pub(super) struct ClipPreview {
    clip: Option<Arc<AnimationClip>>,
    pub(super) time: f32,
    pub(super) rate: f32,
    playing: bool,
    pub(super) looping: bool,
    pub(super) root_motion: bool,
    pub(super) skeleton_gizmo: bool,
}

impl Default for ClipPreview {
    fn default() -> Self {
        Self {
            clip: None,
            time: 0.0,
            rate: 1.0,
            playing: true,
            looping: true,
            root_motion: false,
            skeleton_gizmo: true,
        }
    }
}

/// The entity a preview's scene is spawned under; despawning it takes the scene with it.
#[derive(Component)]
struct ClipPreviewRoot;

/// Replaces the clip the preview plays and goes back to its start.
pub struct ShowClip(pub Arc<AnimationClip>);
impl Signal for ShowClip {}

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

/// How many bones the shown skeleton has and how many of them the clip drives; written when either changes.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ClipPreviewBones {
    pub bones: usize,
    pub driven: usize,
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

fn advance_clip(mut preview: ResMut<ClipPreview>, time: Res<Time>) {
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
    preview: Res<ClipPreview>,
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
            if entity == root && !preview.root_motion {
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

fn first_rooted<'a>(
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
struct Reported {
    progress: Option<ClipPreviewProgress>,
    bones: Option<ClipPreviewBones>,
}

impl FromWorld for Reported {
    fn from_world(_world: &World) -> Self {
        Self::default()
    }
}

fn report_clip(
    preview: Res<ClipPreview>,
    stores: Option<Res<AssetStore<Skeleton>>>,
    skeletons: Query<(Entity, &SkeletonComponent)>,
    mut progress_events: EventWriter<ClipPreviewProgress>,
    mut bones_events: EventWriter<ClipPreviewBones>,
    mut reported: SystemLocal<Reported>,
) {
    let progress = ClipPreviewProgress {
        time: preview.time,
        playing: preview.playing,
    };
    if reported.progress.as_ref() != Some(&progress) {
        reported.progress = Some(progress.clone());
        progress_events.write(progress);
    }

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
    let bones = ClipPreviewBones {
        bones: shown.map_or(0, |component| component.bones().len()),
        driven,
    };
    if reported.bones.as_ref() != Some(&bones) {
        reported.bones = Some(bones.clone());
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
    preview: Res<ClipPreview>,
    skeletons: Query<(Entity, &SkeletonComponent), Added<SkeletonComponent>>,
    mut cmd: CommandQueue,
) {
    for (entity, skeleton) in skeletons.iter() {
        if skeleton.root().is_some() {
            apply_skeleton_gizmo(&mut cmd, entity, preview.skeleton_gizmo);
        }
    }
}

fn show_clip(on: On<ShowClip>, mut preview: ResMut<ClipPreview>) {
    preview.clip = Some(Arc::clone(&on.signal().0));
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
        spawn_scene(&mut cmd, scene, root);
    }
}

fn set_clip_transport(on: On<SetClipTransport>, mut preview: ResMut<ClipPreview>) {
    let transport = on.signal();
    preview.playing = transport.playing;
    preview.looping = transport.looping;
    preview.rate = transport.rate;
}

fn seek_clip(on: On<SeekClip>, mut preview: ResMut<ClipPreview>) {
    preview.time = on.signal().0;
}

fn set_root_motion(on: On<SetRootMotion>, mut preview: ResMut<ClipPreview>) {
    preview.root_motion = on.signal().0;
}

fn set_skeleton_gizmo(
    on: On<SetSkeletonGizmo>,
    mut preview: ResMut<ClipPreview>,
    skeletons: Query<(Entity, &SkeletonComponent)>,
    mut cmd: CommandQueue,
) {
    let shown = on.signal().0;
    preview.skeleton_gizmo = shown;
    for (entity, skeleton) in skeletons.iter() {
        if skeleton.root().is_some() {
            apply_skeleton_gizmo(&mut cmd, entity, shown);
        }
    }
}

pub(super) fn clip_preview(app: &mut App) {
    if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
        app.register_plugin(DebugGizmosPlugin);
    }
    app.insert_resource(ClipPreview::default())
        .register_event::<ClipPreviewProgress>()
        .register_event::<ClipPreviewBones>()
        .add_system(
            Update,
            (advance_clip, gizmo_new_skeletons).at_preview_time(),
        )
        .add_system(
            LateUpdate,
            pose_clip
                .before(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_system(
            LateUpdate,
            follow_root
                .after(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_system(LateUpdate, report_clip.after(pose_clip).at_preview_time())
        .add_listener(show_clip.at_preview_time())
        .add_listener(show_clip_scene.at_preview_time())
        .add_listener(set_clip_transport.at_preview_time())
        .add_listener(seek_clip.at_preview_time())
        .add_listener(set_root_motion.at_preview_time())
        .add_listener(set_skeleton_gizmo.at_preview_time());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hosted::testing::plain_recipe,
        preview::{OrbitCamera, PreviewPlugin},
    };
    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel};
    use concerto_app::plugins::TransformPlugin;
    use concerto_ecs::{
        Entity, IntoSystem, System, With, World,
        component::{name::Name, scene::SceneEntityRef},
        entity::hierarchy::{ChildOf, Children},
        events::{event_channel::EventChannel, event_cursor::EventCursor},
        system::BoxedSystem,
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        transform::{Transform, systems::propagate_global_transforms},
    };
    use concerto_mesh::{
        SkeletonGizmo,
        skeleton::{JointPose, Skeleton, SkeletonComponent},
    };
    use concerto_scene::scene::SceneNode;
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
    const BONES_MIDDLE: Vec3 = Vec3::new(3.5, 5.25, 4.5);
    const NEAR: f32 = 1e-4;

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

    #[test]
    fn a_document_starts_with_the_previews_values() {
        let document = crate::clip_editor::ClipDocument::default();
        let preview = ClipPreview::default();

        assert_eq!(document.time, preview.time);
        assert_eq!(document.rate, preview.rate);
        assert_eq!(document.playing, preview.playing);
        assert_eq!(document.looping, preview.looping);
        assert_eq!(document.root_motion, preview.root_motion);
        assert_eq!(document.skeleton_gizmo, preview.skeleton_gizmo);
    }

    #[derive(Clone, Copy)]
    struct Rig {
        owner: Entity,
        root: Entity,
        driven: Entity,
        still: Entity,
    }

    fn spawn_rig(world: &mut World, parent: Transform, rooted: bool) -> Rig {
        let id = AssetId::new();
        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(id, skeleton());
        let parent = world.spawn(parent);
        let root = world.spawn(Transform::from_translation(ROOT_REST));
        let driven = world.spawn(Transform::from_translation(DRIVEN_REST));
        let still = world.spawn(Transform::from_translation(STILL_AT));
        world.entity_mut(parent).add_child(root);
        world.entity_mut(root).add_child(driven);
        world.entity_mut(driven).add_child(still);
        let owner = world.spawn(SkeletonComponent::new(
            AssetHandle::weak(id),
            [root, driven, still].map(SceneEntityRef::Entity).to_vec(),
            rooted.then_some(SceneEntityRef::Entity(root)),
        ));
        Rig {
            owner,
            root,
            driven,
            still,
        }
    }

    struct Stage {
        world: World,
        gizmos: BoxedSystem,
        pose: BoxedSystem,
        propagate: BoxedSystem,
        follow: BoxedSystem,
        report: BoxedSystem,
    }

    impl Stage {
        fn new() -> Self {
            let mut world = World::new();
            world.register_component::<SkeletonComponent>();
            world.register_component::<Transform>();
            world.register_component::<ChildOf>();
            world.register_component::<Children>();
            world.insert_resource(ClipPreview::default());
            world.insert_resource(AssetStore::<Skeleton>::new());
            world.insert_resource(EventChannel::<ClipPreviewProgress>::new());
            world.insert_resource(EventChannel::<ClipPreviewBones>::new());
            world.add_listener(show_clip);
            world.add_listener(show_clip_scene);
            world.add_listener(set_clip_transport);
            world.add_listener(seek_clip);
            world.add_listener(set_root_motion);
            world.add_listener(set_skeleton_gizmo);
            let mut gizmos = gizmo_new_skeletons.into_boxed_system();
            let mut pose = pose_clip.into_boxed_system();
            let mut propagate = propagate_global_transforms.into_boxed_system();
            let mut follow = follow_root.into_boxed_system();
            let mut report = report_clip.into_boxed_system();
            for system in [
                &mut gizmos,
                &mut pose,
                &mut propagate,
                &mut follow,
                &mut report,
            ] {
                system.initialize(&mut world);
            }
            Self {
                world,
                gizmos,
                pose,
                propagate,
                follow,
                report,
            }
        }

        fn frame(&mut self) {
            self.gizmos.run_and_apply((), &mut self.world);
            self.pose.run_and_apply((), &mut self.world);
            self.propagate.run_and_apply((), &mut self.world);
            self.follow.run_and_apply((), &mut self.world);
            self.report.run_and_apply((), &mut self.world);
            self.world.tick();
        }

        fn rig(&mut self) -> Rig {
            spawn_rig(&mut self.world, Transform::IDENTITY, true)
        }

        fn show_clip(&mut self) {
            self.world.trigger(ShowClip(clip()));
        }

        fn seek(&mut self, time: f32) {
            self.world.trigger(SeekClip(time));
        }

        fn preview(&self) -> &ClipPreview {
            self.world.get_resource::<ClipPreview>().unwrap()
        }

        fn transform(&self, entity: Entity) -> &Transform {
            self.world
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
        }

        fn camera(&mut self) -> Entity {
            self.world.spawn(OrbitCamera::default())
        }

        fn target(&self, camera: Entity) -> Vec3 {
            self.world
                .get_component_for_entity::<OrbitCamera>(camera)
                .unwrap()
                .target
        }

        fn written<E: Event + Clone + 'static>(&self) -> Vec<E> {
            self.world
                .get_resource::<EventChannel<E>>()
                .unwrap()
                .read_from(&mut EventCursor::default())
                .cloned()
                .collect()
        }

        fn has_gizmo(&self, entity: Entity) -> bool {
            self.world
                .get_component_for_entity::<SkeletonGizmo>(entity)
                .is_some()
        }

        fn roots(&mut self) -> Vec<Entity> {
            self.world
                .query::<Entity, With<ClipPreviewRoot>>()
                .iter(&mut self.world)
                .collect()
        }

        fn names(&mut self) -> Vec<String> {
            self.world
                .query::<&Name, ()>()
                .iter(&mut self.world)
                .map(|name| name.as_str().to_owned())
                .collect()
        }
    }

    fn scene_of(name: &str) -> Scene {
        Scene {
            nodes: vec![SceneNode {
                name: name.into(),
                children: vec![],
                components: vec![],
            }],
            referenced_assets: vec![],
        }
    }

    #[test]
    fn advancing_scales_the_delta_by_the_rate() {
        assert_eq!(advanced(1.0, 0.5, 2.0, 10.0, false), (2.0, true));
        assert_eq!(advanced(1.0, 0.5, 0.5, 10.0, true), (1.25, true));
    }

    #[test]
    fn advancing_past_the_end_wraps_while_looping() {
        assert_eq!(advanced(1.5, 1.0, 1.0, 2.0, true), (0.5, true));
        assert_eq!(advanced(1.0, 1.0, 1.0, 2.0, true), (0.0, true));
    }

    #[test]
    fn advancing_past_the_end_stops_there_when_not_looping() {
        assert_eq!(advanced(1.5, 1.0, 1.0, 2.0, false), (2.0, false));
        assert_eq!(advanced(1.0, 1.0, 1.0, 2.0, false), (2.0, false));
    }

    #[test]
    fn a_clip_with_no_duration_never_advances() {
        assert_eq!(advanced(0.0, 1.0, 1.0, 0.0, true), (0.0, true));
        assert_eq!(advanced(0.0, 1.0, 1.0, 0.0, false), (0.0, true));
    }

    #[test]
    fn a_seek_poses_a_driven_bone_exactly_at_that_time() {
        let mut stage = Stage::new();
        let rig = stage.rig();
        stage.show_clip();

        stage.seek(0.5);
        stage.frame();

        assert!(
            stage
                .transform(rig.driven)
                .translation
                .abs_diff_eq(DRIVEN_REST.lerp(DRIVEN_END, 0.25), NEAR)
        );
    }

    #[test]
    fn a_bone_without_channels_keeps_its_transform() {
        let mut stage = Stage::new();
        let rig = stage.rig();
        stage.show_clip();

        stage.seek(1.0);
        stage.frame();

        assert_eq!(stage.transform(rig.still).translation, STILL_AT);
    }

    #[test]
    fn a_rootless_skeleton_is_skipped() {
        let mut stage = Stage::new();
        let rig = spawn_rig(&mut stage.world, Transform::IDENTITY, false);
        stage.show_clip();

        stage.seek(1.0);
        stage.frame();

        assert_eq!(stage.transform(rig.driven).translation, DRIVEN_REST);
        assert_eq!(stage.transform(rig.root).translation, ROOT_REST);
    }

    #[test]
    fn nothing_is_posed_before_a_clip_arrives() {
        let mut stage = Stage::new();
        let rig = stage.rig();

        stage.frame();

        assert_eq!(stage.transform(rig.driven).translation, DRIVEN_REST);
    }

    #[test]
    fn the_root_takes_the_clips_rotation_with_root_motion_on_and_off() {
        let mut stage = Stage::new();
        let rig = stage.rig();
        stage.show_clip();
        stage.seek(2.0);

        stage.frame();
        assert!(
            stage
                .transform(rig.root)
                .rotation
                .abs_diff_eq(turned(), NEAR)
        );

        stage.world.trigger(SetRootMotion(true));
        stage.frame();
        assert!(
            stage
                .transform(rig.root)
                .rotation
                .abs_diff_eq(turned(), NEAR)
        );
    }

    #[test]
    fn with_root_motion_off_the_root_keeps_its_horizontal_position_and_the_clips_height() {
        let mut stage = Stage::new();
        let rig = stage.rig();
        stage.show_clip();

        stage.seek(2.0);
        stage.frame();

        assert!(
            stage
                .transform(rig.root)
                .translation
                .abs_diff_eq(Vec3::new(ROOT_REST.x, ROOT_END.y, ROOT_REST.z), NEAR)
        );
    }

    #[test]
    fn with_root_motion_on_the_root_takes_the_sampled_translation() {
        let mut stage = Stage::new();
        let rig = stage.rig();
        stage.show_clip();
        stage.world.trigger(SetRootMotion(true));

        stage.seek(2.0);
        stage.frame();

        assert!(
            stage
                .transform(rig.root)
                .translation
                .abs_diff_eq(ROOT_END, NEAR)
        );
    }

    #[test]
    fn with_root_motion_off_under_a_rotated_parent_the_root_keeps_height_along_world_up() {
        let mut stage = Stage::new();
        let parent = Transform::from_rotation(Quat::from_rotation_x(FRAC_PI_2));
        let rig = spawn_rig(&mut stage.world, parent, true);
        stage.show_clip();

        stage.seek(2.0);
        stage.frame();

        assert!(
            stage
                .transform(rig.root)
                .translation
                .abs_diff_eq(Vec3::new(0.0, 1.0, -2.0), NEAR)
        );
    }

    #[test]
    fn the_first_frame_with_a_root_targets_the_middle_of_its_bones() {
        let mut stage = Stage::new();
        let camera = stage.camera();
        stage.rig();

        stage.frame();

        assert!(stage.target(camera).abs_diff_eq(BONES_MIDDLE, NEAR));
    }

    #[test]
    fn horizontal_root_movement_moves_the_target() {
        let mut stage = Stage::new();
        let camera = stage.camera();
        stage.rig();
        stage.show_clip();
        stage.world.trigger(SetRootMotion(true));
        stage.frame();

        stage.seek(2.0);
        stage.frame();

        let moved = ROOT_END - ROOT_REST;
        assert!(
            stage
                .target(camera)
                .abs_diff_eq(BONES_MIDDLE + Vec3::new(moved.x, 0.0, moved.z), NEAR)
        );
    }

    #[test]
    fn root_movement_along_up_leaves_the_target_alone() {
        let mut stage = Stage::new();
        let camera = stage.camera();
        stage.rig();
        stage.show_clip();
        stage.frame();

        stage.seek(2.0);
        stage.frame();

        assert!(stage.target(camera).abs_diff_eq(BONES_MIDDLE, NEAR));
    }

    #[test]
    fn turning_root_motion_off_brings_the_target_back() {
        let mut stage = Stage::new();
        let camera = stage.camera();
        stage.rig();
        stage.show_clip();
        stage.world.trigger(SetRootMotion(true));
        stage.frame();
        stage.seek(2.0);
        stage.frame();

        stage.world.trigger(SetRootMotion(false));
        stage.frame();

        assert!(stage.target(camera).abs_diff_eq(BONES_MIDDLE, NEAR));
    }

    #[test]
    fn a_new_scenes_root_is_targeted_rather_than_followed() {
        let mut stage = Stage::new();
        let camera = stage.camera();
        let first = stage.rig();
        stage.frame();
        stage.world.despawn(first.owner);
        let parent = Transform::from_translation(Vec3::new(5.0, 0.0, 0.0));
        spawn_rig(&mut stage.world, parent, true);

        stage.frame();

        assert!(
            stage
                .target(camera)
                .abs_diff_eq(BONES_MIDDLE + Vec3::new(5.0, 0.0, 0.0), NEAR)
        );
    }

    #[test]
    fn progress_is_reported_only_when_it_changes() {
        let mut stage = Stage::new();
        stage.frame();
        stage.frame();
        assert_eq!(
            stage.written::<ClipPreviewProgress>(),
            [ClipPreviewProgress {
                time: 0.0,
                playing: true
            }]
        );

        stage.seek(1.0);
        stage.frame();
        stage.frame();
        stage.world.trigger(SetClipTransport {
            playing: false,
            looping: true,
            rate: 1.0,
        });
        stage.frame();

        assert_eq!(
            stage.written::<ClipPreviewProgress>()[1..],
            [
                ClipPreviewProgress {
                    time: 1.0,
                    playing: true
                },
                ClipPreviewProgress {
                    time: 1.0,
                    playing: false
                }
            ]
        );
    }

    #[test]
    fn no_skeleton_reports_no_bones() {
        let mut stage = Stage::new();

        stage.frame();

        assert_eq!(
            stage.written::<ClipPreviewBones>(),
            [ClipPreviewBones {
                bones: 0,
                driven: 0
            }]
        );
    }

    #[test]
    fn the_bones_are_reported_once_with_the_driven_count_of_the_first_rooted_skeleton() {
        let mut stage = Stage::new();
        spawn_rig(&mut stage.world, Transform::IDENTITY, false);
        stage.frame();
        stage.rig();
        stage.show_clip();
        stage.frame();
        stage.frame();

        assert_eq!(
            stage.written::<ClipPreviewBones>(),
            [
                ClipPreviewBones {
                    bones: 0,
                    driven: 0
                },
                ClipPreviewBones {
                    bones: 3,
                    driven: 2
                }
            ]
        );
    }

    #[test]
    fn showing_a_clip_replaces_it_and_goes_back_to_the_start() {
        let mut stage = Stage::new();
        stage.seek(1.5);

        let shown = clip();
        stage.world.trigger(ShowClip(Arc::clone(&shown)));

        assert_eq!(stage.preview().time, 0.0);
        assert!(Arc::ptr_eq(stage.preview().clip.as_ref().unwrap(), &shown));
    }

    #[test]
    fn a_seek_moves_the_time() {
        let mut stage = Stage::new();

        stage.seek(1.25);

        assert_eq!(stage.preview().time, 1.25);
    }

    #[test]
    fn a_transport_signal_sets_playing_looping_and_rate() {
        let mut stage = Stage::new();

        stage.world.trigger(SetClipTransport {
            playing: false,
            looping: false,
            rate: 0.25,
        });

        let preview = stage.preview();
        assert_eq!(
            (preview.playing, preview.looping, preview.rate),
            (false, false, 0.25)
        );
    }

    #[test]
    fn the_gizmo_signal_changes_only_the_rooted_skeletons_that_exist() {
        let mut stage = Stage::new();
        let rooted = stage.rig();
        let rootless = spawn_rig(&mut stage.world, Transform::IDENTITY, false);

        stage.world.trigger(SetSkeletonGizmo(true));
        assert!(stage.has_gizmo(rooted.owner));
        assert!(!stage.has_gizmo(rootless.owner));

        stage.world.trigger(SetSkeletonGizmo(false));
        assert!(!stage.has_gizmo(rooted.owner));
    }

    #[test]
    fn a_skeleton_that_appears_later_follows_the_gizmo_setting() {
        let mut stage = Stage::new();
        let shown = stage.rig();
        let rootless = spawn_rig(&mut stage.world, Transform::IDENTITY, false);
        stage.frame();
        assert!(stage.has_gizmo(shown.owner));
        assert!(!stage.has_gizmo(rootless.owner));

        stage.world.trigger(SetSkeletonGizmo(false));
        let hidden = stage.rig();
        stage.world.insert(SkeletonGizmo::default(), hidden.owner);
        stage.frame();

        assert!(!stage.has_gizmo(hidden.owner));
    }

    #[test]
    fn a_second_scene_despawns_the_first_and_its_root_carries_a_transform() {
        let mut stage = Stage::new();
        stage.world.trigger(ShowClipScene(Some(scene_of("first"))));
        let first = stage.roots();

        stage.world.trigger(ShowClipScene(Some(scene_of("second"))));

        assert_eq!(stage.names(), ["second"]);
        let roots = stage.roots();
        assert_eq!(roots.len(), 1);
        assert_ne!(roots, first);
        assert_eq!(stage.transform(roots[0]).translation, Vec3::ZERO);
        let children = stage
            .world
            .get_component_for_entity::<Children>(roots[0])
            .unwrap();
        assert_eq!(children.iter().len(), 1);
    }

    #[test]
    fn no_scene_empties_the_stage() {
        let mut stage = Stage::new();
        stage.world.trigger(ShowClipScene(Some(scene_of("first"))));

        stage.world.trigger(ShowClipScene(None));

        assert!(stage.names().is_empty());
        assert!(stage.roots().is_empty());
    }

    #[test]
    fn a_preview_app_runs_every_clip_system_in_its_schedule() {
        let mut app = App::for_previewing();
        plain_recipe(&mut app);
        app.register_plugin(TransformPlugin)
            .register_plugin(PreviewPlugin);
        clip_preview(&mut app);
        app.insert_resource(AssetStore::<Skeleton>::new());
        app.finish_plugin_build();
        app.update();
        let world = app.main_mut().world_mut();
        let rig = spawn_rig(world, Transform::IDENTITY, true);
        world.trigger(ShowClip(clip()));

        let mut cursor = EventCursor::default();
        let mut bones = Vec::new();
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            app.update();
            let channel = app
                .get_resource::<EventChannel<ClipPreviewBones>>()
                .unwrap();
            bones.extend(channel.read_from(&mut cursor).cloned());
        }

        let world = app.main_mut().world_mut();
        assert!(world.get_resource::<ClipPreview>().unwrap().time > 0.0);
        assert!(
            world
                .get_component_for_entity::<Transform>(rig.driven)
                .unwrap()
                .translation
                .x
                > 0.0
        );
        assert!(
            world
                .get_component_for_entity::<SkeletonGizmo>(rig.owner)
                .is_some()
        );
        let orbit = world
            .query::<&OrbitCamera, ()>()
            .iter(world)
            .next()
            .unwrap();
        assert!(orbit.target.abs_diff_eq(BONES_MIDDLE, 0.1));
        assert!(bones.contains(&ClipPreviewBones {
            bones: 3,
            driven: 2
        }));
    }
}

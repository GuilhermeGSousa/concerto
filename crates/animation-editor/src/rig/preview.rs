//! Everything that runs in a rig editor's preview `App`: the source and target characters side by side, standing in their reference poses or playing a clip through the retarget.
use std::sync::Arc;

use concerto_animation::retarget::PoseRetarget;
use concerto_app::{
    App,
    edit_time::AtPreviewTime,
    schedule_groups::{LateUpdate, Update},
};
use concerto_color::Color;
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
use concerto_ecs::{
    Added, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    component::name::Name,
    entity::hierarchy::ChildOf,
    signal::{On, Signal},
    system::input::SystemLocal,
    world::FromWorld,
};
use concerto_editor::preview::OrbitCamera;
use concerto_foundation::{
    assets::AssetId,
    transform::{GlobalTransform, Transform, systems::propagate_global_transforms},
};
use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
use concerto_reflect::Reflect;
use concerto_render::plugin::RenderPlugin;
use concerto_scene::{
    scene::Scene,
    spawner::{SpawnedScene, spawn_scene},
};
use glam::Vec3;

use crate::preview::{ClipPlayback, clip_playback};

/// Which side of the retarget a preview stage shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RigSide {
    Source,
    Target,
}

/// Whether the preview stands both characters in their reference poses or plays the clip through the retarget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum RigPreviewMode {
    #[default]
    Reference,
    Clip,
}

/// What one side of the preview shows: a skeleton, and the scene that carries it when there is one.
#[derive(Clone)]
pub struct RigStage {
    pub scene: Option<Scene>,
    pub skeleton: Arc<Skeleton>,
    pub skeleton_id: AssetId,
}

impl RigStage {
    /// A stage showing `skeleton`, the asset `skeleton_id`, as bones alone.
    pub fn new(skeleton: Arc<Skeleton>, skeleton_id: AssetId) -> Self {
        Self {
            scene: None,
            skeleton,
            skeleton_id,
        }
    }

    /// Shows `scene`, whose skeleton is posed when it is `skeleton_id`.
    pub fn with_scene(mut self, scene: Scene) -> Self {
        self.scene = Some(scene);
        self
    }
}

/// Replaces what one side of the preview shows; `None` leaves that side empty.
pub struct ShowRigStage {
    pub side: RigSide,
    pub stage: Option<RigStage>,
}
impl Signal for ShowRigStage {}

/// Sets where along X each side stands, without spawning either again.
pub struct PlaceRigSides {
    pub source: f32,
    pub target: f32,
}
impl Signal for PlaceRigSides {}

/// Sets a side's reference pose, `None` being the rest pose, its root bone, and each bone's colour in skeleton order.
pub struct SetRigSide {
    pub side: RigSide,
    pub reference: Option<Vec<JointPose>>,
    pub root: Option<usize>,
    pub colors: Vec<Color>,
}
impl Signal for SetRigSide {}

impl SetRigSide {
    /// A side standing in its rest pose, with no root bone and no colours.
    pub fn new(side: RigSide) -> Self {
        Self {
            side,
            reference: None,
            root: None,
            colors: Vec::new(),
        }
    }

    /// Sets the reference pose, in skeleton order.
    pub fn with_reference(mut self, reference: Vec<JointPose>) -> Self {
        self.reference = Some(reference);
        self
    }

    /// Sets the root bone, by index in the skeleton.
    pub fn with_root(mut self, root: usize) -> Self {
        self.root = Some(root);
        self
    }

    /// Adds the colour of the next bone.
    pub fn with_color(mut self, color: Color) -> Self {
        self.colors.push(color);
        self
    }
}

/// Sets the retarget that poses the target from the source; `None` leaves the target in its reference pose.
pub struct SetPoseRetarget(pub Option<PoseRetarget>);
impl Signal for SetPoseRetarget {}

/// Sets what the preview poses, whether the characters' roots move, and whether their bones are drawn.
pub struct SetRigPreview {
    pub mode: RigPreviewMode,
    pub root_motion: bool,
    pub bones: bool,
}
impl Signal for SetRigPreview {}

const BONE_GREY: Color = Color::srgba(0.6, 0.6, 0.6, 1.0);
const JOINT_SIZE: f32 = 0.02;

struct FoundBones {
    entities: Vec<Entity>,
    model_space: Entity,
}

struct ShownStage {
    skeleton: Arc<Skeleton>,
    skeleton_id: AssetId,
    root: Entity,
    scene: Option<SpawnedScene>,
    bones: Option<FoundBones>,
}

#[derive(Default)]
struct ShownSide {
    stage: Option<ShownStage>,
    offset: f32,
    reference: Option<Vec<JointPose>>,
    root: Option<usize>,
    colors: Vec<Color>,
}

impl ShownSide {
    fn reference_pose(&self, skeleton: &Skeleton) -> Vec<JointPose> {
        self.reference
            .as_ref()
            .filter(|reference| reference.len() == skeleton.bone_count())
            .cloned()
            .unwrap_or_else(|| skeleton.rest_pose.to_vec())
    }

    fn root_bone(&self) -> Option<Entity> {
        let bones = self.stage.as_ref()?.bones.as_ref()?;
        bones.entities.get(self.root.unwrap_or(0)).copied()
    }
}

#[derive(Resource, Default)]
pub(crate) struct RigSides {
    source: ShownSide,
    target: ShownSide,
}

impl RigSides {
    fn side_mut(&mut self, side: RigSide) -> &mut ShownSide {
        match side {
            RigSide::Source => &mut self.source,
            RigSide::Target => &mut self.target,
        }
    }
}

#[cfg(test)]
impl RigSides {
    fn side(&self, side: RigSide) -> &ShownSide {
        match side {
            RigSide::Source => &self.source,
            RigSide::Target => &self.target,
        }
    }

    pub(crate) fn skeleton_of(&self, side: RigSide) -> Option<AssetId> {
        Some(self.side(side).stage.as_ref()?.skeleton_id)
    }

    pub(crate) fn root_of(&self, side: RigSide) -> Option<Entity> {
        Some(self.side(side).stage.as_ref()?.root)
    }

    pub(crate) fn offset_of(&self, side: RigSide) -> f32 {
        self.side(side).offset
    }

    pub(crate) fn root_bone_of(&self, side: RigSide) -> Option<usize> {
        self.side(side).root
    }

    pub(crate) fn colors_of(&self, side: RigSide) -> &[Color] {
        &self.side(side).colors
    }
}

#[cfg(test)]
impl RigPosing {
    pub(crate) fn retargets(&self) -> bool {
        self.retarget.is_some()
    }

    pub(crate) fn mode(&self) -> RigPreviewMode {
        self.mode
    }
}

#[derive(Resource)]
pub(crate) struct RigPosing {
    mode: RigPreviewMode,
    root_motion: bool,
    bones: bool,
    retarget: Option<PoseRetarget>,
}

impl Default for RigPosing {
    fn default() -> Self {
        Self {
            mode: RigPreviewMode::Reference,
            root_motion: false,
            bones: true,
            retarget: None,
        }
    }
}

fn show_rig_stage(on: On<ShowRigStage>, mut sides: ResMut<RigSides>, mut cmd: CommandQueue) {
    let signal = on.signal();
    let side = sides.side_mut(signal.side);
    if let Some(shown) = side.stage.take() {
        cmd.despawn(shown.root);
    }
    let Some(stage) = &signal.stage else {
        return;
    };
    let root = cmd
        .spawn(Transform::from_translation(Vec3::X * side.offset))
        .entity();
    let scene = stage
        .scene
        .as_ref()
        .map(|scene| spawn_scene(&mut cmd, scene, root));
    side.stage = Some(ShownStage {
        skeleton: Arc::clone(&stage.skeleton),
        skeleton_id: stage.skeleton_id,
        root,
        scene,
        bones: None,
    });
}

fn place_rig_sides(
    on: On<PlaceRigSides>,
    mut sides: ResMut<RigSides>,
    transforms: Query<&mut Transform>,
) {
    let place = on.signal();
    place_side(sides.side_mut(RigSide::Source), place.source, &transforms);
    place_side(sides.side_mut(RigSide::Target), place.target, &transforms);
}

fn place_side(side: &mut ShownSide, offset: f32, transforms: &Query<&mut Transform>) {
    side.offset = offset;
    let Some(stage) = &side.stage else {
        return;
    };
    if let Some(mut transform) = transforms.get_entity(stage.root) {
        transform.translation.x = offset;
    }
}

fn set_rig_side(on: On<SetRigSide>, mut sides: ResMut<RigSides>) {
    let signal = on.signal();
    let side = sides.side_mut(signal.side);
    side.reference = signal.reference.clone();
    side.root = signal.root;
    side.colors = signal.colors.clone();
}

fn set_pose_retarget(on: On<SetPoseRetarget>, mut posing: ResMut<RigPosing>) {
    posing.retarget = on.signal().0.clone();
}

fn set_rig_preview(on: On<SetRigPreview>, mut posing: ResMut<RigPosing>) {
    let signal = on.signal();
    posing.mode = signal.mode;
    posing.root_motion = signal.root_motion;
    posing.bones = signal.bones;
}

fn scene_bones(
    stage: &ShownStage,
    scene: &SpawnedScene,
    added: &Query<(Entity, &SkeletonComponent), Added<SkeletonComponent>>,
    parents: &Query<&ChildOf>,
) -> Option<FoundBones> {
    let (_, component) = added.iter().find(|(entity, component)| {
        scene.node_entities.contains(entity) && component.skeleton().id() == stage.skeleton_id
    })?;
    let entities = component
        .bones()
        .iter()
        .map(|bone| bone.entity())
        .collect::<Option<Vec<Entity>>>()
        .filter(|entities| entities.len() == stage.skeleton.bone_count())?;
    let top = stage.skeleton.parents.iter().position(Option::is_none)?;
    let model_space = parents.get_entity(entities[top])?.parent();
    Some(FoundBones {
        entities,
        model_space,
    })
}

fn spawn_bones(cmd: &mut CommandQueue, skeleton: &Skeleton, root: Entity) -> FoundBones {
    let entities: Vec<Entity> = skeleton
        .names
        .iter()
        .zip(skeleton.rest_pose.iter())
        .map(|(name, rest)| {
            cmd.spawn((Name::new(name.as_str()), transform_of(rest)))
                .entity()
        })
        .collect();
    for (bone, parent) in skeleton.parents.iter().enumerate() {
        let parent = parent.map_or(root, |parent| entities[usize::from(parent)]);
        cmd.add_child(parent, entities[bone]);
    }
    FoundBones {
        entities,
        model_space: root,
    }
}

fn find_rig_bones(
    mut sides: ResMut<RigSides>,
    added: Query<(Entity, &SkeletonComponent), Added<SkeletonComponent>>,
    parents: Query<&ChildOf>,
    mut cmd: CommandQueue,
) {
    let sides = &mut *sides;
    for side in [&mut sides.source, &mut sides.target] {
        let Some(stage) = &mut side.stage else {
            continue;
        };
        if stage.bones.is_some() {
            continue;
        }
        let found = stage
            .scene
            .as_ref()
            .and_then(|scene| scene_bones(stage, scene, &added, &parents));
        let bones = match found {
            Some(bones) => bones,
            None => {
                if let Some(scene) = stage.scene.take() {
                    for root in scene.root_entities {
                        cmd.despawn(root);
                    }
                }
                spawn_bones(&mut cmd, &stage.skeleton, stage.root)
            }
        };
        stage.bones = Some(bones);
    }
}

fn transform_of(joint: &JointPose) -> Transform {
    Transform::from_translation_rotation_scale(joint.translation, joint.rotation, joint.scale)
}

fn model_position(skeleton: &Skeleton, pose: &[JointPose], bone: usize) -> Vec3 {
    let mut model = pose[bone].to_mat4();
    let mut current = bone;
    while let Some(parent) = skeleton.parents[current] {
        current = usize::from(parent);
        model = pose[current].to_mat4() * model;
    }
    model.w_axis.truncate()
}

fn pose_side(
    side: &ShownSide,
    pose: &[JointPose],
    root_motion: bool,
    transforms: &Query<&mut Transform>,
    globals: &Query<&GlobalTransform>,
) {
    let Some(stage) = &side.stage else {
        return;
    };
    let Some(bones) = &stage.bones else {
        return;
    };
    for (bone, joint) in bones.entities.iter().zip(pose) {
        if let Some(mut transform) = transforms.get_entity(*bone) {
            transform.translation = joint.translation;
            transform.rotation = joint.rotation;
            transform.scale = joint.scale;
        }
    }
    let mut at = Vec3::X * side.offset;
    if let Some(root) = side.root.filter(|root| !root_motion && *root < pose.len()) {
        let reference = side.reference_pose(&stage.skeleton);
        let moved = model_position(&stage.skeleton, pose, root)
            - model_position(&stage.skeleton, &reference, root);
        let moved = globals
            .get_entity(bones.model_space)
            .map_or(moved, |space| space.matrix().transform_vector3(moved));
        at -= Vec3::new(moved.x, 0.0, moved.z);
    }
    if let Some(mut transform) = transforms.get_entity(stage.root) {
        transform.translation = at;
    }
}

fn pose_rig_sides(
    playback: Res<ClipPlayback>,
    mut posing: ResMut<RigPosing>,
    sides: Res<RigSides>,
    transforms: Query<&mut Transform>,
    globals: Query<&GlobalTransform>,
) {
    let clip = playback
        .clip
        .as_ref()
        .filter(|_| posing.mode == RigPreviewMode::Clip);
    let source = sides.source.stage.as_ref().map(|stage| match clip {
        Some(clip) => clip.sample_pose(&stage.skeleton, playback.time),
        None => sides.source.reference_pose(&stage.skeleton),
    });
    let target = sides.target.stage.as_ref().map(|stage| {
        let mut pose = sides.target.reference_pose(&stage.skeleton);
        let retargeted = clip
            .and(source.as_ref())
            .zip(posing.retarget.as_mut())
            .map(|(source, retarget)| retarget.retarget_into(source, &mut pose));
        if let Some(Err(_)) = retargeted {
            pose = sides.target.reference_pose(&stage.skeleton);
        }
        pose
    });
    if let Some(pose) = &source {
        pose_side(
            &sides.source,
            pose,
            posing.root_motion,
            &transforms,
            &globals,
        );
    }
    if let Some(pose) = &target {
        pose_side(
            &sides.target,
            pose,
            posing.root_motion,
            &transforms,
            &globals,
        );
    }
}

fn draw_rig_bones(
    posing: Res<RigPosing>,
    sides: Res<RigSides>,
    globals: Query<&GlobalTransform>,
    mut gizmos: DebugGizmos,
) {
    if !posing.bones {
        return;
    }
    for side in [&sides.source, &sides.target] {
        let Some(stage) = &side.stage else {
            continue;
        };
        let Some(bones) = &stage.bones else {
            continue;
        };
        let joints: Vec<Option<Vec3>> = bones
            .entities
            .iter()
            .map(|bone| Some(globals.get_entity(*bone)?.translation()))
            .collect();
        for (bone, joint) in joints.iter().enumerate() {
            let Some(joint) = *joint else {
                continue;
            };
            let color = side.colors.get(bone).copied().unwrap_or(BONE_GREY);
            gizmos.sphere(joint, JOINT_SIZE, color);
            let parent = stage.skeleton.parents[bone]
                .and_then(|parent| joints.get(usize::from(parent)).copied().flatten());
            if let Some(parent) = parent {
                gizmos.line(parent, joint, color);
            }
        }
    }
}

struct FollowedMidpoint {
    roots: Vec<Entity>,
    at: Vec3,
}

#[derive(Default)]
struct FollowedRoots {
    last: Option<FollowedMidpoint>,
}

impl FromWorld for FollowedRoots {
    fn from_world(_world: &World) -> Self {
        Self::default()
    }
}

fn follow_rig_roots(
    sides: Res<RigSides>,
    globals: Query<&GlobalTransform>,
    cameras: Query<&mut OrbitCamera>,
    mut followed: SystemLocal<FollowedRoots>,
) {
    let roots: Vec<Entity> = [&sides.source, &sides.target]
        .into_iter()
        .filter_map(ShownSide::root_bone)
        .collect();
    let joints: Vec<Vec3> = roots
        .iter()
        .filter_map(|root| Some(globals.get_entity(*root)?.translation()))
        .collect();
    if joints.is_empty() || joints.len() != roots.len() {
        followed.last = None;
        return;
    }
    let at = joints.iter().sum::<Vec3>() / joints.len() as f32;
    let last = followed.last.replace(FollowedMidpoint {
        roots: roots.clone(),
        at,
    });
    for mut orbit in cameras.iter() {
        match &last {
            Some(last) if last.roots == roots => {
                let moved = at - last.at;
                orbit.target += Vec3::new(moved.x, 0.0, moved.z);
            }
            _ => orbit.target = at,
        }
    }
}

/// Adds a retarget preview to `app`: the stages [`ShowRigStage`] gives it, posed in their reference poses or by the clip [`crate::preview::ShowClip`] gives it.
pub fn retarget_preview(app: &mut App) {
    if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
        app.register_plugin(DebugGizmosPlugin);
    }
    clip_playback(app);
    app.insert_resource(RigSides::default())
        .insert_resource(RigPosing::default())
        .add_system(Update, find_rig_bones.at_preview_time())
        .add_system(
            LateUpdate,
            pose_rig_sides
                .before(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_system(
            LateUpdate,
            follow_rig_roots
                .after(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_listener(show_rig_stage.at_preview_time())
        .add_listener(place_rig_sides.at_preview_time())
        .add_listener(set_rig_side.at_preview_time())
        .add_listener(set_pose_retarget.at_preview_time())
        .add_listener(set_rig_preview.at_preview_time());
    if app.is_plugin_added::<DebugGizmosPlugin>() {
        app.add_system(
            LateUpdate,
            draw_rig_bones
                .after(propagate_global_transforms)
                .at_preview_time(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_animation::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        retarget::{RetargetChain, RetargetRig},
    };
    use concerto_app::plugins::TransformPlugin;
    use concerto_ecs::{
        World,
        component::{name::Name, scene::SceneEntityRef},
        entity::hierarchy::ChildOf,
    };
    use concerto_editor::{hosted::testing::plain_recipe, preview::PreviewPlugin};
    use concerto_foundation::{
        assets::handle::AssetHandle,
        transform::{GlobalTransform, Transform},
    };
    use concerto_mesh::skeleton::SkeletonComponent;
    use concerto_scene::scene::SceneNode;
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use crate::preview::{SeekClip, SetClipTransport, ShowClip};

    const NAMES: [&str; 4] = ["root", "pelvis", "spine", "head"];
    const PELVIS: usize = 1;
    const NEAR: f32 = 1e-4;

    fn ids(first: u128) -> Vec<Uuid> {
        (first..first + 4).map(Uuid::from_u128).collect()
    }

    fn skeleton(first_id: u128, heights: [f32; 4]) -> Arc<Skeleton> {
        let at = |y| JointPose {
            translation: Vec3::new(0.0, y, 0.0),
            ..JointPose::IDENTITY
        };
        Arc::new(
            Skeleton::new(
                vec![Mat4::IDENTITY; 4],
                vec![None, Some(0), Some(1), Some(2)],
                NAMES.map(Into::into).to_vec(),
                heights.map(at).to_vec(),
                ids(first_id),
            )
            .unwrap(),
        )
    }

    fn source() -> Arc<Skeleton> {
        skeleton(10, [0.0, 1.0, 0.3, 0.4])
    }

    fn target() -> Arc<Skeleton> {
        skeleton(20, [0.0, 0.9, 0.25, 0.35])
    }

    fn rig() -> RetargetRig {
        RetargetRig::new()
            .with_root("pelvis")
            .with_chain(RetargetChain::new("spine", "spine", "head"))
    }

    fn retarget() -> PoseRetarget {
        let source_rig = rig().resolve(&source(), None).unwrap();
        let target_rig = rig().resolve(&target(), None).unwrap();
        PoseRetarget::new(&source_rig, &source(), &target_rig, &target()).unwrap()
    }

    fn clip() -> Arc<AnimationClip> {
        let source = source();
        let mut clip = AnimationClip::default();
        clip.add_channel(
            source.bone_ids[PELVIS],
            AnimationChannel::new(
                vec![0.0, 2.0],
                AnimationChanelOutput::from_translation(
                    [[0.0, 1.0, 0.0], [2.0, 1.0, 1.0]].into_iter(),
                ),
            ),
        );
        clip.add_channel(
            source.bone_ids[2],
            AnimationChannel::new(
                vec![0.0, 2.0],
                AnimationChanelOutput::from_rotation(
                    [
                        Quat::IDENTITY.to_array(),
                        Quat::from_rotation_x(0.6).to_array(),
                    ]
                    .into_iter(),
                ),
            ),
        );
        Arc::new(clip)
    }

    fn bent(skeleton: &Skeleton) -> Vec<JointPose> {
        let mut pose = skeleton.rest_pose.to_vec();
        pose[2].rotation = Quat::from_rotation_z(0.4);
        pose[3].translation.y += 0.1;
        pose
    }

    fn scene_of(skeleton_id: AssetId) -> Scene {
        let node = |name: &str, children: Vec<usize>| {
            let mut node = SceneNode {
                name: name.into(),
                children,
                components: vec![],
            };
            node.push_component(&Transform::IDENTITY).unwrap();
            node
        };
        let mut armature = node("armature", vec![1]);
        armature
            .push_component(&SkeletonComponent::new(
                AssetHandle::weak(skeleton_id),
                (1..=4).map(SceneEntityRef::Index).collect(),
                Some(SceneEntityRef::Index(2)),
            ))
            .unwrap();
        Scene {
            nodes: vec![
                armature,
                node("a", vec![2]),
                node("b", vec![3]),
                node("c", vec![4]),
                node("d", vec![]),
            ],
            referenced_assets: vec![],
        }
    }

    struct Preview {
        app: App,
    }

    impl Preview {
        fn new() -> Self {
            let mut app = App::for_previewing();
            plain_recipe(&mut app);
            app.register_plugin(TransformPlugin)
                .register_plugin(PreviewPlugin);
            app.register_type::<SkeletonComponent>();
            retarget_preview(&mut app);
            app.finish_plugin_build();
            app.update();
            let mut preview = Self { app };
            preview.trigger(SetClipTransport {
                playing: false,
                looping: true,
                rate: 1.0,
            });
            preview
        }

        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }

        fn trigger<S: Signal>(&mut self, signal: S) {
            self.world().trigger(signal);
        }

        fn frames(&mut self, count: usize) {
            for _ in 0..count {
                self.app.update();
            }
        }

        fn stage(&mut self, side: RigSide, stage: RigStage) {
            self.trigger(ShowRigStage {
                side,
                stage: Some(stage),
            });
        }

        fn both_stages(&mut self) {
            self.stage(RigSide::Source, RigStage::new(source(), AssetId::new()));
            self.stage(RigSide::Target, RigStage::new(target(), AssetId::new()));
        }

        fn shown(&self, side: RigSide) -> &ShownStage {
            let sides = self.app.get_resource::<RigSides>().unwrap();
            let shown = match side {
                RigSide::Source => &sides.source,
                RigSide::Target => &sides.target,
            };
            shown.stage.as_ref().unwrap()
        }

        fn root(&self, side: RigSide) -> Entity {
            self.shown(side).root
        }

        fn bones(&self, side: RigSide) -> Vec<Entity> {
            self.shown(side).bones.as_ref().unwrap().entities.clone()
        }

        fn local(&mut self, entity: Entity) -> Transform {
            self.world()
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .clone()
        }

        fn global(&mut self, entity: Entity) -> Vec3 {
            self.world()
                .get_component_for_entity::<GlobalTransform>(entity)
                .unwrap()
                .translation()
        }

        fn name(&mut self, entity: Entity) -> String {
            self.world()
                .get_component_for_entity::<Name>(entity)
                .unwrap()
                .as_str()
                .to_owned()
        }

        fn names(&mut self) -> Vec<String> {
            let world = self.world();
            world
                .query::<&Name, ()>()
                .iter(world)
                .map(|name| name.as_str().to_owned())
                .collect()
        }

        fn parent(&mut self, entity: Entity) -> Entity {
            self.world()
                .get_component_for_entity::<ChildOf>(entity)
                .unwrap()
                .parent()
        }

        fn camera(&mut self) -> Vec3 {
            let world = self.world();
            world
                .query::<&OrbitCamera, ()>()
                .iter(world)
                .next()
                .unwrap()
                .target
        }

        fn posed(&mut self, side: RigSide) -> Vec<Transform> {
            self.bones(side)
                .into_iter()
                .map(|bone| self.local(bone))
                .collect()
        }
    }

    fn assert_posed(posed: &[Transform], pose: &[JointPose]) {
        assert_eq!(posed.len(), pose.len());
        for (transform, joint) in posed.iter().zip(pose) {
            assert!(
                transform.translation.abs_diff_eq(joint.translation, NEAR),
                "{:?} is not {:?}",
                transform.translation,
                joint.translation
            );
            assert!(
                transform.rotation.abs_diff_eq(joint.rotation, NEAR),
                "{:?} is not {:?}",
                transform.rotation,
                joint.rotation
            );
        }
    }

    #[test]
    fn reference_mode_poses_each_sides_reference_pose() {
        let mut preview = Preview::new();
        preview.both_stages();
        preview.trigger(SetRigSide::new(RigSide::Source).with_reference(bent(&source())));
        preview.trigger(SetRigSide::new(RigSide::Target));
        preview.trigger(ShowClip(clip()));
        preview.trigger(SeekClip(1.0));

        preview.frames(2);

        assert_posed(&preview.posed(RigSide::Source), &bent(&source()));
        assert_posed(&preview.posed(RigSide::Target), &target().rest_pose);
    }

    #[test]
    fn clip_mode_poses_the_source_from_the_clip_and_the_target_from_retarget_into() {
        let mut preview = Preview::new();
        preview.both_stages();
        preview.trigger(SetPoseRetarget(Some(retarget())));
        preview.trigger(SetRigPreview {
            mode: RigPreviewMode::Clip,
            root_motion: true,
            bones: true,
        });
        preview.trigger(ShowClip(clip()));
        preview.trigger(SeekClip(1.5));

        preview.frames(2);

        let sampled = clip().sample_pose(&source(), 1.5);
        let mut retargeted = target().rest_pose.to_vec();
        retarget().retarget_into(&sampled, &mut retargeted).unwrap();
        assert_posed(&preview.posed(RigSide::Source), &sampled);
        assert_posed(&preview.posed(RigSide::Target), &retargeted);
        assert!(
            !retargeted[PELVIS]
                .translation
                .abs_diff_eq(target().rest_pose[PELVIS].translation, NEAR)
        );
    }

    #[test]
    fn with_root_motion_off_each_pelvis_stays_over_its_spot() {
        let mut preview = Preview::new();
        preview.trigger(PlaceRigSides {
            source: -1.0,
            target: 1.0,
        });
        preview.both_stages();
        preview.trigger(SetRigSide::new(RigSide::Source).with_root(PELVIS));
        preview.trigger(SetRigSide::new(RigSide::Target).with_root(PELVIS));
        preview.trigger(SetPoseRetarget(Some(retarget())));
        preview.trigger(SetRigPreview {
            mode: RigPreviewMode::Clip,
            root_motion: false,
            bones: true,
        });
        preview.trigger(ShowClip(clip()));
        preview.trigger(SeekClip(2.0));

        preview.frames(3);

        let source_pelvis = preview.bones(RigSide::Source)[PELVIS];
        let target_pelvis = preview.bones(RigSide::Target)[PELVIS];
        assert!(
            preview
                .global(source_pelvis)
                .abs_diff_eq(Vec3::new(-1.0, 1.0, 0.0), NEAR),
            "{}",
            preview.global(source_pelvis)
        );
        assert!(
            preview
                .global(target_pelvis)
                .abs_diff_eq(Vec3::new(1.0, 0.9, 0.0), NEAR),
            "{}",
            preview.global(target_pelvis)
        );
    }

    #[test]
    fn a_stage_with_no_scene_spawns_bones_parented_as_the_skeleton_says() {
        let mut preview = Preview::new();
        preview.stage(RigSide::Source, RigStage::new(source(), AssetId::new()));

        preview.frames(1);

        let root = preview.root(RigSide::Source);
        let bones = preview.bones(RigSide::Source);
        let names: Vec<String> = bones.iter().map(|bone| preview.name(*bone)).collect();
        assert_eq!(names, NAMES);
        assert_eq!(preview.parent(bones[0]), root);
        for pair in bones.windows(2) {
            assert_eq!(preview.parent(pair[1]), pair[0]);
        }
        for bone in bones {
            assert!(
                preview
                    .world()
                    .get_component_for_entity::<SkeletonComponent>(bone)
                    .is_none()
            );
        }
    }

    #[test]
    fn a_stage_whose_scene_holds_its_skeleton_poses_the_scenes_bones() {
        let mut preview = Preview::new();
        let id = AssetId::new();
        preview.stage(
            RigSide::Source,
            RigStage::new(source(), id).with_scene(scene_of(id)),
        );
        preview.trigger(SetRigSide::new(RigSide::Source).with_reference(bent(&source())));

        preview.frames(2);

        let bones = preview.bones(RigSide::Source);
        let names: Vec<String> = bones.iter().map(|bone| preview.name(*bone)).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert!(preview.names().contains(&"armature".to_owned()));
        assert_posed(&preview.posed(RigSide::Source), &bent(&source()));
    }

    #[test]
    fn a_stage_whose_scene_holds_another_skeleton_is_replaced_by_bones() {
        let mut preview = Preview::new();
        preview.stage(
            RigSide::Source,
            RigStage::new(source(), AssetId::new()).with_scene(scene_of(AssetId::new())),
        );

        preview.frames(2);

        let mut names = preview.names();
        names.sort();
        let mut expected = NAMES.map(str::to_owned).to_vec();
        expected.sort();
        assert_eq!(names, expected);
        let bones = preview.bones(RigSide::Source);
        assert_eq!(preview.parent(bones[0]), preview.root(RigSide::Source));
    }

    #[test]
    fn placing_the_sides_moves_them_without_spawning_them_again() {
        let mut preview = Preview::new();
        preview.stage(RigSide::Source, RigStage::new(source(), AssetId::new()));
        preview.frames(1);
        let root = preview.root(RigSide::Source);
        let bones = preview.bones(RigSide::Source);

        preview.trigger(PlaceRigSides {
            source: -2.0,
            target: 3.0,
        });
        preview.stage(RigSide::Target, RigStage::new(target(), AssetId::new()));
        preview.frames(2);

        assert_eq!(preview.root(RigSide::Source), root);
        assert_eq!(preview.bones(RigSide::Source), bones);
        assert_eq!(preview.global(root), Vec3::new(-2.0, 0.0, 0.0));
        let target_root = preview.root(RigSide::Target);
        assert_eq!(preview.global(target_root), Vec3::new(3.0, 0.0, 0.0));
    }

    #[test]
    fn the_camera_targets_the_midpoint_of_the_root_bones_and_follows_it_horizontally() {
        let mut preview = Preview::new();
        preview.trigger(PlaceRigSides {
            source: -1.0,
            target: 1.0,
        });
        preview.both_stages();
        preview.trigger(SetRigSide::new(RigSide::Source).with_root(PELVIS));
        preview.trigger(SetRigSide::new(RigSide::Target).with_root(PELVIS));
        preview.trigger(SetPoseRetarget(Some(retarget())));
        preview.trigger(ShowClip(clip()));
        preview.frames(3);
        assert!(
            preview
                .camera()
                .abs_diff_eq(Vec3::new(0.0, 0.95, 0.0), NEAR)
        );

        preview.trigger(SetRigPreview {
            mode: RigPreviewMode::Clip,
            root_motion: true,
            bones: true,
        });
        preview.trigger(SeekClip(2.0));
        preview.frames(2);

        assert!(
            preview
                .camera()
                .abs_diff_eq(Vec3::new(1.9, 0.95, 0.95), NEAR),
            "{}",
            preview.camera()
        );
    }

    #[test]
    fn showing_no_stage_empties_that_side() {
        let mut preview = Preview::new();
        preview.stage(RigSide::Source, RigStage::new(source(), AssetId::new()));
        preview.frames(1);

        preview.trigger(ShowRigStage {
            side: RigSide::Source,
            stage: None,
        });

        assert!(preview.names().is_empty());
    }
}

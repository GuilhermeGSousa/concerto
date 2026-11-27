//! Everything that runs in a ragdoll tab's preview `App`: the skeleton's character with its ragdoll on a floor, dropped, poked, or following a clip.
use std::sync::Arc;

use concerto_animation::{
    clip::AnimationClip,
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
};
use concerto_animation_editor::preview::{
    ClearClip, SetClipTransport, SetSkeletonGizmo, ShowClip, clip_preview,
};
use concerto_app::{
    App,
    edit_time::AtPreviewTime,
    schedule_groups::{LateFixedUpdate, LateUpdate, Update},
};
use concerto_debug_gizmos::DebugGizmosPlugin;
use concerto_ecs::{
    Added, Entity, IntoSystemConfig, Query, Res, ResMut, Resource,
    command::CommandQueue,
    component::{name::Name as EntityName, scene::SceneEntityRef},
    signal::{On, Signal},
};
use concerto_editor::preview::OrbitCamera;
use concerto_foundation::{
    assets::{AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle},
    transform::{GlobalTransform, Transform, systems::propagate_global_transforms},
};
use concerto_mesh::{JointPose, Skeleton, SkeletonComponent};
use concerto_name::Name;
use concerto_physics::{
    collider::{Collider, register_colliders},
    physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState,
    ragdoll::RagdollId,
    step_simulation,
};
use concerto_physics_gizmos::{PhysicsGizmos, draw_ragdolls};
use concerto_ragdoll::{
    CharacterRagdoll, LimpNode, Ragdoll, RagdollBody, character::create_character_ragdolls,
    drive::drive_from_graph,
};
use concerto_render::plugin::RenderPlugin;
use concerto_scene::{
    scene::Scene,
    spawner::{SpawnFilter, SpawnedScene, spawn_scene},
};
use glam::Vec3;

/// Replaces what the preview stands on its floor.
///
/// It stands the skeleton's character there, posed in the rest pose, in
/// `scene` when it has one and as bare bones otherwise.
pub struct ShowRagdollStage {
    pub skeleton: Arc<Skeleton>,
    pub skeleton_id: AssetId,
    pub scene: Option<Scene>,
}
impl Signal for ShowRagdollStage {}

/// Gives the preview the Ragdoll asset `id` as edited so far, and rebuilds the character's body from it.
pub struct SetRagdoll {
    pub id: AssetId,
    pub ragdoll: Ragdoll,
}
impl Signal for SetRagdoll {}

/// Lifts the character until its lowest bone is `height` metres above the floor, and lets it go limp.
pub struct DropRagdoll {
    pub height: f32,
}
impl Signal for DropRagdoll {}

/// Hits the part on `bone` with `impulse`, in N·s, making the body limp first when it is not.
pub struct PokeRagdoll {
    pub bone: Name,
    pub impulse: Vec3,
}
impl Signal for PokeRagdoll {}

/// Plays `clip` on the character, whose ragdoll follows it.
pub struct PlayRagdoll(pub Arc<AnimationClip>);
impl Signal for PlayRagdoll {}

/// Puts the character back in its rest pose on the floor's origin, following its animation with none of the simulated pose drawn.
pub struct ResetRagdoll;
impl Signal for ResetRagdoll {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Lift {
    #[default]
    None,
    Rising {
        height: f32,
    },
    Lifted,
}

struct Poke {
    bone: Name,
    impulse: Vec3,
}

#[derive(Resource, Default)]
struct RagdollStage {
    root: Option<Entity>,
    floor: Option<Entity>,
    skeleton: Option<Arc<Skeleton>>,
    skeleton_id: AssetId,
    scene: Option<SpawnedScene>,
    character: Option<Entity>,
    ragdoll: Option<AssetId>,
    lift: Lift,
    poke: Option<Poke>,
    framed: bool,
    limp: bool,
    limp_graph: Option<AssetHandle<AnimationGraph>>,
}

impl RagdollStage {
    fn character_ragdoll(&self) -> Option<CharacterRagdoll> {
        Some(CharacterRagdoll::new(AssetHandle::weak(self.ragdoll?)))
    }

    fn insert_character(&self, cmd: &mut CommandQueue, character: Entity) {
        if let Some(body) = self.character_ragdoll() {
            cmd.insert(body, character);
        }
        if self.limp {
            if let Some(graph) = &self.limp_graph {
                cmd.insert(AnimationHandleComponent::new(graph.clone()), character);
            }
        } else if let Some(skeleton) = &self.skeleton {
            cmd.remove::<AnimationHandleComponent>(character);
            cmd.insert(AnimationPlayer::new(skeleton.bone_count()), character);
        }
    }
}

fn copy_skeleton(skeleton: &Skeleton) -> Skeleton {
    Skeleton {
        inverse_bindposes: skeleton.inverse_bindposes.clone(),
        parents: skeleton.parents.clone(),
        names: skeleton.names.clone(),
        rest_pose: skeleton.rest_pose.clone(),
        bone_ids: skeleton.bone_ids.clone(),
    }
}

fn transform_of(joint: &JointPose) -> Transform {
    Transform::from_translation_rotation_scale(joint.translation, joint.rotation, joint.scale)
}

fn show_stage(
    on: On<ShowRagdollStage>,
    mut stage: ResMut<RagdollStage>,
    mut skeletons: ResMut<AssetStore<Skeleton>>,
    server: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    let signal = on.signal();
    if let Some(root) = stage.root.take() {
        cmd.despawn(root);
    }
    if stage.floor.is_none() {
        stage.floor = Some(
            cmd.spawn((
                Collider::cuboid(200.0, 0.5, 200.0),
                Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
            ))
            .entity(),
        );
    }
    skeletons.insert(signal.skeleton_id, copy_skeleton(&signal.skeleton));
    let root = cmd.spawn(Transform::IDENTITY).entity();
    let scene = signal
        .scene
        .as_ref()
        .map(|scene| spawn_scene(&mut cmd, scene, root, &SpawnFilter::default()));
    *stage = RagdollStage {
        root: Some(root),
        floor: stage.floor,
        skeleton: Some(Arc::clone(&signal.skeleton)),
        skeleton_id: signal.skeleton_id,
        scene,
        ragdoll: stage.ragdoll,
        limp_graph: stage
            .limp_graph
            .clone()
            .or_else(|| Some(server.add(AnimationGraph::from_node(LimpNode::default())))),
        ..RagdollStage::default()
    };
    cmd.trigger(SetSkeletonGizmo(false));
}

fn set_ragdoll(
    on: On<SetRagdoll>,
    mut stage: ResMut<RagdollStage>,
    mut ragdolls: ResMut<AssetStore<Ragdoll>>,
    characters: Query<&CharacterRagdoll>,
    mut cmd: CommandQueue,
) {
    let signal = on.signal();
    ragdolls.insert(signal.id, signal.ragdoll.clone());
    stage.ragdoll = Some(signal.id);
    if let Some(character) = stage.character
        && characters.get_entity(character).is_some()
        && let Some(rebuilt) = stage.character_ragdoll()
    {
        cmd.insert(rebuilt, character);
    }
}

fn spawn_bones(cmd: &mut CommandQueue, skeleton: &Skeleton, root: Entity) -> Entity {
    let bones: Vec<Entity> = skeleton
        .names
        .iter()
        .zip(skeleton.rest_pose.iter())
        .map(|(name, rest)| {
            cmd.spawn((EntityName::new(name.as_str()), transform_of(rest)))
                .entity()
        })
        .collect();
    for (bone, parent) in skeleton.parents.iter().enumerate() {
        let parent = parent.map_or(root, |parent| bones[usize::from(parent)]);
        cmd.add_child(parent, bones[bone]);
    }
    let top = skeleton.parents.iter().position(Option::is_none);
    let component = SkeletonComponent::new(
        AssetHandle::default(),
        bones
            .iter()
            .map(|bone| SceneEntityRef::Entity(*bone))
            .collect(),
        top.map(|top| SceneEntityRef::Entity(bones[top])),
    );
    cmd.spawn(component).entity()
}

fn find_character(
    mut stage: ResMut<RagdollStage>,
    added: Query<(Entity, &SkeletonComponent), Added<SkeletonComponent>>,
    mut cmd: CommandQueue,
) {
    let (Some(skeleton), Some(root)) = (stage.skeleton.clone(), stage.root) else {
        return;
    };
    if stage.character.is_some() || stage.ragdoll.is_none() {
        return;
    }
    let in_scene = stage.scene.as_ref().and_then(|scene| {
        added
            .iter()
            .find(|(entity, component)| {
                scene.node_entities.contains(entity)
                    && component.skeleton().id() == stage.skeleton_id
                    && component.bones().len() == skeleton.bone_count()
            })
            .map(|(entity, _)| entity)
    });
    let character = match in_scene {
        Some(character) => character,
        None => {
            if stage.scene.is_some() && added.iter().next().is_none() {
                return;
            }
            if let Some(scene) = stage.scene.take() {
                for root in scene.root_entities {
                    cmd.despawn(root);
                }
            }
            let character = spawn_bones(&mut cmd, &skeleton, root);
            cmd.insert(
                SkeletonComponent::new(AssetHandle::weak(stage.skeleton_id), Vec::new(), None),
                character,
            );
            character
        }
    };
    stage.character = Some(character);
    stage.insert_character(&mut cmd, character);
}

fn pose_at_rest(
    stage: &RagdollStage,
    skeleton: &Skeleton,
    components: &Query<&SkeletonComponent>,
    transforms: &mut Query<&mut Transform>,
) {
    if let Some(root) = stage.root
        && let Some(mut transform) = transforms.get_entity_mut(root)
    {
        **transform = Transform::IDENTITY;
    }
    let Some(component) = stage
        .character
        .and_then(|character| components.get_entity(character))
    else {
        return;
    };
    for (bone, rest) in component.bones().iter().zip(skeleton.rest_pose.iter()) {
        if let Some(mut transform) = bone
            .entity()
            .and_then(|bone| transforms.get_entity_mut(bone))
        {
            transform.translation = rest.translation;
            transform.rotation = rest.rotation;
            transform.scale = rest.scale;
        }
    }
}

fn reset_ragdoll(
    _: On<ResetRagdoll>,
    mut stage: ResMut<RagdollStage>,
    components: Query<&SkeletonComponent>,
    mut transforms: Query<&mut Transform>,
    mut cmd: CommandQueue,
) {
    let Some(skeleton) = stage.skeleton.clone() else {
        return;
    };
    pose_at_rest(&stage, &skeleton, &components, &mut transforms);
    stage.lift = Lift::None;
    stage.poke = None;
    cmd.trigger(ClearClip);
    stage.limp = false;
    if let Some(character) = stage.character {
        stage.insert_character(&mut cmd, character);
    }
}

fn play_ragdoll(
    on: On<PlayRagdoll>,
    mut stage: ResMut<RagdollStage>,
    components: Query<&SkeletonComponent>,
    mut transforms: Query<&mut Transform>,
    mut cmd: CommandQueue,
) {
    let Some(skeleton) = stage.skeleton.clone() else {
        return;
    };
    pose_at_rest(&stage, &skeleton, &components, &mut transforms);
    stage.lift = Lift::None;
    stage.poke = None;
    stage.limp = false;
    if let Some(character) = stage.character {
        stage.insert_character(&mut cmd, character);
    }
    cmd.trigger(ShowClip(Arc::clone(&on.signal().0)));
    cmd.trigger(SetClipTransport {
        playing: true,
        looping: true,
        rate: 1.0,
    });
}

fn drop_ragdoll(
    on: On<DropRagdoll>,
    mut stage: ResMut<RagdollStage>,
    components: Query<&SkeletonComponent>,
    mut transforms: Query<&mut Transform>,
    mut cmd: CommandQueue,
) {
    let Some(skeleton) = stage.skeleton.clone() else {
        return;
    };
    pose_at_rest(&stage, &skeleton, &components, &mut transforms);
    stage.poke = None;
    cmd.trigger(ClearClip);
    if let Some(character) = stage.character {
        cmd.remove::<CharacterRagdoll>(character);
    }
    stage.lift = Lift::Rising {
        height: on.signal().height.max(0.0),
    };
}

fn lift_character(
    mut stage: ResMut<RagdollStage>,
    components: Query<&SkeletonComponent>,
    globals: Query<&GlobalTransform>,
    mut transforms: Query<&mut Transform>,
    mut cmd: CommandQueue,
) {
    match stage.lift {
        Lift::None => {}
        Lift::Rising { height } => {
            let Some(component) = stage
                .character
                .and_then(|character| components.get_entity(character))
            else {
                return;
            };
            let lowest = component
                .bones()
                .iter()
                .filter_map(|bone| globals.get_entity(bone.entity()?))
                .map(|global| global.translation().y)
                .fold(0.0, f32::min);
            if let Some(root) = stage.root
                && let Some(mut transform) = transforms.get_entity_mut(root)
            {
                transform.translation = Vec3::Y * (height - lowest);
            }
            stage.lift = Lift::Lifted;
        }
        Lift::Lifted => {
            stage.limp = true;
            if let Some(character) = stage.character {
                stage.insert_character(&mut cmd, character);
            }
            stage.lift = Lift::None;
        }
    }
}

fn frame_character(
    mut stage: ResMut<RagdollStage>,
    components: Query<&SkeletonComponent>,
    globals: Query<&GlobalTransform>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if stage.framed {
        return;
    }
    let Some(component) = stage
        .character
        .and_then(|character| components.get_entity(character))
    else {
        return;
    };
    let joints: Vec<Vec3> = component
        .bones()
        .iter()
        .filter_map(|bone| Some(globals.get_entity(bone.entity()?)?.translation()))
        .collect();
    let lowest = joints
        .iter()
        .fold(Vec3::splat(f32::MAX), |low, joint| low.min(*joint));
    let highest = joints
        .iter()
        .fold(Vec3::splat(f32::MIN), |high, joint| high.max(*joint));
    let size = highest - lowest;
    if joints.is_empty() || size.max_element() < 0.1 {
        return;
    }
    for mut orbit in cameras.iter_mut() {
        orbit.target = (lowest + highest) / 2.0;
        orbit.distance = size.max_element() * 2.2;
    }
    stage.framed = true;
}

fn poke_ragdoll(on: On<PokeRagdoll>, mut stage: ResMut<RagdollStage>, mut cmd: CommandQueue) {
    let signal = on.signal();
    stage.poke = Some(Poke {
        bone: signal.bone,
        impulse: signal.impulse,
    });
    if !stage.limp {
        stage.limp = true;
        if let Some(character) = stage.character {
            stage.insert_character(&mut cmd, character);
        }
    }
}

fn apply_poke(
    mut stage: ResMut<RagdollStage>,
    characters: Query<(&RagdollId, &RagdollBody, &AnimationPlayer)>,
    mut state: ResMut<PhysicsState>,
) {
    let Some(character) = stage.character else {
        return;
    };
    let Some((id, body, player)) = characters.get_entity(character) else {
        return;
    };
    if !stage.limp || stage.poke.is_none() || body.is_animated(player) {
        return;
    }
    let Some(poke) = stage.poke.take() else {
        return;
    };
    if let Some(part) = body.resolved().part_of(poke.bone.as_str())
        && let Some(target) = state.ragdoll_bodies(*id).get(part).copied()
    {
        state.add_impulse(target, poke.impulse);
    }
}

/// Adds the ragdoll preview to `app`: a floor with physics, the character [`ShowRagdollStage`] gives it with the ragdoll [`SetRagdoll`] gives it drawn over it, and the actions [`DropRagdoll`], [`PokeRagdoll`], [`PlayRagdoll`] and [`ResetRagdoll`].
pub fn ragdoll_preview(app: &mut App) {
    if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
        app.register_plugin(DebugGizmosPlugin);
    }
    app.register_asset::<Skeleton>()
        .register_asset::<Ragdoll>()
        .register_type::<Ragdoll>()
        .register_type::<CharacterRagdoll>();
    clip_preview(app);
    if app.get_resource::<PhysicsState>().is_none() {
        app.insert_resource(PhysicsState::new());
    }
    app.insert_resource(PhysicsPipeline::new())
        .insert_resource(PhysicsGizmos {
            colliders: false,
            ragdolls: true,
            contacts: false,
            sensing: false,
        })
        .insert_resource(RagdollStage::default())
        .add_system(Update, find_character.at_preview_time())
        .add_system(
            LateUpdate,
            register_colliders
                .after(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_system(
            LateUpdate,
            lift_character
                .after(propagate_global_transforms)
                .before(create_character_ragdolls)
                .at_preview_time(),
        )
        .add_system(
            LateUpdate,
            frame_character
                .after(propagate_global_transforms)
                .at_preview_time(),
        )
        .add_system(
            LateFixedUpdate,
            apply_poke
                .after(drive_from_graph)
                .before(step_simulation)
                .at_preview_time(),
        )
        .add_system(LateFixedUpdate, step_simulation.at_preview_time())
        .add_listener(show_stage.at_preview_time())
        .add_listener(set_ragdoll.at_preview_time())
        .add_listener(reset_ragdoll.at_preview_time())
        .add_listener(play_ragdoll.at_preview_time())
        .add_listener(drop_ragdoll.at_preview_time())
        .add_listener(poke_ragdoll.at_preview_time());
    if app.is_plugin_added::<DebugGizmosPlugin>() {
        app.add_system(
            LateUpdate,
            draw_ragdolls
                .after(propagate_global_transforms)
                .at_preview_time(),
        );
    }
}

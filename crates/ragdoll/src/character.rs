use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res, ResMut, Without,
    component::scene::SceneEntityRef,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::{
        On,
        lifecycle::{Remove, Replace},
    },
};
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::{JointName, Skeleton, SkeletonComponent};
use concerto_name::Name;
use concerto_physics::{
    body::BodyId,
    carried::Carried,
    layers::CollisionLayers,
    physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState,
    query::QueryFilter,
    ragdoll::{BodyPose, JointDrive, RagdollId},
    rigid_body::MotionType,
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Mat4, Quat, Vec3};
use log::warn;

use crate::{asset::Ragdoll, resolve::ResolvedRagdoll};

/// What a character's ragdoll does.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Default)]
pub enum RagdollMode {
    /// Every part is kinematic and is driven to its bone each fixed step: the body pushes props and can be hit, and the animation is untouched.
    #[default]
    FollowsAnimation,
    /// Every part is dynamic with its motors off, and keeps the velocity it had: the body falls as a body.
    Limp,
}

/// How much of one part's simulated pose is drawn, overriding the character's `weight` for the part on `bone`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Default)]
pub struct PartWeight {
    pub bone: JointName,
    pub weight: f32,
}

/// Gives a character a physical body: goes on the entity with the `SkeletonComponent`, names the ragdoll asset of that skeleton, and builds the ragdoll at the current pose once both have loaded.
#[derive(Component, Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct CharacterRagdoll {
    pub ragdoll: AssetHandle<Ragdoll>,
    /// The character's own capsule, carried under the body while the character is physical; it should have `CollisionLayers::character()` so the parts pass through it.
    pub capsule: SceneEntityRef,
    pub mode: RagdollMode,
    /// How much of the simulated pose is drawn on the skeleton: 0 is the animated pose, 1 the ragdoll's.
    pub weight: f32,
    /// Weights of single parts, each replacing `weight` for its part.
    pub part_weights: Vec<PartWeight>,
}

impl CharacterRagdoll {
    /// A ragdoll that follows the animation.
    pub fn new(ragdoll: AssetHandle<Ragdoll>) -> Self {
        Self {
            ragdoll,
            ..Self::default()
        }
    }

    pub fn with_capsule(mut self, capsule: Entity) -> Self {
        self.capsule = SceneEntityRef::Entity(capsule);
        self
    }

    pub fn with_mode(mut self, mode: RagdollMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Draws `weight` of the simulated pose on the part on `bone`, whatever the character's `weight` is.
    pub fn with_part_weight(mut self, bone: impl Into<JointName>, weight: f32) -> Self {
        self.part_weights.push(PartWeight {
            bone: bone.into(),
            weight,
        });
        self
    }

    /// Lets the body go: every part dynamic, and the whole simulated pose drawn.
    pub fn go_limp(&mut self) {
        self.mode = RagdollMode::Limp;
        self.weight = 1.0;
    }

    /// The weight of the part on `bone`, from 0 to 1.
    pub fn weight_of(&self, bone: &str) -> f32 {
        self.part_weights
            .iter()
            .find(|part| &*part.bone == bone)
            .map_or(self.weight, |part| part.weight)
            .clamp(0.0, 1.0)
    }
}

/// A character's built ragdoll, beside its `RagdollId`: which bone each part follows.
#[derive(Component, Clone)]
pub struct RagdollBody {
    resolved: ResolvedRagdoll,
    bones: Vec<Entity>,
    skeleton_bones: Vec<Entity>,
    motion: MotionType,
    previous: Vec<BodyPose>,
    current: Vec<BodyPose>,
    drawn_roots: Vec<Option<Mat4>>,
    carrying: Option<Carrying>,
}

#[derive(Clone, Copy)]
struct Carrying {
    capsule: Entity,
    body: BodyId,
    height: f32,
    layers: CollisionLayers,
}

impl RagdollBody {
    /// The asset fitted to the skeleton.
    pub fn resolved(&self) -> &ResolvedRagdoll {
        &self.resolved
    }

    /// The bone entity the part follows; a `RayHit::part` of this character resolves through it.
    pub fn bone(&self, part: usize) -> Option<Entity> {
        self.bones.get(part).copied()
    }

    /// The name of the bone the part follows.
    pub fn bone_name(&self, part: usize) -> Option<Name> {
        self.resolved.bone_name(part)
    }

    /// The bone entity of every part, in part order.
    pub fn bones(&self) -> &[Entity] {
        &self.bones
    }

    /// The world pose of every part between the last two fixed steps: `alpha` 0 is the step before last, 1 the last.
    pub fn interpolated_pose(&self, alpha: f32) -> Vec<BodyPose> {
        self.previous
            .iter()
            .zip(&self.current)
            .map(|(previous, current)| {
                BodyPose::new(
                    previous.translation.lerp(current.translation, alpha),
                    previous.rotation.slerp(current.rotation, alpha),
                )
            })
            .collect()
    }
}

/// Marks a character whose ragdoll asset does not fit its skeleton, so it is reported once; inserting `CharacterRagdoll` again clears it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct RagdollRefused;

fn part_poses(bones: &[Entity], transforms: &Query<&GlobalTransform>) -> Option<Vec<BodyPose>> {
    bones
        .iter()
        .map(|bone| {
            transforms.get_entity(*bone).map(|transform| {
                BodyPose::new(transform.translation(), transform.rotation().normalize())
            })
        })
        .collect()
}

/// Builds the ragdoll of every character whose ragdoll asset and skeleton have loaded, at its bones' current pose, with every part kinematic. Does nothing where physics is not stepped.
pub fn create_character_ragdolls(
    characters: Query<
        (Entity, &CharacterRagdoll, &SkeletonComponent),
        (Without<RagdollId>, Without<RagdollRefused>),
    >,
    transforms: Query<&GlobalTransform>,
    capsules: Query<&CollisionLayers>,
    ragdolls: Res<AssetStore<Ragdoll>>,
    skeletons: Res<AssetStore<Skeleton>>,
    pipeline: Option<Res<PhysicsPipeline>>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    if pipeline.is_none() {
        return;
    }
    for (entity, character, skeleton_component) in characters.iter() {
        let (Some(ragdoll), Some(skeleton)) = (
            ragdolls.get(&character.ragdoll),
            skeletons.get(skeleton_component.skeleton()),
        ) else {
            continue;
        };
        let resolved = match ragdoll.resolve(skeleton) {
            Ok(resolved) => resolved,
            Err(error) => {
                warn!("the ragdoll of character {entity:?} is not built: {error}");
                cmd.insert(RagdollRefused, entity);
                continue;
            }
        };
        let Some(skeleton_bones) = skeleton_component
            .bones()
            .iter()
            .map(|bone| bone.entity())
            .collect::<Option<Vec<Entity>>>()
        else {
            continue;
        };
        if skeleton_bones.len() != skeleton.bone_count() {
            continue;
        }
        let bones: Vec<Entity> = resolved
            .bones()
            .iter()
            .map(|bone| skeleton_bones[*bone])
            .collect();
        let Some(pose) = part_poses(&bones, &transforms) else {
            continue;
        };
        let Some(id) = state.create_ragdoll(resolved.built(), entity, &pose) else {
            continue;
        };
        if let Some(capsule) = character.capsule.entity() {
            let layers = capsules.get_entity(capsule).copied().unwrap_or_default();
            if layers.collides_with(&resolved.settings().layers) {
                warn!(
                    "the capsule {capsule:?} of character {entity:?} collides with its ragdoll's parts: give it `CollisionLayers::character()`"
                );
            }
        }
        for part in 0..resolved.part_count() {
            state.set_ragdoll_part_motion(id, part, MotionType::Kinematic);
        }
        cmd.insert(
            (
                id,
                RagdollBody {
                    resolved,
                    bones,
                    drawn_roots: vec![None; skeleton_bones.len()],
                    skeleton_bones,
                    motion: MotionType::Kinematic,
                    previous: pose.clone(),
                    current: pose,
                    carrying: None,
                },
            ),
            entity,
        );
    }
}

/// Each fixed step, before physics steps, makes every ragdoll's parts kinematic or dynamic as its mode asks, and drives the parts of one that follows its animation to its bones' poses.
pub fn drive_character_ragdolls(
    characters: Query<(&CharacterRagdoll, &RagdollId, &mut RagdollBody)>,
    transforms: Query<&GlobalTransform>,
    mut state: ResMut<PhysicsState>,
) {
    for (character, id, mut body) in characters.iter() {
        let motion = match character.mode {
            RagdollMode::FollowsAnimation => MotionType::Kinematic,
            RagdollMode::Limp => MotionType::Dynamic,
        };
        if !matches!(
            (body.motion, motion),
            (MotionType::Kinematic, MotionType::Kinematic)
                | (MotionType::Dynamic, MotionType::Dynamic)
        ) {
            for part in 0..body.resolved.part_count() {
                state.set_ragdoll_part_motion(*id, part, motion);
                if matches!(motion, MotionType::Dynamic) {
                    state.set_ragdoll_joint_drive(*id, part, JointDrive::Off);
                }
            }
            body.motion = motion;
        }
        if character.mode == RagdollMode::FollowsAnimation
            && let Some(pose) = part_poses(&body.bones, &transforms)
        {
            state.drive_ragdoll_kinematic(*id, &pose);
        }
    }
}

const GROUND_REACH: f32 = 3.0;

fn ground_under(state: &PhysicsState, point: Vec3) -> Option<f32> {
    state
        .cast_ray(
            point + Vec3::Y * 0.5,
            Vec3::NEG_Y * (GROUND_REACH + 0.5),
            QueryFilter::default(),
        )
        .map(|hit| hit.point.y)
}

fn yaw(rotation: Quat) -> Quat {
    let twist = Quat::from_xyzw(0.0, rotation.y, 0.0, rotation.w);
    if twist.length_squared() < 1e-8 {
        Quat::IDENTITY
    } else {
        twist.normalize()
    }
}

/// Each fixed step, before physics steps, while a character's root part has any weight: its capsule is `Carried`, is in no collision layer, and is put on the ground under the ragdoll's root part. When the weight is back at 0 the capsule is let go where it stands.
pub fn carry_capsules(
    characters: Query<(&CharacterRagdoll, &RagdollId, &mut RagdollBody)>,
    bodies: Query<(&BodyId, Option<&CollisionLayers>)>,
    carried: Query<&mut Carried>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let mut pose = Vec::new();
    for (character, id, mut body) in characters.iter() {
        let physical = body
            .bone_name(0)
            .is_some_and(|bone| character.weight_of(&bone) > 0.0);
        let capsule = character.capsule.entity().and_then(|capsule| {
            bodies.get_entity(capsule).map(|(body, layers)| Carrying {
                capsule,
                body: *body,
                height: 0.0,
                layers: layers.copied().unwrap_or_default(),
            })
        });
        let wanted = capsule.filter(|_| physical);
        if let Some(carrying) = body.carrying
            && wanted.is_none_or(|wanted| wanted.body != carrying.body)
        {
            state.set_body_layers(carrying.body, carrying.layers);
            cmd.remove::<Carried>(carrying.capsule);
            body.carrying = None;
        }
        let Some(wanted) = wanted else {
            continue;
        };
        state.ragdoll_pose(*id, &mut pose);
        let (Some(root), Some(root_body)) = (
            pose.first().copied(),
            state.ragdoll_bodies(*id).first().copied(),
        ) else {
            continue;
        };
        let carrying = match body.carrying {
            Some(carrying) => carrying,
            None => {
                let stands = state.body_transform(wanted.body).translation;
                state.set_body_layers(wanted.body, CollisionLayers::NONE);
                let carrying = Carrying {
                    height: ground_under(&state, stands).map_or(0.0, |ground| stands.y - ground),
                    ..wanted
                };
                body.carrying = Some(carrying);
                carrying
            }
        };
        let stands = state.body_transform(carrying.body).translation;
        let ground = ground_under(&state, root.translation);
        let target = Vec3::new(
            root.translation.x,
            ground.map_or(stands.y, |ground| ground + carrying.height),
            root.translation.z,
        );
        state.set_body_translation(carrying.body, target);
        state.set_linear_velocity(carrying.body, Vec3::ZERO);
        let rest = body.resolved.settings().parts[0].pose.rotation;
        let update = Carried {
            velocity: state.linear_velocity(root_body),
            facing: yaw(root.rotation * rest.inverse()),
        };
        match carried.get_entity(carrying.capsule) {
            Some(mut carried) => **carried = update,
            None => cmd.insert(update, carrying.capsule),
        }
    }
}

/// After each physics step, keeps every ragdoll's last two poses, which the drawn pose is interpolated between.
pub fn record_ragdoll_poses(
    characters: Query<(&RagdollId, &mut RagdollBody)>,
    state: Res<PhysicsState>,
) {
    let mut pose = Vec::new();
    for (id, mut body) in characters.iter() {
        state.ragdoll_pose(*id, &mut pose);
        if pose.len() != body.current.len() {
            continue;
        }
        let body = &mut *body;
        std::mem::swap(&mut body.previous, &mut body.current);
        body.current.clone_from(&pose);
    }
}

fn blend(animated: Mat4, simulated: BodyPose, weight: f32) -> Mat4 {
    let (scale, rotation, translation) = animated.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(
        scale,
        rotation.slerp(simulated.rotation, weight),
        translation.lerp(simulated.translation, weight),
    )
}

/// Each frame, after the animation has written the bones and before transforms propagate, writes onto the bones the ragdoll's pose, interpolated between the last two fixed steps, by each part's weight. A bone with no part keeps its animated pose under its parent, so fingers and a face still animate on a limp body.
pub fn write_ragdoll_poses(
    characters: Query<(&CharacterRagdoll, &mut RagdollBody, &SkeletonComponent)>,
    skeletons: Res<AssetStore<Skeleton>>,
    globals: Query<&GlobalTransform>,
    locals: Query<&mut Transform>,
    time: Res<Time>,
) {
    let alpha = time.fixed_alpha();
    for (character, mut body, skeleton_component) in characters.iter() {
        let Some(skeleton) = skeletons.get(skeleton_component.skeleton()) else {
            continue;
        };
        let weights: Vec<f32> = (0..body.resolved.part_count())
            .map(|part| {
                body.bone_name(part)
                    .map_or(0.0, |bone| character.weight_of(&bone))
            })
            .collect();
        if weights.iter().all(|weight| *weight <= 0.0) {
            body.drawn_roots.iter_mut().for_each(|drawn| *drawn = None);
            continue;
        }
        let simulated = body.interpolated_pose(alpha);
        let mut part_of_bone: Vec<Option<usize>> = vec![None; body.skeleton_bones.len()];
        for (part, bone) in body.resolved.bones().iter().enumerate() {
            part_of_bone[*bone] = Some(part);
        }
        let mut world: Vec<Mat4> = Vec::with_capacity(body.skeleton_bones.len());
        for (bone, entity) in body.skeleton_bones.clone().into_iter().enumerate() {
            let Some(mut local) = locals.get_entity(entity) else {
                world.push(Mat4::IDENTITY);
                continue;
            };
            let animated_local = local.compute_matrix();
            let parent = match skeleton.parents[bone] {
                Some(parent) => world[parent as usize],
                None => {
                    let propagated = globals
                        .get_entity(entity)
                        .map_or(Mat4::IDENTITY, GlobalTransform::matrix);
                    propagated * body.drawn_roots[bone].unwrap_or(animated_local).inverse()
                }
            };
            let animated = parent * animated_local;
            let weight = part_of_bone[bone].map_or(0.0, |part| weights[part]);
            let drawn = match part_of_bone[bone] {
                Some(part) if weight > 0.0 => blend(animated, simulated[part], weight),
                _ => animated,
            };
            if weight > 0.0 {
                let (_, rotation, translation) =
                    (parent.inverse() * drawn).to_scale_rotation_translation();
                local.translation = translation;
                local.rotation = rotation.normalize();
            }
            if skeleton.parents[bone].is_none() {
                body.drawn_roots[bone] = Some(local.compute_matrix());
            }
            world.push(drawn);
        }
    }
}

fn drop_body(
    entity: Entity,
    bodies: &Query<&RagdollBody>,
    state: &mut PhysicsState,
    cmd: &mut CommandQueue,
) {
    if let Some(carrying) = bodies.get_entity(entity).and_then(|body| body.carrying) {
        state.set_body_layers(carrying.body, carrying.layers);
        cmd.remove::<Carried>(carrying.capsule);
    }
    cmd.remove::<RagdollId>(entity);
    cmd.remove::<RagdollBody>(entity);
    cmd.remove::<RagdollRefused>(entity);
}

/// Destroys a character's ragdoll when its `CharacterRagdoll` is removed.
pub fn drop_removed_ragdoll(
    on: On<Remove<CharacterRagdoll>>,
    bodies: Query<&RagdollBody>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    drop_body(on.signal().entity, &bodies, &mut state, &mut cmd);
}

/// Destroys a character's ragdoll when its `CharacterRagdoll` is inserted again, so it is rebuilt from the new one.
pub fn drop_replaced_ragdoll(
    on: On<Replace<CharacterRagdoll>>,
    bodies: Query<&RagdollBody>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    drop_body(on.signal().entity, &bodies, &mut state, &mut cmd);
}

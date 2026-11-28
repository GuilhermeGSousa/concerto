use std::sync::Arc;

use concerto_animation::player::AnimationPlayer;
use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res, ResMut, Without,
    component::scene::SceneEntityRef,
    entity::hierarchy::ChildOf,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::{
        EntitySignal, On, Signal,
        lifecycle::{Remove, Replace},
        listener::Listener,
    },
};
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::{GlobalTransform, Transform, parent_world_of},
};
use concerto_mesh::{Skeleton, SkeletonComponent};
use concerto_name::Name;
use concerto_physics::{
    body::BodyId,
    carried::Carried,
    layers::CollisionLayers,
    physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState,
    query::QueryFilter,
    ragdoll::{BodyPose, JointDrive, JointSpring, RagdollId},
    rigid_body::MotionType,
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Affine3A, Mat4, Quat, Vec3};
use log::warn;

use crate::{
    asset::{PartStrength, Ragdoll, StrengthProfile},
    dying::{Settled, mark_dead},
    messages::{BodyDead, PendingImpulses, Wounded, keep_wounds},
    resolve::ResolvedRagdoll,
    sensing::BodySense,
    shot::Wounds,
};

/// Triggered on a character that can no longer hold its pose.
///
/// That is when its root part has been further than `lost_distance` from its
/// animated place for `lost_seconds`.
pub struct PoseLost;

impl Signal for PoseLost {}
impl EntitySignal for PoseLost {}

/// Gives a character a physical body.
///
/// It goes on the entity with the `SkeletonComponent` and names the ragdoll
/// asset of that skeleton. The ragdoll is built at the current pose once both
/// have loaded.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(PendingImpulses, Wounds, BodySense, Listener<Wounded> = keep_wounds(), Listener<BodyDead> = mark_dead())]
pub struct CharacterRagdoll {
    /// The ragdoll asset of the skeleton.
    pub ragdoll: AssetHandle<Ragdoll>,
    /// The character's own capsule, carried under the body while the character is physical.
    ///
    /// It should have `CollisionLayers::character()` so the parts pass
    /// through it.
    pub capsule: SceneEntityRef,
    /// How far the root part may be from its animated place, in metres, before the pose counts as lost.
    pub lost_distance: f32,
    /// How long the root part must stay that far for `PoseLost` to be triggered.
    pub lost_seconds: f32,
    /// The strength profile of the ragdoll asset that holds each part to its animation for the share of it no node of the graph holds; none by default.
    pub profile: Name,
}

impl Default for CharacterRagdoll {
    fn default() -> Self {
        Self {
            ragdoll: AssetHandle::default(),
            capsule: SceneEntityRef::default(),
            lost_distance: 0.5,
            lost_seconds: 0.3,
            profile: Name::default(),
        }
    }
}

impl CharacterRagdoll {
    /// A ragdoll of `ragdoll`, which follows the animation until its graph draws it from physics.
    pub fn new(ragdoll: AssetHandle<Ragdoll>) -> Self {
        Self {
            ragdoll,
            ..Self::default()
        }
    }

    /// Sets the character's own capsule, carried under the body while the character is physical.
    pub fn with_capsule(mut self, capsule: Entity) -> Self {
        self.capsule = SceneEntityRef::Entity(capsule);
        self
    }

    /// Sets the strength profile that holds each part to its animation for the share of it no node of the graph holds.
    pub fn with_profile(mut self, profile: impl Into<Name>) -> Self {
        self.profile = profile.into();
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Held {
    pub(crate) kinematic: bool,
    pub(crate) stiffness: f32,
    pub(crate) damping: f32,
    pub(crate) torque_limit: f32,
}

const SLACK_TORQUE: f32 = 1.0e-3;
const MAX_LEAD: f32 = 0.5;

impl Held {
    pub(crate) const KINEMATIC: Self = Self {
        kinematic: true,
        stiffness: 0.0,
        damping: 0.0,
        torque_limit: 0.0,
    };

    pub(crate) fn of(strength: PartStrength) -> Self {
        match strength {
            PartStrength::Kinematic => Self::KINEMATIC,
            PartStrength::Powered {
                stiffness,
                damping,
                torque_limit,
            } => Self {
                kinematic: false,
                stiffness,
                damping,
                torque_limit,
            },
            PartStrength::Limp => Self {
                kinematic: false,
                ..Self::KINEMATIC
            },
        }
    }

    fn slack(&self) -> bool {
        self.torque_limit <= SLACK_TORQUE || self.stiffness <= 0.0
    }
}

/// A character's built ragdoll, beside its `RagdollId`: which bone each part follows.
#[derive(Component, Clone)]
pub struct RagdollBody {
    resolved: Arc<ResolvedRagdoll>,
    profiles: Arc<[StrengthProfile]>,
    bones: Vec<Entity>,
    skeleton_bones: Vec<Entity>,
    held: Vec<Held>,
    aimed: Vec<Quat>,
    slack: Vec<bool>,
    lost_for: f32,
    lost: bool,
    animated: Vec<BodyPose>,
    animated_model: Vec<Mat4>,
    part_roots: Vec<usize>,
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

    /// The bone entity the part follows.
    ///
    /// A `RayHit::part` of this character resolves through it.
    pub fn bone(&self, part: usize) -> Option<Entity> {
        self.bones.get(part).copied()
    }

    /// The name of the bone the part follows.
    pub fn bone_name(&self, part: usize) -> Option<Name> {
        self.resolved.bone_name(part)
    }

    /// Whether the part is kinematic right now.
    pub fn is_kinematic(&self, part: usize) -> bool {
        self.held.get(part).is_some_and(|held| held.kinematic)
    }

    pub(crate) fn shared_resolved(&self) -> &Arc<ResolvedRagdoll> {
        &self.resolved
    }

    pub(crate) fn profiles(&self) -> &[StrengthProfile] {
        &self.profiles
    }

    /// The world pose the animation asked of every part on the last frame, in part order, whatever was drawn.
    pub fn animated_pose(&self) -> &[BodyPose] {
        &self.animated
    }

    /// The bone entity of every part, in part order.
    pub fn bones(&self) -> &[Entity] {
        &self.bones
    }

    /// The world pose of every part after the last physics step, in part order.
    pub fn simulated_pose(&self) -> &[BodyPose] {
        &self.current
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

/// Marks a character whose ragdoll asset does not fit its skeleton, so it is reported once.
///
/// Inserting [`CharacterRagdoll`] again clears it.
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

/// Builds the ragdoll of every character whose ragdoll asset and skeleton have loaded.
///
/// The ragdoll is built at the bones' current pose, with every part
/// kinematic. Does nothing where physics is not stepped.
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
        let part_roots: Vec<usize> = resolved
            .bones()
            .iter()
            .map(|bone| {
                let mut root = *bone;
                while let Some(parent) = skeleton.parents[root] {
                    root = parent as usize;
                }
                root
            })
            .collect();
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
                    resolved: Arc::new(resolved),
                    profiles: ragdoll.profiles.as_slice().into(),
                    bones,
                    drawn_roots: vec![None; skeleton_bones.len()],
                    skeleton_bones,
                    held: vec![Held::KINEMATIC; pose.len()],
                    aimed: vec![Quat::IDENTITY; pose.len()],
                    slack: vec![false; pose.len()],
                    lost_for: 0.0,
                    lost: false,
                    animated: pose.clone(),
                    animated_model: vec![Mat4::IDENTITY; pose.len()],
                    part_roots,
                    previous: pose.clone(),
                    current: pose,
                    carrying: None,
                },
            ),
            entity,
        );
    }
}

impl RagdollBody {
    pub(crate) fn animated_targets(&self) -> Vec<Quat> {
        let animated = &self.animated;
        (0..self.resolved.part_count())
            .map(|part| match self.resolved.settings().parts[part].parent {
                Some(parent) => {
                    (animated[parent].rotation.inverse() * animated[part].rotation).normalize()
                }
                None => Quat::IDENTITY,
            })
            .collect()
    }

    pub(crate) fn track_lost(
        &mut self,
        character: &CharacterRagdoll,
        id: RagdollId,
        state: &PhysicsState,
        simulated: &mut Vec<BodyPose>,
        dt: f32,
    ) -> bool {
        if self.held.first().is_some_and(|root| root.kinematic) {
            self.lost_for = 0.0;
            self.lost = false;
            return false;
        }
        state.ragdoll_pose(id, simulated);
        let astray =
            simulated
                .first()
                .zip(self.animated.first())
                .is_some_and(|(root, animated)| {
                    root.translation.distance(animated.translation) > character.lost_distance
                });
        if !astray {
            self.lost_for = 0.0;
            self.lost = false;
            return false;
        }
        self.lost_for += dt;
        if !self.lost && self.lost_for >= character.lost_seconds {
            self.lost = true;
            return true;
        }
        false
    }
}

pub(crate) struct Asked {
    pub(crate) targets: Vec<Quat>,
    pub(crate) forces: Vec<Vec3>,
    pub(crate) torques: Vec<Vec3>,
}

pub(crate) struct Driven<'a> {
    pub(crate) id: RagdollId,
    pub(crate) animated: &'a [BodyPose],
    pub(crate) wanted: &'a [Held],
    pub(crate) asked: &'a Asked,
    pub(crate) dt: f32,
}

pub(crate) fn drive_part(
    driven: &Driven,
    part: usize,
    body: &mut RagdollBody,
    state: &mut ResMut<PhysicsState>,
) {
    let id = driven.id;
    let asked = driven.asked;
    let before = body.held[part];
    let held = driven.wanted[part];
    body.held[part] = held;
    if before.kinematic != held.kinematic {
        let motion = if held.kinematic {
            MotionType::Kinematic
        } else {
            MotionType::Dynamic
        };
        state.set_ragdoll_part_motion(id, part, motion);
    }
    if held.kinematic {
        body.slack[part] = false;
        let part_body = state.ragdoll_bodies(id)[part];
        state.move_body_kinematic(part_body, driven.animated[part]);
        body.aimed[part] = Quat::IDENTITY;
        return;
    }
    if asked.forces[part] != Vec3::ZERO || asked.torques[part] != Vec3::ZERO {
        let part_body = state.ragdoll_bodies(id)[part];
        state.add_force(part_body, asked.forces[part]);
        state.add_torque(part_body, asked.torques[part]);
    }
    if body.resolved.settings().parts[part].parent.is_none() {
        return;
    }
    if held.slack() {
        if !body.slack[part] {
            state.set_ragdoll_joint_drive(id, part, JointDrive::Off);
            body.slack[part] = true;
        }
        body.aimed[part] = Quat::IDENTITY;
        return;
    }
    body.slack[part] = false;
    let target = asked.targets[part];
    let before_target = if before.kinematic || before.slack() {
        target
    } else {
        body.aimed[part]
    };
    let turned = (target * before_target.inverse()).normalize();
    let turned = if turned.w < 0.0 { -turned } else { turned };
    let lead = held.damping / held.stiffness;
    let ahead = (turned.to_scaled_axis() / driven.dt * lead).clamp_length_max(MAX_LEAD);
    body.aimed[part] = target;
    state.set_ragdoll_joint_drive(
        id,
        part,
        JointDrive::Spring(JointSpring {
            rotation: (Quat::from_scaled_axis(ahead) * target).normalize(),
            stiffness: held.stiffness,
            damping: held.damping,
            torque_limit: held.torque_limit,
        }),
    );
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

fn heading(rotation: Quat) -> Quat {
    let across = (rotation * Vec3::X).cross(Vec3::Y);
    let on_its_side = (1.0 - across.length() / 0.3).max(0.0).powi(2);
    let ahead = across + (rotation * Vec3::Z).with_y(0.0) * on_its_side;
    if ahead.length_squared() < 1e-8 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_y(ahead.x.atan2(ahead.z))
    }
}

/// Carries each character's capsule under its ragdoll, each fixed step before physics steps.
///
/// While the character's root part is drawn from physics by any amount, its
/// capsule is `Carried`, is in no collision layer, and is put on the ground
/// under the ragdoll's root part. When the amount is back at 0 the capsule is
/// let go where it stands.
pub fn carry_capsules(
    mut characters: Query<
        (
            &CharacterRagdoll,
            &RagdollId,
            &mut RagdollBody,
            Option<&AnimationPlayer>,
        ),
        Without<Settled>,
    >,
    bodies: Query<(&BodyId, Option<&CollisionLayers>)>,
    mut carried: Query<&mut Carried>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let mut pose = Vec::new();
    for (character, id, mut body, player) in characters.iter_mut() {
        let physical = body.drawn_amount(player, 0) > 0.0;
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
            facing: heading(root.rotation * rest.inverse()),
        };
        match carried.get_entity_mut(carrying.capsule) {
            Some(mut carried) => **carried = update,
            None => cmd.insert(update, carrying.capsule),
        }
    }
}

/// After each physics step, keeps every ragdoll's last two poses, which the drawn pose is interpolated between.
pub fn record_ragdoll_poses(
    mut characters: Query<(&RagdollId, &mut RagdollBody)>,
    state: Res<PhysicsState>,
) {
    let mut pose = Vec::new();
    for (id, mut body) in characters.iter_mut() {
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

/// Writes each ragdoll's pose onto its bones, each frame after the animation has written the bones and before transforms propagate.
///
/// It notes the animated pose of every part, then writes the ragdoll's pose,
/// interpolated between the last two fixed steps, by its bone's
/// [`PhysicsAmount`](crate::PhysicsAmount) in the graph's drive pass. A bone
/// with no part keeps its animated pose under its parent, so fingers and a
/// face still animate on a limp body.
pub fn write_ragdoll_poses(
    mut characters: Query<(
        &mut RagdollBody,
        &SkeletonComponent,
        Option<&AnimationPlayer>,
    )>,
    skeletons: Res<AssetStore<Skeleton>>,
    globals: Query<&GlobalTransform>,
    mut locals: Query<&mut Transform>,
    parents: Query<&ChildOf>,
    time: Res<Time>,
) {
    let alpha = time.fixed_alpha();
    for (mut body, skeleton_component, player) in characters.iter_mut() {
        let Some(skeleton) = skeletons.get(skeleton_component.skeleton()) else {
            continue;
        };
        let weights: Vec<f32> = (0..body.resolved.part_count())
            .map(|part| body.drawn_amount(player, part))
            .collect();
        let simulated = body.interpolated_pose(alpha);
        let mut part_of_bone: Vec<Option<usize>> = vec![None; body.skeleton_bones.len()];
        for (part, bone) in body.resolved.bones().iter().enumerate() {
            part_of_bone[*bone] = Some(part);
        }
        let root_parents: Vec<Mat4> = body
            .skeleton_bones
            .iter()
            .enumerate()
            .map(|(bone, entity)| match skeleton.parents[bone] {
                Some(_) => Mat4::IDENTITY,
                None => Mat4::from(parent_world_of(*entity, &parents, &globals, |ancestor| {
                    locals
                        .get_entity_mut(ancestor)
                        .map(|local| Affine3A::from_mat4(local.compute_matrix()))
                })),
            })
            .collect();
        let mut world: Vec<Mat4> = Vec::with_capacity(body.skeleton_bones.len());
        let mut model: Vec<Mat4> = Vec::with_capacity(body.skeleton_bones.len());
        for (bone, entity) in body.skeleton_bones.clone().into_iter().enumerate() {
            let Some(mut local) = locals.get_entity_mut(entity) else {
                world.push(Mat4::IDENTITY);
                model.push(Mat4::IDENTITY);
                continue;
            };
            let animated_local = local.compute_matrix();
            let parent = match skeleton.parents[bone] {
                Some(parent) => {
                    model.push(model[parent as usize] * animated_local);
                    world[parent as usize]
                }
                None => {
                    model.push(animated_local);
                    root_parents[bone]
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
        let body = &mut *body;
        for (part, bone) in body.resolved.bones().iter().enumerate() {
            body.animated_model[part] = model[*bone];
        }
    }
}

/// Places every part's animated pose in the world, each frame after transforms have propagated.
///
/// The animated pose is the one the animation asked for, whatever was drawn.
pub fn record_animated_poses(
    mut characters: Query<&mut RagdollBody>,
    globals: Query<&GlobalTransform>,
) {
    for mut body in characters.iter_mut() {
        let body = &mut *body;
        for part in 0..body.animated.len() {
            let root = body.part_roots[part];
            let (Some(global), Some(drawn)) = (
                globals.get_entity(body.skeleton_bones[root]),
                body.drawn_roots[root],
            ) else {
                continue;
            };
            let (_, rotation, translation) =
                (global.matrix() * drawn.inverse() * body.animated_model[part])
                    .to_scale_rotation_translation();
            body.animated[part] = BodyPose::new(translation, rotation.normalize());
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

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use glam::{Quat, Vec3};

    use super::heading;

    #[test]
    fn a_body_heads_the_way_its_hips_point_however_it_lies() {
        let toward = 2.0;
        let turned = Quat::from_rotation_y(toward);
        let cases = [
            ("upright", turned),
            ("upright and leaning", turned * Quat::from_rotation_x(0.3)),
            ("on its back", turned * Quat::from_rotation_x(-FRAC_PI_2)),
            (
                "on its back, rocked about its spine",
                turned * Quat::from_rotation_x(-FRAC_PI_2) * Quat::from_rotation_y(0.6),
            ),
            (
                "on its front, rocked about its spine",
                turned * Quat::from_rotation_x(FRAC_PI_2) * Quat::from_rotation_y(-0.6),
            ),
            ("on its side", turned * Quat::from_rotation_z(FRAC_PI_2)),
        ];

        for (lying, rotation) in cases {
            let error = (heading(rotation) * Vec3::Z).distance(turned * Vec3::Z);
            assert!(error < 1e-4, "{lying}: heads {error} off");
        }
    }
}

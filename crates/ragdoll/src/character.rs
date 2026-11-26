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
use concerto_mesh::{JointName, Skeleton, SkeletonComponent};
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
    behaviour::{BodyDrive, Claim},
    drive::{drawn_amount, old_path_owns, physics_amount},
    dying::{Dying, Settled},
    generate::BRACED,
    hit::Flinch,
    messages::{PendingImpulses, Wounded, keep_wounds},
    resolve::ResolvedRagdoll,
    shot::Wounds,
};

/// What a character's ragdoll does.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Default)]
pub enum RagdollMode {
    /// Every part is kinematic and is driven to its bone each fixed step: the body pushes props and can be hit, and the animation is untouched.
    #[default]
    FollowsAnimation,
    /// Every part is dynamic with its motors off, and keeps the velocity it had: the body falls as a body.
    Limp,
    /// Each part is held to the animated pose as the character's strength profile says: kinematic, powered by its joint's motor, or limp. The body tries to do what the animation says and physics decides how well it manages.
    Powered,
}

/// Triggered on a character whose root part has been further than `lost_distance` from its animated place for `lost_seconds`: it can no longer hold the pose.
pub struct PoseLost;

impl Signal for PoseLost {}
impl EntitySignal for PoseLost {}

/// How much of one part's simulated pose is drawn, overriding the character's `weight` for the part on `bone`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Default)]
pub struct PartWeight {
    pub bone: JointName,
    pub weight: f32,
}

/// Gives a character a physical body: goes on the entity with the `SkeletonComponent`, names the ragdoll asset of that skeleton, and builds the ragdoll at the current pose once both have loaded.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(PendingImpulses, Wounds, Listener<Wounded> = keep_wounds())]
pub struct CharacterRagdoll {
    pub ragdoll: AssetHandle<Ragdoll>,
    /// The character's own capsule, carried under the body while the character is physical; it should have `CollisionLayers::character()` so the parts pass through it.
    pub capsule: SceneEntityRef,
    pub mode: RagdollMode,
    /// How much of the simulated pose is drawn on the skeleton: 0 is the animated pose, 1 the ragdoll's.
    pub weight: f32,
    /// Weights of single parts, each replacing `weight` for its part.
    pub part_weights: Vec<PartWeight>,
    /// The strength profile of the ragdoll asset used in the powered mode.
    pub profile: Name,
    /// The seconds over which a change of profile takes hold.
    pub blend: f32,
    /// How far the root part may be from its animated place, in metres, before the pose counts as lost.
    pub lost_distance: f32,
    /// How long the root part must stay that far for `PoseLost` to be triggered.
    pub lost_seconds: f32,
    /// The strength profile of the ragdoll asset that holds each part to its animation where no behaviour of the graph holds it.
    pub follow: Name,
}

impl Default for CharacterRagdoll {
    fn default() -> Self {
        Self {
            ragdoll: AssetHandle::default(),
            capsule: SceneEntityRef::default(),
            mode: RagdollMode::default(),
            weight: 0.0,
            part_weights: Vec::new(),
            profile: Name::default(),
            blend: 0.2,
            lost_distance: 0.5,
            lost_seconds: 0.3,
            follow: Name::from(BRACED),
        }
    }
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

    pub fn with_profile(mut self, profile: impl Into<Name>) -> Self {
        self.profile = profile.into();
        self
    }

    pub fn with_follow(mut self, follow: impl Into<Name>) -> Self {
        self.follow = follow.into();
        self
    }

    /// Holds the body to the animation with the strength profile `profile`, and draws the simulated pose.
    pub fn power(&mut self, profile: impl Into<Name>) {
        self.mode = RagdollMode::Powered;
        self.profile = profile.into();
        self.weight = 1.0;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Held {
    pub(crate) kinematic: bool,
    pub(crate) stiffness: f32,
    pub(crate) damping: f32,
    pub(crate) torque_limit: f32,
}

const SLACK_TORQUE: f32 = 1.0e-3;
const MAX_LEAD: f32 = 0.5;
const DYING_STIFFNESS: f32 = 400.0;
const DYING_TORQUE: f32 = 120.0;

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

    fn towards(self, wanted: Self, step: f32) -> Self {
        if self.kinematic || wanted.kinematic || step >= 1.0 {
            return wanted;
        }
        let from = if self.slack() {
            Self {
                stiffness: wanted.stiffness,
                damping: wanted.damping,
                ..self
            }
        } else {
            self
        };
        let to = if wanted.slack() {
            Self {
                stiffness: from.stiffness,
                damping: from.damping,
                ..wanted
            }
        } else {
            wanted
        };
        Self {
            kinematic: false,
            stiffness: from.stiffness + (to.stiffness - from.stiffness) * step,
            damping: from.damping + (to.damping - from.damping) * step,
            torque_limit: from.torque_limit + (to.torque_limit - from.torque_limit) * step,
        }
    }
}

/// A character's built ragdoll, beside its `RagdollId`: which bone each part follows.
#[derive(Component, Clone)]
pub struct RagdollBody {
    resolved: Arc<ResolvedRagdoll>,
    bones: Vec<Entity>,
    skeleton_bones: Vec<Entity>,
    held: Vec<Held>,
    aimed: Vec<Quat>,
    slack: Vec<bool>,
    changing: Vec<bool>,
    lost_for: f32,
    lost: bool,
    animated: Vec<BodyPose>,
    animated_model: Vec<Mat4>,
    part_roots: Vec<usize>,
    previous: Vec<BodyPose>,
    current: Vec<BodyPose>,
    velocities: Vec<Vec3>,
    part_of_bone: Arc<[Option<usize>]>,
    profiles: Arc<[StrengthProfile]>,
    drawn: Vec<f32>,
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

    /// Whether the part is kinematic right now.
    pub fn is_kinematic(&self, part: usize) -> bool {
        self.held.get(part).is_some_and(|held| held.kinematic)
    }

    pub(crate) fn is_changing(&self, part: usize) -> bool {
        self.changing.get(part).copied().unwrap_or(false)
    }

    /// The world pose the animation asked of every part on the last frame, in part order, whatever was drawn.
    pub fn animated_pose(&self) -> &[BodyPose] {
        &self.animated
    }

    /// The bone entity of every part, in part order.
    pub fn bones(&self) -> &[Entity] {
        &self.bones
    }

    /// Whether the body is drawn wholly from the animation: no bone has a physics amount in `player`'s graph, and no part was drawn from physics by its weight on the last frame.
    pub fn is_animated(&self, player: &AnimationPlayer) -> bool {
        self.drawn.iter().all(|amount| *amount <= 0.0)
            && self
                .resolved
                .bones()
                .iter()
                .all(|bone| physics_amount(Some(player), *bone) <= 0.0)
    }

    pub(crate) fn shared_resolved(&self) -> Arc<ResolvedRagdoll> {
        Arc::clone(&self.resolved)
    }

    pub(crate) fn shared_part_of_bone(&self) -> Arc<[Option<usize>]> {
        Arc::clone(&self.part_of_bone)
    }

    pub(crate) fn shared_profiles(&self) -> Arc<[StrengthProfile]> {
        Arc::clone(&self.profiles)
    }

    pub(crate) fn simulated_pose(&self) -> &[BodyPose] {
        &self.current
    }

    pub(crate) fn velocities(&self) -> &[Vec3] {
        &self.velocities
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
        let mut part_of_bone = vec![None; skeleton_bones.len()];
        for (part, bone) in resolved.bones().iter().enumerate() {
            part_of_bone[*bone] = Some(part);
        }
        cmd.insert(
            (
                id,
                RagdollBody {
                    resolved: Arc::new(resolved),
                    bones,
                    drawn_roots: vec![None; skeleton_bones.len()],
                    skeleton_bones,
                    held: vec![Held::KINEMATIC; pose.len()],
                    aimed: vec![Quat::IDENTITY; pose.len()],
                    slack: vec![false; pose.len()],
                    changing: vec![false; pose.len()],
                    lost_for: 0.0,
                    lost: false,
                    animated: pose.clone(),
                    animated_model: vec![Mat4::IDENTITY; pose.len()],
                    part_roots,
                    velocities: vec![Vec3::ZERO; pose.len()],
                    part_of_bone: part_of_bone.into(),
                    profiles: ragdoll.profiles.as_slice().into(),
                    drawn: vec![0.0; pose.len()],
                    previous: pose.clone(),
                    current: pose,
                    carrying: None,
                },
            ),
            entity,
        );
    }
}

fn wanted_strengths(
    character: &CharacterRagdoll,
    body: &RagdollBody,
    ragdoll: Option<&Ragdoll>,
) -> Vec<Held> {
    let parts = body.resolved.part_count();
    match character.mode {
        RagdollMode::FollowsAnimation => vec![Held::KINEMATIC; parts],
        RagdollMode::Limp => vec![Held::of(PartStrength::Limp); parts],
        RagdollMode::Powered => {
            let profile = ragdoll.and_then(|ragdoll| ragdoll.profile(&character.profile));
            (0..parts)
                .map(|part| {
                    let strength = match (profile, body.bone_name(part)) {
                        (Some(profile), Some(bone)) => profile.strength_of(&bone),
                        _ => PartStrength::Kinematic,
                    };
                    Held::of(strength)
                })
                .collect()
        }
    }
}

/// Each fixed step, before physics steps, holds every ragdoll's parts to the animated pose as its mode and strength profile ask: a kinematic part is driven to its bone, a powered part's motor is aimed at its bone's rotation under its parent's, leading it by the animation's own speed, and a limp part's motor is off. A change of strength takes hold over the character's `blend`. Triggers `PoseLost` when the root part has been too far from its animated place for too long. A character that follows its animation and that no behaviour claims is left to `drive_from_graph`.
pub fn drive_character_ragdolls(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &RagdollId,
            &mut RagdollBody,
            Option<&Flinch>,
            Option<&mut BodyDrive>,
            Option<&Dying>,
        ),
        Without<Settled>,
    >,
    ragdolls: Res<AssetStore<Ragdoll>>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut simulated = Vec::new();
    for (entity, character, id, mut body, flinch, drive, dying) in characters.iter_mut() {
        let claims = drive.map(|mut drive| drive.take()).unwrap_or_default();
        if !old_path_owns(character, !claims.is_empty()) {
            continue;
        }
        let animated = body.animated.clone();
        let mut wanted = wanted_strengths(character, &body, ragdolls.get(&character.ragdoll));
        if let Some(flinch) = flinch {
            for (held, scale) in wanted.iter_mut().zip(flinch.scales()) {
                held.stiffness *= scale;
                held.damping *= scale;
                held.torque_limit *= scale;
            }
        }
        let step = if character.blend > 0.0 && flinch.is_none() && dying.is_none() {
            dt / character.blend
        } else {
            1.0
        };
        let count = body.resolved.part_count();
        let mut asked = Asked {
            targets: body.animated_targets(),
            forces: vec![Vec3::ZERO; count],
            torques: vec![Vec3::ZERO; count],
            claimed: vec![false; count],
        };
        apply_claims(&claims, &mut wanted, &mut asked);
        if let Some(dying) = dying {
            drain_strengths(dying, &body.resolved, &mut wanted, &mut asked);
        }
        body.note_changes(&wanted);
        let body = &mut *body;
        let driven = Driven {
            id: *id,
            animated: &animated,
            wanted: &wanted,
            asked: &asked,
            step,
            dt,
        };
        for part in 0..body.resolved.settings().parts.len() {
            drive_part(&driven, part, body, &mut state);
        }
        if body.track_lost(character, *id, &state, &mut simulated, dt) {
            cmd.entity(entity).trigger(PoseLost);
        }
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

    pub(crate) fn note_changes(&mut self, wanted: &[Held]) {
        for (part, held) in wanted.iter().enumerate() {
            self.changing[part] = held.kinematic != self.held[part].kinematic;
        }
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
    pub(crate) claimed: Vec<bool>,
}

pub(crate) struct Driven<'a> {
    pub(crate) id: RagdollId,
    pub(crate) animated: &'a [BodyPose],
    pub(crate) wanted: &'a [Held],
    pub(crate) asked: &'a Asked,
    pub(crate) step: f32,
    pub(crate) dt: f32,
}

fn apply_claims(claims: &[Claim], wanted: &mut [Held], asked: &mut Asked) {
    let count = wanted.len();
    for claim in claims {
        if claim.part >= count || claim.weight <= 0.0 {
            continue;
        }
        let strength = Held::of(claim.command.strength);
        wanted[claim.part] = if strength.kinematic || wanted[claim.part].kinematic {
            if claim.weight >= 0.5 {
                strength
            } else {
                wanted[claim.part]
            }
        } else {
            wanted[claim.part].towards(strength, claim.weight)
        };
        if let Some(target) = claim.command.target {
            asked.targets[claim.part] = asked.targets[claim.part]
                .slerp(target, claim.weight)
                .normalize();
        }
        asked.forces[claim.part] += claim.command.force * claim.weight;
        asked.torques[claim.part] += claim.command.torque * claim.weight;
        asked.claimed[claim.part] = true;
    }
}

fn drain_strengths(
    dying: &Dying,
    resolved: &ResolvedRagdoll,
    wanted: &mut [Held],
    asked: &mut Asked,
) {
    for (part, held) in wanted.iter_mut().enumerate() {
        let left = dying.left(resolved.segment(part));
        if held.kinematic {
            *held = Held::of(PartStrength::powered(DYING_STIFFNESS, DYING_TORQUE));
        }
        held.stiffness *= left;
        held.damping *= left;
        held.torque_limit *= left;
        asked.forces[part] *= left;
        asked.torques[part] *= left;
    }
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
    let blend = if asked.claimed[part] {
        1.0
    } else {
        driven.step
    };
    let held = before.towards(driven.wanted[part], blend);
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

/// Each fixed step, before physics steps, while a character's root part is drawn from physics by any amount: its capsule is `Carried`, is in no collision layer, and is put on the ground under the ragdoll's root part. When the amount is back at 0 the capsule is let go where it stands.
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
        let physical = drawn_amount(character, player, &body, 0) > 0.0;
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

/// After each physics step, keeps every ragdoll's last two poses, which the drawn pose is interpolated between, and its parts' velocities.
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
        for (velocity, part) in body.velocities.iter_mut().zip(state.ragdoll_bodies(*id)) {
            *velocity = state.linear_velocity(*part);
        }
    }
}

struct Drawn {
    animated: Mat4,
    under_parent: Mat4,
    root: bool,
}

fn blend(drawn: Drawn, simulated: BodyPose, weight: f32) -> Mat4 {
    let (scale, rotation, translation) = drawn.animated.to_scale_rotation_translation();
    let translation = if drawn.root {
        translation.lerp(simulated.translation, weight)
    } else {
        drawn.under_parent.w_axis.truncate()
    };
    Mat4::from_scale_rotation_translation(
        scale,
        rotation.slerp(simulated.rotation, weight),
        translation,
    )
}

/// Each frame, after the animation has written the bones and before transforms propagate, notes the animated pose of every part and writes onto the bones the ragdoll's pose, interpolated between the last two fixed steps, by each bone's `PhysicsAmount` in the graph or its part's weight, whichever is larger: a bone's world rotation is slerped from its own animated one toward its part's, and it keeps its animated offset from its drawn parent, so no bone stretches; only the root part's place is blended. A bone with no part keeps its animated pose under its drawn parent, so fingers and a face still animate on a limp body.
pub fn write_ragdoll_poses(
    mut characters: Query<(
        &CharacterRagdoll,
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
    for (character, mut body, skeleton_component, player) in characters.iter_mut() {
        let Some(skeleton) = skeletons.get(skeleton_component.skeleton()) else {
            continue;
        };
        let weights: Vec<f32> = (0..body.resolved.part_count())
            .map(|part| drawn_amount(character, player, &body, part))
            .collect();
        let simulated = body.interpolated_pose(alpha);
        let part_of_bone = body.shared_part_of_bone();
        let root_parts: Vec<bool> = body
            .resolved
            .settings()
            .parts
            .iter()
            .map(|part| part.parent.is_none())
            .collect();
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
        let mut chain_roots: Vec<Mat4> = Vec::with_capacity(body.skeleton_bones.len());
        for (bone, entity) in body.skeleton_bones.clone().into_iter().enumerate() {
            let Some(mut local) = locals.get_entity_mut(entity) else {
                world.push(Mat4::IDENTITY);
                model.push(Mat4::IDENTITY);
                chain_roots.push(Mat4::IDENTITY);
                continue;
            };
            let animated_local = local.compute_matrix();
            let parent = match skeleton.parents[bone] {
                Some(parent) => {
                    model.push(model[parent as usize] * animated_local);
                    chain_roots.push(chain_roots[parent as usize]);
                    world[parent as usize]
                }
                None => {
                    model.push(animated_local);
                    chain_roots.push(root_parents[bone]);
                    root_parents[bone]
                }
            };
            let weight = part_of_bone[bone].map_or(0.0, |part| weights[part]);
            let under_parent = parent * animated_local;
            let drawn = match part_of_bone[bone] {
                Some(part) if weight > 0.0 => blend(
                    Drawn {
                        animated: chain_roots[bone] * model[bone],
                        under_parent,
                        root: root_parts[part],
                    },
                    simulated[part],
                    weight,
                ),
                _ => under_parent,
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
        body.drawn = weights;
    }
}

/// Each frame, after transforms have propagated, places every part's animated pose in the world: the pose the animation asked for, whatever was drawn.
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

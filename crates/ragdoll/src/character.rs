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
    transform::GlobalTransform,
};
use concerto_mesh::{Skeleton, SkeletonComponent};
use concerto_name::Name;
use concerto_physics::{
    body::BodyId,
    physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState,
    ragdoll::{BodyPose, RagdollId},
    rigid_body::MotionType,
};
use concerto_reflect::{Reflect, ReflectDefault};
use log::warn;

use crate::{asset::Ragdoll, resolve::ResolvedRagdoll};

/// What a character's ragdoll does.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Default)]
pub enum RagdollMode {
    /// Every part is kinematic and is driven to its bone each fixed step: the body pushes props and can be hit, and the animation is untouched.
    #[default]
    FollowsAnimation,
}

/// Gives a character a physical body: goes on the entity with the `SkeletonComponent`, names the ragdoll asset of that skeleton, and builds the ragdoll at the current pose once both have loaded.
#[derive(Component, Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct CharacterRagdoll {
    pub ragdoll: AssetHandle<Ragdoll>,
    /// The character's own capsule, which its parts never collide with.
    pub capsule: SceneEntityRef,
    pub mode: RagdollMode,
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
}

/// A character's built ragdoll, beside its `RagdollId`: which bone each part follows.
#[derive(Component, Clone)]
pub struct RagdollBody {
    resolved: ResolvedRagdoll,
    bones: Vec<Entity>,
    ignored: Option<BodyId>,
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
        let Some(bones) = resolved
            .bones()
            .iter()
            .map(|bone| {
                skeleton_component
                    .bones()
                    .get(*bone)
                    .and_then(|bone| bone.entity())
            })
            .collect::<Option<Vec<Entity>>>()
        else {
            continue;
        };
        let Some(pose) = part_poses(&bones, &transforms) else {
            continue;
        };
        let Some(id) = state.create_ragdoll(resolved.built(), entity, &pose) else {
            continue;
        };
        for part in 0..resolved.part_count() {
            state.set_ragdoll_part_motion(id, part, MotionType::Kinematic);
        }
        cmd.insert(
            (
                id,
                RagdollBody {
                    resolved,
                    bones,
                    ignored: None,
                },
            ),
            entity,
        );
    }
}

/// Each fixed step, before physics steps, drives the parts of every ragdoll that follows its animation to its bones' poses, and keeps the parts from colliding with the character's capsule.
pub fn follow_animation(
    characters: Query<(&CharacterRagdoll, &RagdollId, &mut RagdollBody)>,
    transforms: Query<&GlobalTransform>,
    bodies: Query<&BodyId>,
    mut state: ResMut<PhysicsState>,
) {
    for (character, id, mut body) in characters.iter() {
        let capsule = character
            .capsule
            .entity()
            .and_then(|capsule| bodies.get_entity(capsule))
            .copied();
        if body.ignored != capsule {
            state.set_ragdoll_ignored_body(*id, capsule);
            body.ignored = capsule;
        }
        match character.mode {
            RagdollMode::FollowsAnimation => {
                if let Some(pose) = part_poses(&body.bones, &transforms) {
                    state.drive_ragdoll_kinematic(*id, &pose);
                }
            }
        }
    }
}

fn drop_body(entity: Entity, cmd: &mut CommandQueue) {
    cmd.remove::<RagdollId>(entity);
    cmd.remove::<RagdollBody>(entity);
    cmd.remove::<RagdollRefused>(entity);
}

/// Destroys a character's ragdoll when its `CharacterRagdoll` is removed.
pub fn drop_removed_ragdoll(on: On<Remove<CharacterRagdoll>>, mut cmd: CommandQueue) {
    drop_body(on.signal().entity, &mut cmd);
}

/// Destroys a character's ragdoll when its `CharacterRagdoll` is inserted again, so it is rebuilt from the new one.
pub fn drop_replaced_ragdoll(on: On<Replace<CharacterRagdoll>>, mut cmd: CommandQueue) {
    drop_body(on.signal().entity, &mut cmd);
}

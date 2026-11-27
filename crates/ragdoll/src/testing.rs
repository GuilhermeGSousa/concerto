//! A reference humanoid built in code, for tests and benchmarks of everything that stands on a ragdoll.

use concerto_animation::retarget::{RetargetChain, RetargetRig};
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_ecs::entity::Entity;
use concerto_ecs::entity::hierarchy::ChildOf;
use concerto_ecs::world::World;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton, SkeletonComponent};
use glam::{Mat4, Quat, Vec3};

use crate::asset::Ragdoll;
use crate::generate::{HumanoidSettings, generate_humanoid};
use crate::resolve::ResolvedRagdoll;
pub use crate::sandbox::{PlacedRagdoll, RagdollWorld, joint_gaps};

struct Bone {
    name: &'static str,
    parent: Option<&'static str>,
    at: Vec3,
}

const fn bone(name: &'static str, parent: &'static str, x: f32, y: f32, z: f32) -> Bone {
    Bone {
        name,
        parent: Some(parent),
        at: Vec3::new(x, y, z),
    }
}

const BONES: [Bone; 25] = [
    Bone {
        name: "root",
        parent: None,
        at: Vec3::ZERO,
    },
    bone("pelvis", "root", 0.0, 1.0, 0.0),
    bone("spine_01", "pelvis", 0.0, 1.1, 0.0),
    bone("spine_02", "spine_01", 0.0, 1.25, 0.0),
    bone("spine_03", "spine_02", 0.0, 1.4, 0.0),
    bone("neck_01", "spine_03", 0.0, 1.55, 0.0),
    bone("Head", "neck_01", 0.0, 1.65, 0.0),
    bone("clavicle_l", "spine_03", 0.05, 1.5, 0.0),
    bone("upperarm_l", "clavicle_l", 0.2, 1.5, 0.0),
    bone("lowerarm_l", "upperarm_l", 0.48, 1.5, 0.0),
    bone("hand_l", "lowerarm_l", 0.74, 1.5, 0.0),
    bone("index_l", "hand_l", 0.82, 1.5, 0.0),
    bone("clavicle_r", "spine_03", -0.05, 1.5, 0.0),
    bone("upperarm_r", "clavicle_r", -0.2, 1.5, 0.0),
    bone("lowerarm_r", "upperarm_r", -0.48, 1.5, 0.0),
    bone("hand_r", "lowerarm_r", -0.74, 1.5, 0.0),
    bone("index_r", "hand_r", -0.82, 1.5, 0.0),
    bone("thigh_l", "pelvis", 0.1, 0.95, 0.0),
    bone("calf_l", "thigh_l", 0.1, 0.52, 0.0),
    bone("foot_l", "calf_l", 0.1, 0.08, 0.0),
    bone("ball_l", "foot_l", 0.1, 0.02, 0.12),
    bone("thigh_r", "pelvis", -0.1, 0.95, 0.0),
    bone("calf_r", "thigh_r", -0.1, 0.52, 0.0),
    bone("foot_r", "calf_r", -0.1, 0.08, 0.0),
    bone("ball_r", "foot_r", -0.1, 0.02, 0.12),
];

fn index_of(name: &str) -> usize {
    BONES
        .iter()
        .position(|bone| bone.name == name)
        .expect("a reference bone names an earlier bone as its parent")
}

/// A 25-bone humanoid skeleton standing in a T-pose at the origin, 1.65 m to the head bone, facing +Z with its left side at +X.
///
/// # Panics
///
/// Panics if the built-in bone table is malformed: a bone names a parent the table does not
/// hold, or the bones do not form a valid skeleton. The table is fixed and well formed.
pub fn humanoid_skeleton() -> Skeleton {
    let parents: Vec<Option<u16>> = BONES
        .iter()
        .map(|bone| bone.parent.map(|parent| index_of(parent) as u16))
        .collect();
    let rest = BONES
        .iter()
        .map(|bone| JointPose {
            translation: bone.at
                - bone
                    .parent
                    .map_or(Vec3::ZERO, |parent| BONES[index_of(parent)].at),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        })
        .collect();
    Skeleton::new(
        BONES
            .iter()
            .map(|bone| Mat4::from_translation(-bone.at))
            .collect(),
        parents,
        BONES.iter().map(|bone| bone.name.into()).collect(),
        rest,
        BONES
            .iter()
            .map(|bone| uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, bone.name.as_bytes()))
            .collect(),
    )
    .expect("the reference skeleton is well formed")
}

/// The retarget rig of [`humanoid_skeleton`], with the chain names the generator looks for by default.
pub fn humanoid_rig() -> RetargetRig {
    let chain = |name: &str, start: &str, end: &str| RetargetChain {
        name: name.into(),
        start: start.into(),
        end: end.into(),
    };
    RetargetRig::new()
        .with_root("pelvis")
        .with_chain(chain("spine", "spine_01", "spine_03"))
        .with_chain(chain("neck", "neck_01", "Head"))
        .with_chain(chain("arm_l", "clavicle_l", "hand_l"))
        .with_chain(chain("arm_r", "clavicle_r", "hand_r"))
        .with_chain(chain("leg_l", "thigh_l", "ball_l"))
        .with_chain(chain("leg_r", "thigh_r", "ball_r"))
}

/// The reference humanoid: its skeleton, the ragdoll generated for it, and that ragdoll fitted and built.
pub struct ReferenceHumanoid {
    /// The skeleton, built in code.
    pub skeleton: Skeleton,
    /// The ragdoll asset generated for the skeleton.
    pub ragdoll: Ragdoll,
    /// The ragdoll fitted to the skeleton and built.
    pub resolved: ResolvedRagdoll,
}

/// Builds the reference humanoid of 75 kg.
///
/// # Panics
///
/// Panics if the reference rig does not resolve against the reference skeleton, the generator
/// rejects it, or the generated ragdoll does not fit the skeleton; none of these happens with
/// the built-in tables.
pub fn reference_humanoid() -> ReferenceHumanoid {
    let skeleton = humanoid_skeleton();
    let rig = humanoid_rig()
        .resolve(&skeleton, None)
        .expect("the reference rig fits the reference skeleton");
    let ragdoll = generate_humanoid(
        AssetHandle::default(),
        &skeleton,
        &rig,
        &[],
        &HumanoidSettings::default(),
    )
    .expect("the reference rig has every chain a humanoid needs");
    let resolved = ragdoll
        .resolve(&skeleton)
        .expect("the generated ragdoll fits the reference skeleton");
    ReferenceHumanoid {
        skeleton,
        ragdoll,
        resolved,
    }
}

/// The reference humanoid's skeleton as entities: one per bone under a skeleton entity that holds the `SkeletonComponent`.
#[derive(Clone, Debug)]
pub struct SpawnedHumanoid {
    /// The entity that holds the `SkeletonComponent`.
    pub skeleton: Entity,
    /// One entity per bone, in the skeleton's joint order.
    pub bones: Vec<Entity>,
}

impl SpawnedHumanoid {
    /// The entity of the bone called `name`.
    ///
    /// # Panics
    ///
    /// Panics when the reference humanoid has no bone called `name`.
    pub fn bone(&self, name: &str) -> Entity {
        self.bones[index_of(name)]
    }
}

/// Spawns the reference humanoid's bones in their rest pose under a new entity at `root`, which gets a `SkeletonComponent` naming `skeleton`.
pub fn spawn_humanoid(
    world: &mut World,
    skeleton: AssetHandle<Skeleton>,
    root: Transform,
) -> SpawnedHumanoid {
    let rest = humanoid_skeleton();
    let entity = world.spawn(root);
    let mut bones: Vec<Entity> = Vec::with_capacity(BONES.len());
    for (index, bone) in BONES.iter().enumerate() {
        let parent = bone.parent.map_or(entity, |parent| bones[index_of(parent)]);
        let local = &rest.rest_pose[index];
        bones.push(world.spawn((
            Transform::from_translation_rotation(local.translation, local.rotation),
            ChildOf::new(parent),
        )));
    }
    world.insert(
        SkeletonComponent::new(
            skeleton,
            bones.iter().copied().map(SceneEntityRef::Entity).collect(),
            bones.first().copied().map(SceneEntityRef::Entity),
        ),
        entity,
    );
    SpawnedHumanoid {
        skeleton: entity,
        bones,
    }
}

//! A reference humanoid built in code, for tests and benchmarks of everything that stands on a ragdoll.

use concerto_animation::retarget::{RetargetChain, RetargetRig};
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_ecs::entity::Entity;
use concerto_ecs::entity::hierarchy::ChildOf;
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::world::World;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::time::Time;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton, SkeletonComponent};
use concerto_physics::collider::{Collider, register_colliders};
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::ragdoll::{BodyPose, RagdollId};
use glam::{Mat4, Quat, Vec3};

use crate::asset::Ragdoll;
use crate::generate::{HumanoidSettings, generate_humanoid};
use crate::resolve::ResolvedRagdoll;

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
    pub skeleton: Skeleton,
    pub ragdoll: Ragdoll,
    pub resolved: ResolvedRagdoll,
}

/// Builds the reference humanoid of 75 kg.
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

/// A physics world with a floor whose top is at y = 0, to stand ragdolls in without an `App`.
pub struct RagdollWorld {
    pub world: World,
    pipeline: PhysicsPipeline,
}

/// A ragdoll placed in a [`RagdollWorld`].
#[derive(Clone, Copy, Debug)]
pub struct PlacedRagdoll {
    pub id: RagdollId,
    pub owner: Entity,
}

impl RagdollWorld {
    /// A world holding only the floor, 400 m across.
    pub fn new() -> Self {
        let mut world = World::new();
        world.register_component::<Collider>();
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        world.spawn((
            Collider::cuboid(200.0, 0.5, 200.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        ));
        let mut world = Self {
            world,
            pipeline: PhysicsPipeline::new(),
        };
        world.register_bodies();
        world
    }

    /// Creates the bodies of the colliders spawned since the last call.
    pub fn register_bodies(&mut self) {
        let mut schedule = Schedule::new();
        schedule.add_system(register_colliders);
        schedule
            .compile::<SingleThreadedExecutor>(&mut self.world)
            .run(&mut self.world);
    }

    pub fn state(&mut self) -> &mut PhysicsState {
        self.world
            .get_resource_mut::<PhysicsState>()
            .expect("the world was built with a PhysicsState")
    }

    /// Places `ragdoll` in its rest pose moved by `root`, owned by a new entity.
    pub fn place(&mut self, ragdoll: &ResolvedRagdoll, root: BodyPose) -> PlacedRagdoll {
        let owner = self.world.spawn(Transform::default());
        let pose: Vec<BodyPose> = ragdoll
            .settings()
            .parts
            .iter()
            .map(|part| root.then(part.pose))
            .collect();
        let id = self
            .state()
            .create_ragdoll(ragdoll.built(), owner, &pose)
            .expect("the physics backend creates ragdolls");
        PlacedRagdoll { id, owner }
    }

    /// Applies `impulse`, in N·s, at the centre of mass of the part on the bone called `bone`.
    pub fn impulse(
        &mut self,
        ragdoll: &ResolvedRagdoll,
        placed: PlacedRagdoll,
        bone: &str,
        impulse: Vec3,
    ) {
        let part = ragdoll
            .part_of(bone)
            .unwrap_or_else(|| panic!("the ragdoll has no part on `{bone}`"));
        let body = self.state().ragdoll_bodies(placed.id)[part];
        self.state().add_impulse(body, impulse);
    }

    /// Steps the simulation once.
    pub fn step(&mut self) {
        let Self { world, pipeline } = self;
        pipeline.step(
            world
                .get_resource_mut::<PhysicsState>()
                .expect("the world was built with a PhysicsState"),
        );
    }

    /// Steps the simulation for `seconds` of fixed steps.
    pub fn step_for(&mut self, seconds: f32) {
        let steps = (seconds / Time::fixed_delta_time().as_secs_f32()).round() as usize;
        for _ in 0..steps {
            self.step();
        }
    }

    /// The world pose of every part of `placed`.
    pub fn pose(&mut self, placed: PlacedRagdoll) -> Vec<BodyPose> {
        let mut pose = Vec::new();
        self.state().ragdoll_pose(placed.id, &mut pose);
        pose
    }

    /// The speed of each part of `placed`, in m/s.
    pub fn speeds(&mut self, placed: PlacedRagdoll) -> Vec<f32> {
        let state = self.state();
        state
            .ragdoll_bodies(placed.id)
            .iter()
            .map(|body| state.linear_velocity(*body).length())
            .collect()
    }
}

impl Default for RagdollWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// How far each part of `pose` but the root is from where its joint holds it on its parent, in metres, in part order.
pub fn joint_gaps(ragdoll: &ResolvedRagdoll, pose: &[BodyPose]) -> Vec<f32> {
    let parts = &ragdoll.settings().parts;
    parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| {
            let parent = part.parent?;
            let anchor = parts[parent]
                .pose
                .inverse()
                .transform_point(part.pose.translation);
            Some(
                pose[parent]
                    .transform_point(anchor)
                    .distance(pose[index].translation),
            )
        })
        .collect()
}

/// The reference humanoid's skeleton as entities: one per bone under a skeleton entity that holds the `SkeletonComponent`.
#[derive(Clone, Debug)]
pub struct SpawnedHumanoid {
    pub skeleton: Entity,
    pub bones: Vec<Entity>,
}

impl SpawnedHumanoid {
    /// The entity of the bone called `name`.
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

//! A physics world of its own to try a ragdoll in, without an `App`: for tests, benchmarks and the editor's trials.

use concerto_ecs::entity::Entity;
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::world::World;
use concerto_foundation::time::Time;
use concerto_foundation::transform::Transform;
use concerto_name::Name;
use concerto_physics::collider::{Collider, register_colliders};
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::ragdoll::{BodyPose, RagdollId};
use glam::Vec3;

use crate::overlap::PartOverlap;
use crate::resolve::ResolvedRagdoll;

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
    ///
    /// # Panics
    ///
    /// Panics when the physics backend has no ragdolls (Rapier) or the world
    /// is out of bodies.
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
    ///
    /// # Panics
    ///
    /// Panics when `ragdoll` has no part on `bone`, or when `placed` is no
    /// longer in the world.
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

/// What to do to a ragdoll standing in its rest pose on a floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trial {
    /// Let it go from `height` metres above the floor.
    Drop { height: f32 },
    /// Hit the part on `bone` with `impulse`, in N·s.
    Poke { bone: Name, impulse: Vec3 },
}

/// What happened to a ragdoll in a [`Trial`].
#[derive(Clone, Debug, PartialEq)]
pub struct TrialReport {
    /// The seconds after which every part stayed slower than 0.2 m/s; `None` when the body was still moving at the end.
    pub rested_after: Option<f32>,
    /// The fastest any part moved, in m/s.
    pub peak_speed: f32,
    /// The widest any joint opened from its anchor, in metres.
    pub peak_joint_gap: f32,
    /// The pairs of parts that overlap in the rest pose, deepest first.
    pub overlaps: Vec<PartOverlap>,
}

const REST_SPEED: f32 = 0.2;

/// Runs `trial` on `ragdoll` for `seconds` in a world of its own and reports what happened.
///
/// Returns `None` when the trial names a bone with no part or the backend
/// has no ragdolls.
pub fn run_trial(ragdoll: &ResolvedRagdoll, trial: Trial, seconds: f32) -> Option<TrialReport> {
    let mut world = RagdollWorld::new();
    let height = match trial {
        Trial::Drop { height } => height.max(0.0),
        Trial::Poke { .. } => 0.02,
    };
    let lowest = ragdoll
        .rest()
        .iter()
        .map(|bone| bone.translation.y)
        .fold(f32::MAX, f32::min)
        .min(0.0);
    let owner = world.world.spawn(Transform::default());
    let pose: Vec<BodyPose> = ragdoll
        .settings()
        .parts
        .iter()
        .map(|part| {
            BodyPose::from_translation(Vec3::new(0.0, height - lowest, 0.0)).then(part.pose)
        })
        .collect();
    let id = world
        .state()
        .create_ragdoll(ragdoll.built(), owner, &pose)?;
    let placed = PlacedRagdoll { id, owner };
    if let Trial::Poke { bone, impulse } = trial {
        let part = ragdoll.part_of(&bone)?;
        let body = world.state().ragdoll_bodies(id)[part];
        world.state().add_impulse(body, impulse);
    }
    let step = Time::fixed_delta_time().as_secs_f32();
    let steps = (seconds / step).round().max(1.0) as usize;
    let mut report = TrialReport {
        rested_after: None,
        peak_speed: 0.0,
        peak_joint_gap: 0.0,
        overlaps: ragdoll.overlapping_parts(),
    };
    for index in 0..steps {
        world.step();
        let fastest = world.speeds(placed).into_iter().fold(0.0, f32::max);
        report.peak_speed = report.peak_speed.max(fastest);
        let pose = world.pose(placed);
        let widest = joint_gaps(ragdoll, &pose).into_iter().fold(0.0, f32::max);
        report.peak_joint_gap = report.peak_joint_gap.max(widest);
        if fastest >= REST_SPEED {
            report.rested_after = None;
        } else if report.rested_after.is_none() {
            report.rested_after = Some((index + 1) as f32 * step);
        }
    }
    Some(report)
}

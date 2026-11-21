use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use glam::{Quat, Vec3};

use concerto_physics::ragdoll::BodyPose;
use concerto_physics::rigid_body::MotionType;
use concerto_ragdoll::testing::{
    reference_humanoid, PlacedRagdoll, RagdollWorld, ReferenceHumanoid,
};

const COUNTS: [usize; 3] = [1, 8, 32];
const SPACING: f32 = 2.5;
const RESET_EVERY: usize = 60;
const POSE_EVERY: usize = 20;

#[derive(Clone, Copy)]
enum Mode {
    Limp,
    Kinematic,
    Powered,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Limp => "limp",
            Mode::Kinematic => "kinematic",
            Mode::Powered => "powered",
        }
    }
}

struct Crowd {
    world: RagdollWorld,
    placed: Vec<PlacedRagdoll>,
    targets: Vec<Vec<BodyPose>>,
    mode: Mode,
    steps: usize,
}

fn root(index: usize) -> BodyPose {
    let row = (index / 8) as f32;
    let column = (index % 8) as f32;
    BodyPose::from_translation(Vec3::new(column * SPACING, 0.5, row * SPACING))
}

fn crowd(humanoid: &ReferenceHumanoid, count: usize, mode: Mode) -> Crowd {
    let mut world = RagdollWorld::new();
    let parts = humanoid.resolved.part_count();
    let mut placed = Vec::new();
    let mut targets = Vec::new();
    for index in 0..count {
        let ragdoll = world.place(&humanoid.resolved, root(index));
        let target: Vec<BodyPose> = humanoid
            .resolved
            .settings()
            .parts
            .iter()
            .map(|part| root(index).then(part.pose))
            .collect();
        match mode {
            Mode::Limp => {}
            Mode::Kinematic => {
                for part in 0..parts {
                    world
                        .state()
                        .set_ragdoll_part_motion(ragdoll.id, part, MotionType::Kinematic);
                }
            }
            Mode::Powered => {
                world
                    .state()
                    .set_ragdoll_part_motion(ragdoll.id, 0, MotionType::Kinematic);
                world.state().drive_ragdoll_motors(ragdoll.id, &target);
            }
        }
        placed.push(ragdoll);
        targets.push(target);
    }
    Crowd {
        world,
        placed,
        targets,
        mode,
        steps: 0,
    }
}

impl Crowd {
    fn step(&mut self) {
        let phase = self.steps as f32 * 0.2;
        for (ragdoll, target) in self.placed.iter().zip(&self.targets) {
            match self.mode {
                Mode::Limp => {
                    if self.steps.is_multiple_of(RESET_EVERY) {
                        self.world.state().set_ragdoll_pose(ragdoll.id, target);
                    }
                }
                Mode::Kinematic => {
                    let sway = Vec3::new(0.3 * phase.sin(), 0.0, 0.0);
                    let moved: Vec<BodyPose> = target
                        .iter()
                        .map(|pose| BodyPose::new(pose.translation + sway, pose.rotation))
                        .collect();
                    self.world
                        .state()
                        .drive_ragdoll_kinematic(ragdoll.id, &moved);
                }
                Mode::Powered => {
                    if self.steps.is_multiple_of(POSE_EVERY) {
                        let bend = if (self.steps / POSE_EVERY).is_multiple_of(2) {
                            0.4
                        } else {
                            0.0
                        };
                        let bent: Vec<BodyPose> = target
                            .iter()
                            .enumerate()
                            .map(|(part, pose)| {
                                let turn = if part == 0 { 0.0 } else { bend };
                                BodyPose::new(
                                    pose.translation,
                                    pose.rotation * Quat::from_rotation_x(turn),
                                )
                            })
                            .collect();
                        self.world.state().drive_ragdoll_motors(ragdoll.id, &bent);
                    }
                }
            }
        }
        self.steps += 1;
        self.world.step();
    }
}

fn fixed_step(c: &mut Criterion) {
    let humanoid = reference_humanoid();
    let mut group = c.benchmark_group("ragdoll_fixed_step");
    for mode in [Mode::Limp, Mode::Kinematic, Mode::Powered] {
        for count in COUNTS {
            let mut crowd = crowd(&humanoid, count, mode);
            group.bench_function(BenchmarkId::new(mode.name(), count), |b| {
                b.iter(|| crowd.step());
            });
        }
    }
    group.finish();
}

criterion_group!(benches, fixed_step);
criterion_main!(benches);

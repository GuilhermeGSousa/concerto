use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::world::World;
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_physics::collider::{Collider, register_colliders};
use concerto_physics::physics_state::PhysicsState;
use concerto_ragdoll::{Ragdoll, RagdollJoint, RagdollPart, RagdollShape};
use glam::{Mat4, Quat, Vec3};

pub const BONE: f32 = 0.5;

pub struct Bone {
    pub name: &'static str,
    pub parent: Option<u16>,
    pub translation: Vec3,
}

pub fn skeleton(bones: &[Bone]) -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; bones.len()],
        bones.iter().map(|bone| bone.parent).collect(),
        bones.iter().map(|bone| bone.name.into()).collect(),
        bones
            .iter()
            .map(|bone| JointPose {
                translation: bone.translation,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            })
            .collect(),
        bones.iter().map(|_| uuid::Uuid::new_v4()).collect(),
    )
    .unwrap()
}

pub fn spine_skeleton() -> Skeleton {
    let up = Vec3::new(0.0, BONE, 0.0);
    skeleton(&[
        Bone {
            name: "Root",
            parent: None,
            translation: Vec3::ZERO,
        },
        Bone {
            name: "Hips",
            parent: Some(0),
            translation: Vec3::new(0.0, 1.0, 0.0),
        },
        Bone {
            name: "Spine",
            parent: Some(1),
            translation: up,
        },
        Bone {
            name: "Tail",
            parent: Some(1),
            translation: Vec3::new(0.0, 0.0, -0.2),
        },
        Bone {
            name: "Head",
            parent: Some(2),
            translation: up,
        },
        Bone {
            name: "Jaw",
            parent: Some(4),
            translation: Vec3::new(0.0, 0.1, 0.1),
        },
    ])
}

pub fn spine_part(bone: &str) -> RagdollPart {
    RagdollPart::new(
        bone,
        RagdollShape::Capsule {
            half_height: 0.15,
            radius: 0.08,
        },
    )
    .with_offset(Vec3::new(0.0, BONE / 2.0, 0.0))
    .with_mass(2.0)
    .with_joint(
        RagdollJoint::new(Vec3::Y, Vec3::X)
            .with_swing(0.6, 0.6)
            .with_twist(-0.3, 0.3),
    )
}

pub fn spine_ragdoll() -> Ragdoll {
    Ragdoll::new(AssetHandle::weak(AssetId::from_path(
        "content/hero/scene.gasset#skeleton/0",
    )))
    .with_part(spine_part("Head"))
    .with_part(spine_part("Hips"))
    .with_part(spine_part("Spine"))
}

pub fn world_with_floor() -> World {
    let mut world = World::new();
    world.register_component::<Collider>();
    world.register_component::<Transform>();
    world.insert_resource(PhysicsState::new());
    world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let mut schedule = Schedule::new();
    schedule.add_system(register_colliders);
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);
    world
}

use concerto_foundation::assets::{
    Asset, CreatableAsset, LoadableAsset, asset_server::AssetLoadContext, handle::AssetHandle,
};
use concerto_mesh::{JointName, ReflectSkeletonSource, Skeleton, SkeletonSource};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// The physical body of one skeleton: which bones have a part, and each part's shape, mass and joint.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(SkeletonSource, Default)]
pub struct Ragdoll {
    pub skeleton: AssetHandle<Skeleton>,
    pub parts: Vec<RagdollPart>,
}

impl Ragdoll {
    /// A ragdoll for `skeleton` with no parts.
    pub fn new(skeleton: AssetHandle<Skeleton>) -> Self {
        Self {
            skeleton,
            parts: Vec::new(),
        }
    }

    pub fn with_part(mut self, part: RagdollPart) -> Self {
        self.parts.push(part);
        self
    }

    /// The part on `bone`, if it has one.
    pub fn part(&self, bone: &str) -> Option<&RagdollPart> {
        self.parts.iter().find(|part| &*part.bone == bone)
    }

    /// The part on `bone`, if it has one, for editing.
    pub fn part_mut(&mut self, bone: &str) -> Option<&mut RagdollPart> {
        self.parts.iter_mut().find(|part| &*part.bone == bone)
    }
}

impl Asset for Ragdoll {
    fn name() -> &'static str {
        "Ragdoll"
    }
}

impl LoadableAsset for Ragdoll {
    fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
        if !self.skeleton.id().is_nil() {
            self.skeleton = context.asset_server().load(self.skeleton.id());
        }
        Ok(())
    }
}

impl CreatableAsset for Ragdoll {
    fn new_asset() -> Self {
        Self::default()
    }
}

impl SkeletonSource for Ragdoll {
    fn skeleton(&self) -> Option<&AssetHandle<Skeleton>> {
        Some(&self.skeleton)
    }
}

/// The collision shape of a ragdoll part.
#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub enum RagdollShape {
    /// A capsule along the shape's local Y axis: `half_height` is half the length of the cylinder.
    Capsule {
        half_height: f32,
        radius: f32,
    },
    Box {
        half_extents: Vec3,
    },
    Sphere {
        radius: f32,
    },
}

impl Default for RagdollShape {
    fn default() -> Self {
        Self::Capsule {
            half_height: 0.1,
            radius: 0.05,
        }
    }
}

/// The spring a joint's motor pulls with when it is driven.
#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RagdollMotor {
    /// Spring frequency in Hz.
    pub frequency: f32,
    /// Damping ratio; 1 is critically damped.
    pub damping: f32,
    /// The most torque the motor applies, in N·m.
    pub torque_limit: f32,
}

impl Default for RagdollMotor {
    fn default() -> Self {
        Self {
            frequency: 2.0,
            damping: 1.0,
            torque_limit: 500.0,
        }
    }
}

/// The joint between a part and the part of its nearest ancestor bone that has one; axes are in the part's bone space, angles in radians.
#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RagdollJoint {
    /// The axis the part twists around.
    pub twist_axis: Vec3,
    /// An axis perpendicular to the twist axis; swing around it is limited by `swing_plane`.
    pub plane_axis: Vec3,
    /// Half the swing range around the axis perpendicular to the twist and plane axes.
    pub swing_normal: f32,
    /// Half the swing range around the plane axis.
    pub swing_plane: f32,
    /// Lowest twist angle, from -π to 0.
    pub twist_min: f32,
    /// Highest twist angle, from 0 to π.
    pub twist_max: f32,
    /// Friction torque while the joint is not driven, in N·m.
    pub friction: f32,
    pub motor: RagdollMotor,
}

impl Default for RagdollJoint {
    fn default() -> Self {
        Self {
            twist_axis: Vec3::Y,
            plane_axis: Vec3::X,
            swing_normal: 0.0,
            swing_plane: 0.0,
            twist_min: 0.0,
            twist_max: 0.0,
            friction: 0.0,
            motor: RagdollMotor::default(),
        }
    }
}

impl RagdollJoint {
    /// A joint twisting around `twist_axis`, rigid until it is given limits.
    pub fn new(twist_axis: Vec3, plane_axis: Vec3) -> Self {
        Self {
            twist_axis,
            plane_axis,
            ..Self::default()
        }
    }

    /// A hinge around `axis` that turns from `min` to `max` and swings no other way, as a knee or an elbow does.
    pub fn hinge(axis: Vec3, min: f32, max: f32) -> Self {
        Self::new(axis, axis.any_orthonormal_vector()).with_twist(min, max)
    }

    /// Sets both swing half angles.
    pub fn with_swing(mut self, swing_normal: f32, swing_plane: f32) -> Self {
        self.swing_normal = swing_normal;
        self.swing_plane = swing_plane;
        self
    }

    pub fn with_twist(mut self, twist_min: f32, twist_max: f32) -> Self {
        self.twist_min = twist_min;
        self.twist_max = twist_max;
        self
    }

    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = friction;
        self
    }

    pub fn with_motor(mut self, motor: RagdollMotor) -> Self {
        self.motor = motor;
        self
    }
}

/// One body of a ragdoll, following one bone.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RagdollPart {
    pub bone: JointName,
    pub shape: RagdollShape,
    /// Where the shape's centre is, in the bone's space.
    pub offset: Vec3,
    /// How the shape is turned, in the bone's space.
    pub rotation: Quat,
    /// Mass in kg.
    pub mass: f32,
    /// The joint to the parent part; unused on the root part.
    pub joint: RagdollJoint,
}

impl Default for RagdollPart {
    fn default() -> Self {
        Self {
            bone: JointName::default(),
            shape: RagdollShape::default(),
            offset: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            mass: 1.0,
            joint: RagdollJoint::default(),
        }
    }
}

impl RagdollPart {
    /// A part of 1 kg on `bone` with its shape centred on the bone.
    pub fn new(bone: impl Into<JointName>, shape: RagdollShape) -> Self {
        Self {
            bone: bone.into(),
            shape,
            ..Self::default()
        }
    }

    pub fn with_offset(mut self, offset: Vec3) -> Self {
        self.offset = offset;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_mass(mut self, mass: f32) -> Self {
        self.mass = mass;
        self
    }

    pub fn with_joint(mut self, joint: RagdollJoint) -> Self {
        self.joint = joint;
        self
    }
}

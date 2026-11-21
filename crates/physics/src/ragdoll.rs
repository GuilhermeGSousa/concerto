use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use concerto_ecs::Component;
use concerto_foundation::transform::Transform;
use glam::{Quat, Vec3};

use crate::backend::PhysicsBackend;
use crate::ActiveBackend;

/// A rigid pose: where a body or a ragdoll part is, without scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyPose {
    pub translation: Vec3,
    pub rotation: Quat,
}

impl BodyPose {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };

    pub fn new(translation: Vec3, rotation: Quat) -> Self {
        Self {
            translation,
            rotation,
        }
    }

    pub fn from_translation(translation: Vec3) -> Self {
        Self::new(translation, Quat::IDENTITY)
    }

    /// The pose of `transform`, dropping its scale.
    pub fn from_transform(transform: &Transform) -> Self {
        Self::new(transform.translation, transform.rotation)
    }

    /// This pose as a `Transform` of unit scale.
    pub fn to_transform(&self) -> Transform {
        Transform::from_translation_rotation(self.translation, self.rotation)
    }

    /// `point`, given in this pose's space, in the space this pose is in.
    pub fn transform_point(&self, point: Vec3) -> Vec3 {
        self.translation + self.rotation * point
    }

    /// `local`, given in this pose's space, in the space this pose is in.
    pub fn then(&self, local: BodyPose) -> BodyPose {
        BodyPose::new(
            self.transform_point(local.translation),
            (self.rotation * local.rotation).normalize(),
        )
    }

    /// The pose that undoes this one.
    pub fn inverse(&self) -> BodyPose {
        let rotation = self.rotation.inverse();
        BodyPose::new(rotation * -self.translation, rotation)
    }
}

impl Default for BodyPose {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The collision shape of one ragdoll part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartShape {
    Sphere {
        radius: f32,
    },
    /// A capsule along the part's local Y axis: `half_height` is half the length of the cylinder.
    Capsule {
        half_height: f32,
        radius: f32,
    },
    Box {
        half_extents: Vec3,
    },
}

/// The spring a joint's motor drives to its target with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotorSettings {
    /// Spring frequency in Hz.
    pub frequency: f32,
    /// Damping ratio; 1 is critically damped.
    pub damping: f32,
    /// The most torque the motor applies, in N·m.
    pub torque_limit: f32,
}

impl Default for MotorSettings {
    fn default() -> Self {
        Self {
            frequency: 2.0,
            damping: 1.0,
            torque_limit: f32::MAX,
        }
    }
}

/// Where a joint's motor aims and how hard it pulls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointTarget {
    /// The rotation the part should have relative to its parent part.
    pub rotation: Quat,
    pub motor: MotorSettings,
}

/// What the motor of the joint between a part and its parent does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointDrive {
    /// The joint swings freely within its limits, slowed by its friction.
    Off,
    /// The motor pulls the part towards a rotation.
    Target(JointTarget),
}

/// The swing-twist joint between a ragdoll part and its parent, placed at the part's origin, with axes in the space of the reference pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointSettings {
    /// The axis the part twists around, usually along the bone.
    pub twist_axis: Vec3,
    /// An axis perpendicular to the twist axis; swing around it is limited by `plane_half_cone`.
    pub plane_axis: Vec3,
    /// Half the swing range around the axis perpendicular to the twist and plane axes, in radians.
    pub normal_half_cone: f32,
    /// Half the swing range around the plane axis, in radians.
    pub plane_half_cone: f32,
    /// Lowest twist angle in radians, in `[-π, 0]`.
    pub twist_min: f32,
    /// Highest twist angle in radians, in `[0, π]`.
    pub twist_max: f32,
    /// Friction torque applied while the joint's motor is off, in N·m.
    pub max_friction_torque: f32,
    pub motor: MotorSettings,
}

impl JointSettings {
    /// A joint twisting around `twist_axis` with every limit at zero: it is rigid until limits are given.
    pub fn new(twist_axis: Vec3, plane_axis: Vec3) -> Self {
        Self {
            twist_axis,
            plane_axis,
            normal_half_cone: 0.0,
            plane_half_cone: 0.0,
            twist_min: 0.0,
            twist_max: 0.0,
            max_friction_torque: 0.0,
            motor: MotorSettings::default(),
        }
    }

    /// Sets both swing half angles.
    pub fn with_swing(mut self, normal_half_cone: f32, plane_half_cone: f32) -> Self {
        self.normal_half_cone = normal_half_cone;
        self.plane_half_cone = plane_half_cone;
        self
    }

    pub fn with_twist(mut self, twist_min: f32, twist_max: f32) -> Self {
        self.twist_min = twist_min;
        self.twist_max = twist_max;
        self
    }

    pub fn with_max_friction_torque(mut self, max_friction_torque: f32) -> Self {
        self.max_friction_torque = max_friction_torque;
        self
    }

    pub fn with_motor(mut self, motor: MotorSettings) -> Self {
        self.motor = motor;
        self
    }
}

/// One body of a ragdoll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartSettings {
    /// Index of the parent part, which comes earlier in the list; `None` for the root, the first part.
    pub parent: Option<usize>,
    /// The part's pose in the reference pose, the one every part of the ragdoll is described in.
    pub pose: BodyPose,
    pub shape: PartShape,
    /// The shape's pose relative to the part.
    pub shape_offset: BodyPose,
    /// Mass in kg.
    pub mass: f32,
    /// The joint to the parent; unused on the root.
    pub joint: JointSettings,
}

impl PartSettings {
    /// A part of 1 kg at `pose` with its shape centred on it.
    pub fn new(pose: BodyPose, shape: PartShape) -> Self {
        Self {
            parent: None,
            pose,
            shape,
            shape_offset: BodyPose::IDENTITY,
            mass: 1.0,
            joint: JointSettings::new(Vec3::Y, Vec3::X),
        }
    }

    /// Joins the part to `parent` with `joint`.
    pub fn with_parent(mut self, parent: usize, joint: JointSettings) -> Self {
        self.parent = Some(parent);
        self.joint = joint;
        self
    }

    pub fn with_shape_offset(mut self, shape_offset: BodyPose) -> Self {
        self.shape_offset = shape_offset;
        self
    }

    pub fn with_mass(mut self, mass: f32) -> Self {
        self.mass = mass;
        self
    }
}

/// The parts of one ragdoll as plain data, built into a [`BuiltRagdoll`] before use.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollSettings {
    pub parts: Vec<PartSettings>,
    /// The fastest any part may move, in m/s; a part hit harder is slowed to it.
    pub max_speed: f32,
}

impl Default for RagdollSettings {
    fn default() -> Self {
        Self {
            parts: Vec::new(),
            max_speed: 50.0,
        }
    }
}

impl RagdollSettings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_speed(mut self, max_speed: f32) -> Self {
        self.max_speed = max_speed;
        self
    }

    pub fn with_part(mut self, part: PartSettings) -> Self {
        self.parts.push(part);
        self
    }

    /// Builds the backend's ragdoll recipe: masses stabilized, and each part's collisions with its parent disabled.
    pub fn build(&self) -> Result<BuiltRagdoll, RagdollBuildError> {
        if self.parts.is_empty() {
            return Err(RagdollBuildError::NoParts);
        }
        for (index, part) in self.parts.iter().enumerate() {
            let rooted = part.parent.is_none();
            if rooted != (index == 0) || part.parent.is_some_and(|parent| parent >= index) {
                return Err(RagdollBuildError::BadParent { part: index });
            }
            if part.mass.is_nan() || part.mass <= 0.0 {
                return Err(RagdollBuildError::BadMass { part: index });
            }
        }
        Ok(BuiltRagdoll {
            handle: Arc::new(ActiveBackend::build_ragdoll(self)?),
            settings: Arc::new(self.clone()),
        })
    }
}

/// Why [`RagdollSettings::build`] refused its settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RagdollBuildError {
    NoParts,
    /// The first part must have no parent and every other part a parent earlier in the list.
    BadParent {
        part: usize,
    },
    BadMass {
        part: usize,
    },
    /// The backend refused the parts: a degenerate shape or joint axis.
    Rejected,
}

impl Display for RagdollBuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoParts => f.write_str("a ragdoll needs at least one part"),
            Self::BadParent { part } => write!(
                f,
                "part {part}: the first part must have no parent and every other part an earlier one"
            ),
            Self::BadMass { part } => write!(f, "part {part}: the mass must be above zero"),
            Self::Rejected => f.write_str(
                "the physics backend refused the parts: check for a zero-sized shape or a zero joint axis",
            ),
        }
    }
}

impl Error for RagdollBuildError {}

/// A ragdoll recipe built for the active backend, shared by every ragdoll created from it.
#[derive(Clone)]
pub struct BuiltRagdoll {
    pub(crate) handle: Arc<<ActiveBackend as PhysicsBackend>::RagdollSettingsHandle>,
    settings: Arc<RagdollSettings>,
}

impl BuiltRagdoll {
    pub fn part_count(&self) -> usize {
        self.settings.parts.len()
    }

    /// The settings this was built from.
    pub fn settings(&self) -> &RagdollSettings {
        &self.settings
    }
}

/// A ragdoll living inside the physics simulation, owned by [`PhysicsState`](crate::physics_state::PhysicsState) until it is destroyed. As a component it marks the entity that owns the ragdoll.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Component)]
pub struct RagdollId(pub(crate) <ActiveBackend as PhysicsBackend>::RagdollHandle);

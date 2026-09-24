use std::ops::BitOr;

use concerto_ecs::component::Component;

/// Bitmask of the degrees of freedom a dynamic body may use (combine with
/// `|`). Restricting a body to [`AllowedDofs::TRANSLATION`] keeps it from
/// ever rotating — e.g. a player capsule that must stay upright.
///
/// Backend-neutral: each backend translates this into its own axis-locking
/// representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllowedDofs(u32);

impl AllowedDofs {
    pub const TRANSLATION_X: Self = Self(1 << 0);
    pub const TRANSLATION_Y: Self = Self(1 << 1);
    pub const TRANSLATION_Z: Self = Self(1 << 2);
    pub const ROTATION_X: Self = Self(1 << 3);
    pub const ROTATION_Y: Self = Self(1 << 4);
    pub const ROTATION_Z: Self = Self(1 << 5);
    /// All translation axes, no rotation.
    pub const TRANSLATION: Self =
        Self(Self::TRANSLATION_X.0 | Self::TRANSLATION_Y.0 | Self::TRANSLATION_Z.0);
    /// All rotation axes, no translation.
    pub const ROTATION: Self = Self(Self::ROTATION_X.0 | Self::ROTATION_Y.0 | Self::ROTATION_Z.0);
    pub const ALL: Self = Self(0x3f);

    /// Whether every degree of freedom in `other` is allowed.
    pub fn contains(&self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for AllowedDofs {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl Default for AllowedDofs {
    fn default() -> Self {
        Self::ALL
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub enum MotionType {
    #[default]
    Dynamic,
    /// Unaffected by forces and collisions; moved only by
    /// [`set_linear_velocity`](crate::physics_state::PhysicsState::set_linear_velocity).
    Kinematic,
}

/// Marks an entity's [`Collider`](crate::collider::Collider) as a dynamic
/// body and describes its dynamics.
///
/// The body itself is created by `Collider`'s lifecycle: spawn `RigidBody`
/// in the same bundle as (or before) the `Collider` so it is visible when
/// the body is created. Fields are read at that moment only — changing them
/// afterwards does not affect the live body.
#[derive(Component, Clone, Debug)]
pub struct RigidBody {
    /// Density in kg/m³; the body's mass is the collider shape's volume times
    /// this. Defaults to 1000 (water).
    pub density: f32,
    pub allowed_dofs: AllowedDofs,
    pub motion_type: MotionType,
}

impl RigidBody {
    pub fn with_density(density: f32) -> Self {
        Self {
            density,
            ..Self::default()
        }
    }
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            density: 1000.0,
            allowed_dofs: AllowedDofs::ALL,
            motion_type: MotionType::Dynamic,
        }
    }
}

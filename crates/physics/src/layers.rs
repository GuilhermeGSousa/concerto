use std::ops::{BitAnd, BitOr};

use concerto_ecs::{
    reflect::{ReflectComponent, ReflectSceneComponent},
    Changed, Component, Query, ResMut,
};
use concerto_reflect::{Reflect, ReflectDefault};

use crate::{body::BodyId, physics_state::PhysicsState};

/// A set of collision layers, as a mask of up to fourteen layers.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default)]
pub struct Layers {
    /// One bit per layer; only bits 0 to 13 are used.
    pub bits: u16,
}

impl Layers {
    /// How many layers there are.
    pub const COUNT: u8 = 14;
    /// No layer.
    pub const NONE: Self = Self { bits: 0 };
    /// Every one of the fourteen layers.
    pub const ALL: Self = Self { bits: 0x3fff };
    /// The layer of everything that names no other: level geometry and props.
    pub const DEFAULT: Self = Self::bit(0);
    /// The layer of a character's capsule.
    pub const CHARACTER: Self = Self::bit(1);
    /// The layer of ragdoll parts.
    pub const RAGDOLL: Self = Self::bit(2);
    /// The first layer a game is free to give a meaning; so are the ones after it, up to `COUNT`.
    pub const FIRST_FREE: u8 = 3;

    /// The single layer `index`, from 0 to 13; a higher index gives no layer.
    pub const fn bit(index: u8) -> Self {
        if index < Self::COUNT {
            Self { bits: 1 << index }
        } else {
            Self::NONE
        }
    }

    /// These layers and `other`'s.
    pub const fn with(self, other: Self) -> Self {
        Self {
            bits: (self.bits | other.bits) & Self::ALL.bits,
        }
    }

    /// These layers but `other`'s.
    pub const fn without(self, other: Self) -> Self {
        Self {
            bits: self.bits & !other.bits & Self::ALL.bits,
        }
    }

    /// Whether any layer is in both.
    pub const fn intersects(self, other: Self) -> bool {
        self.bits & other.bits & Self::ALL.bits != 0
    }

    /// Whether no layer is set.
    pub const fn is_empty(self) -> bool {
        self.bits & Self::ALL.bits == 0
    }

    pub(crate) const fn mask(self) -> u32 {
        (self.bits & Self::ALL.bits) as u32
    }
}

impl BitOr for Layers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.with(rhs)
    }
}

impl BitAnd for Layers {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self {
            bits: self.bits & rhs.bits & Self::ALL.bits,
        }
    }
}

/// Which collision layers a body is in and which it collides with.
///
/// Two bodies collide when each is in a layer the other collides with. It
/// goes beside a `Collider`; without it a body is in [`Layers::DEFAULT`] and
/// collides with every layer.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq)]
#[reflect(Component, Default, SceneComponent)]
pub struct CollisionLayers {
    /// The layers the body is in.
    pub memberships: Layers,
    /// The layers the body collides with.
    pub filter: Layers,
}

impl CollisionLayers {
    /// In no layer: the body collides with nothing and no query sees it.
    pub const NONE: Self = Self::new(Layers::NONE, Layers::NONE);

    /// Creates layers from the ones the body is in and the ones it collides with.
    pub const fn new(memberships: Layers, filter: Layers) -> Self {
        Self {
            memberships,
            filter,
        }
    }

    /// A character's capsule: in `Layers::CHARACTER`, colliding with everything but ragdoll parts.
    pub const fn character() -> Self {
        Self::new(Layers::CHARACTER, Layers::ALL.without(Layers::RAGDOLL))
    }

    /// A ragdoll part: in `Layers::RAGDOLL`, colliding with everything but character capsules.
    pub const fn ragdoll() -> Self {
        Self::new(Layers::RAGDOLL, Layers::ALL.without(Layers::CHARACTER))
    }

    /// A trigger volume's default: in `Layers::DEFAULT`, detecting everything but ragdoll parts, so a character entering it is reported once.
    pub const fn sensor() -> Self {
        Self::new(Layers::DEFAULT, Layers::ALL.without(Layers::RAGDOLL))
    }

    /// Whether a body with these layers and one with `other`'s collide.
    pub const fn collides_with(&self, other: &Self) -> bool {
        self.memberships.intersects(other.filter) && other.memberships.intersects(self.filter)
    }
}

impl Default for CollisionLayers {
    fn default() -> Self {
        Self::new(Layers::DEFAULT, Layers::ALL)
    }
}

/// Gives a body its entity's `CollisionLayers` whenever they change.
pub fn apply_collision_layers(
    changed: Query<(&BodyId, &CollisionLayers), Changed<CollisionLayers>>,
    mut state: ResMut<PhysicsState>,
) {
    for (body, layers) in changed.iter() {
        state.set_body_layers(*body, *layers);
    }
}

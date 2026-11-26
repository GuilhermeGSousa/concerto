use concerto_ecs::{Entity, signal::Signal};
use glam::Vec3;

/// A character's body was hit: on this part of its ragdoll, at this world point, with this impulse in N·s. It says nothing about damage or death; the game decides those and reacts to the same signal. It is triggered for every listener, with the character it is about, so the engine's hit routing and a game's own listeners all hear it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyHit {
    pub character: Entity,
    pub part: usize,
    pub point: Vec3,
    pub impulse: Vec3,
    /// What struck it, which chooses the reaction.
    pub kind: HitKind,
}

impl BodyHit {
    /// A blow on `part` of `character` at the world point `point`, with `impulse` in N·s.
    pub fn new(character: Entity, part: usize, point: Vec3, impulse: Vec3) -> Self {
        Self {
            character,
            part,
            point,
            impulse,
            kind: HitKind::Blow,
        }
    }

    /// Marks the hit as a gunshot.
    pub fn shot(mut self) -> Self {
        self.kind = HitKind::Shot;
        self
    }
}

impl Signal for BodyHit {}

/// What struck a body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HitKind {
    /// A punch, a shove, a thrown thing: the upper body flinches over legs that follow the animation.
    #[default]
    Blow,
    /// A bullet: its graph receives a `Shot` message, where a blow sends a `Hit`.
    Shot,
}

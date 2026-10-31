use concerto_ecs::{Component, Entity};

/// Names the capsule whose `CharacterSimulation` a visible character follows, inserted by the game at spawn.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct DisplacementSimulation(pub Entity);

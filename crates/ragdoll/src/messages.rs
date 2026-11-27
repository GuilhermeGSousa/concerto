use concerto_animation::message::AnimationMessage;
use concerto_ecs::signal::{EntitySignal, Signal};
use concerto_reflect::{Reflect, ReflectDefault};

/// Tells a graph its character is dying, its strength draining over `seconds`.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Death {
    /// The seconds over which the strength drains; zero drains it at once.
    pub seconds: f32,
}

impl AnimationMessage for Death {}

/// Raised by a [`DyingNode`](crate::node::DyingNode) once it has drained the body's last strength, and triggered on the character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
pub struct BodyDead;

impl Signal for BodyDead {}

impl EntitySignal for BodyDead {}

impl AnimationMessage for BodyDead {}

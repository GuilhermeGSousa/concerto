use concerto_animation::message::AnimationMessage;
use concerto_ecs::signal::{EntitySignal, Signal};
use concerto_reflect::{Reflect, ReflectDefault};

/// Raised by a [`BalanceNode`](crate::node::BalanceNode) once the body has stood still on its feet, and triggered on the character.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Steadied;

impl Signal for Steadied {}

impl EntitySignal for Steadied {}

impl AnimationMessage for Steadied {}

/// Raised by a [`BalanceNode`](crate::node::BalanceNode) once it has given up keeping the body on its feet, and triggered on the character.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Toppled;

impl Signal for Toppled {}

impl EntitySignal for Toppled {}

impl AnimationMessage for Toppled {}

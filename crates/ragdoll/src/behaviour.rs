use std::fmt::Write;

use concerto_ecs::{
    CommandQueue, Component, Entity, Query, SystemSet,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::Signal,
};
use concerto_name::Name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};

use crate::{
    asset::PartStrength,
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    sensing::BodySense,
};

/// The set, in `FixedUpdate`, that behaviours' systems run in.
///
/// After it the claims of the step are complete, and the ragdoll is driven
/// from them later in the same step.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct BehaviourSet;

/// What a behaviour asks of one part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartCommand {
    /// The rotation the part should have relative to its parent part; `None` keeps the animated one.
    pub target: Option<Quat>,
    /// How the part is held to the target.
    pub strength: PartStrength,
    /// A world-space force on the part at its centre of mass, in N.
    pub force: Vec3,
    /// A world-space torque on the part, in N·m.
    pub torque: Vec3,
}

impl PartCommand {
    /// Holds the part to the animated rotation with `strength`.
    pub fn held(strength: PartStrength) -> Self {
        Self {
            target: None,
            strength,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
        }
    }

    /// Aims the part at `target`, its rotation relative to its parent part.
    pub fn with_target(mut self, target: Quat) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_force(mut self, force: Vec3) -> Self {
        self.force = force;
        self
    }

    pub fn with_torque(mut self, torque: Vec3) -> Self {
        self.torque = torque;
        self
    }
}

/// One behaviour's claim on one part for this fixed step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Claim {
    pub behaviour: Name,
    pub part: usize,
    /// The higher priority wins the part; the animated pose is below every claim.
    pub priority: i32,
    /// How much of the command takes hold over what lower priorities ask, from 0 to 1.
    ///
    /// A behaviour fading in or out blends by it.
    pub weight: f32,
    pub command: PartCommand,
}

/// A behaviour that claimed parts in the last fixed step, for reports.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningBehaviour {
    pub name: Name,
    pub priority: i32,
    pub parts: Vec<usize>,
}

/// Where behaviours write what they want of a character's body.
///
/// Each fixed step every running behaviour claims the parts it drives. For
/// each part the highest priority wins, blended by its weight over what is
/// below it, with the animated pose and the character's strength profile at
/// the bottom. The claims are taken when the ragdoll is driven, so a
/// behaviour claims again every step it runs.
#[derive(Component, Clone, Debug, Default)]
pub struct BodyDrive {
    claims: Vec<Claim>,
    running: Vec<RunningBehaviour>,
}

impl BodyDrive {
    /// Claims `part` for `behaviour` this step.
    pub fn claim(
        &mut self,
        behaviour: impl Into<Name>,
        part: usize,
        priority: i32,
        weight: f32,
        command: PartCommand,
    ) {
        self.claims.push(Claim {
            behaviour: behaviour.into(),
            part,
            priority,
            weight: weight.clamp(0.0, 1.0),
            command,
        });
    }

    /// The claims made since the ragdoll was last driven.
    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    /// The behaviours that claimed parts in the last fixed step, highest priority first.
    pub fn running(&self) -> &[RunningBehaviour] {
        &self.running
    }

    pub(crate) fn take(&mut self) -> Vec<Claim> {
        let mut claims = std::mem::take(&mut self.claims);
        claims.sort_by_key(|claim| claim.priority);
        self.running.clear();
        for claim in claims.iter().rev() {
            match self
                .running
                .iter_mut()
                .find(|running| running.name == claim.behaviour)
            {
                Some(running) => running.parts.push(claim.part),
                None => self.running.push(RunningBehaviour {
                    name: claim.behaviour,
                    priority: claim.priority,
                    parts: vec![claim.part],
                }),
            }
        }
        for running in &mut self.running {
            running.parts.sort_unstable();
            running.parts.dedup();
        }
        claims
    }
}

/// What a physical character is doing.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Default)]
pub enum BodyState {
    /// The parts follow the animation.
    #[default]
    Animated,
    /// On its feet and powered: flinching, or behaviours are running.
    Reacting,
    /// No longer supported.
    Falling,
    /// On the ground.
    Down,
    /// Limp for good; nothing the engine does moves a character out of it.
    Dead,
}

/// The state a physical character is in.
///
/// The engine moves it between `Animated` and `Reacting` and to `Down`. A
/// behaviour that gives up its balance sets `Falling`, and a game or the
/// dying behaviour sets `Dead`. Every change triggers [`BodyStateChanged`].
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct PhysicalState {
    pub state: BodyState,
    #[reflect(skip)]
    announced: BodyState,
    #[reflect(skip)]
    drew: bool,
}

impl PhysicalState {
    pub fn new(state: BodyState) -> Self {
        Self {
            state,
            announced: state,
            drew: false,
        }
    }
}

/// A character's `PhysicalState` changed; triggered for every listener, with the character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyStateChanged {
    pub character: Entity,
    pub from: BodyState,
    pub to: BodyState,
}

impl Signal for BodyStateChanged {}

/// How a behaviour ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    GaveUp,
}

/// A behaviour on a character ended by itself; triggered for every listener, so the next behaviour can start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BehaviourEnded {
    pub character: Entity,
    pub behaviour: Name,
    pub outcome: Outcome,
}

impl Signal for BehaviourEnded {}

pub(crate) const LYING: f32 = 1.0;

/// Moves every character's [`PhysicalState`] to where its body is, each fixed step after the body is sensed.
///
/// The state is `Animated` while the ragdoll follows its animation and
/// nothing claims a part, `Reacting` otherwise, and `Down` once a falling or
/// limp body lies at rest. Triggers [`BodyStateChanged`] for every change,
/// whoever made it.
pub fn update_physical_states(
    mut characters: Query<(
        Entity,
        &mut CharacterRagdoll,
        &mut PhysicalState,
        Option<&BodyDrive>,
        Option<&BodySense>,
    )>,
    mut cmd: CommandQueue,
) {
    for (entity, mut character, mut physical, drive, sense) in characters.iter_mut() {
        let driven = drive.is_some_and(|drive| !drive.running().is_empty());
        let following = character.mode == RagdollMode::FollowsAnimation && !driven;
        let lying = sense.is_some_and(|sense| sense.at_rest && sense.lean > LYING);
        let state = match physical.state {
            BodyState::Dead => BodyState::Dead,
            BodyState::Falling | BodyState::Down if !following => {
                if lying {
                    BodyState::Down
                } else {
                    physical.state
                }
            }
            _ if following => BodyState::Animated,
            _ if character.mode == RagdollMode::Limp && lying => BodyState::Down,
            _ => BodyState::Reacting,
        };
        if physical.state != state {
            physical.state = state;
        }
        if driven && !physical.drew {
            character.weight = 1.0;
            physical.drew = true;
        }
        if physical.drew && state == BodyState::Animated {
            character.weight = 0.0;
            physical.drew = false;
        }
        if physical.announced != physical.state {
            cmd.trigger(BodyStateChanged {
                character: entity,
                from: physical.announced,
                to: physical.state,
            });
            physical.announced = physical.state;
        }
    }
}

/// A character's physical state, its running behaviours with their priorities and parts, and what its body senses, as text an agent can read.
pub fn describe_body(
    physical: Option<&PhysicalState>,
    drive: Option<&BodyDrive>,
    sense: Option<&BodySense>,
    body: &RagdollBody,
) -> String {
    let mut text = String::new();
    let state = physical.map_or(BodyState::Animated, |physical| physical.state);
    let _ = writeln!(text, "state: {state:?}");
    match drive.map(BodyDrive::running) {
        None => text.push_str("behaviours: the character has no BodyDrive\n"),
        Some([]) => text.push_str("behaviours: none running\n"),
        Some(running) => {
            for behaviour in running {
                let parts: Vec<String> = behaviour
                    .parts
                    .iter()
                    .filter_map(|part| body.bone_name(*part))
                    .map(|bone| bone.to_string())
                    .collect();
                let _ = writeln!(
                    text,
                    "behaviour {} · priority {} · {}",
                    behaviour.name,
                    behaviour.priority,
                    parts.join(", ")
                );
            }
        }
    }
    match sense {
        None => text.push_str("sensing: the character has no BodySense\n"),
        Some(sense) => {
            let centre = sense.centre.position;
            let velocity = sense.centre.velocity;
            let _ = writeln!(
                text,
                "centre of mass: [{:.2}, {:.2}, {:.2}] moving [{:.2}, {:.2}, {:.2}] m/s",
                centre.x, centre.y, centre.z, velocity.x, velocity.y, velocity.z
            );
            let feet: Vec<String> = sense
                .supporting
                .iter()
                .filter_map(|part| body.bone_name(*part))
                .map(|bone| bone.to_string())
                .collect();
            let _ = writeln!(
                text,
                "support: {}",
                if feet.is_empty() {
                    "none".to_string()
                } else {
                    feet.join(", ")
                }
            );
            let _ = writeln!(
                text,
                "balance: {}",
                if sense.balanced() {
                    "capture point inside the support".to_string()
                } else {
                    format!(
                        "capture point {:.2} m outside the support",
                        sense.off_balance.length()
                    )
                }
            );
            let _ = writeln!(
                text,
                "lean {:.2} rad · airborne {:.2} s · {}",
                sense.lean,
                sense.airborne,
                if sense.at_rest { "at rest" } else { "moving" }
            );
        }
    }
    text
}

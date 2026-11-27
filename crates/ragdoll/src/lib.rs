//! A character's physical body: the ragdoll asset and what is built from it.
//!
//! A [`CharacterRagdoll`] follows its animation, is powered towards it, or falls limp, as its
//! [`RagdollMode`] says. In [`RagdollMode::Graph`] the character's animation graph drives the
//! body bone by bone instead: nodes such as [`HoldNode`], [`LimpNode`] and [`FlinchNode`] write a
//! [`PhysicsAmount`] and a [`MotorCommand`] for each bone in the graph's drive pass, and
//! [`drive::drive_from_graph`] applies them each physics step.

/// The ragdoll asset: the parts, shapes, joints and strength profiles of one skeleton's body.
pub mod asset;
/// The pose attributes a graph drives a ragdoll with: how much of each bone physics draws, and what its motor is asked.
pub mod attributes;
/// The balance behaviour, which keeps a body on its feet.
pub mod balance;
/// How behaviours claim a body's parts, and the state a physical character is in.
pub mod behaviour;
/// The component that gives a character a ragdoll, and how the ragdoll follows its animation.
pub mod character;
/// Driving a ragdoll from its animation graph's physics attributes.
pub mod drive;
/// Draining a character's strength until it is dead and its body settles.
pub mod dying;
/// The fall behaviour, which brings a falling body down with its hands out and its head held up.
pub mod fall;
/// Generating a first-draft humanoid ragdoll from a retarget rig.
pub mod generate;
/// Blows on a character's body and the flinch they cause.
pub mod hit;
mod ik;
/// The messages a character's animation graph exchanges about its body, and how hits reach the graph.
pub mod messages;
/// Animation graph nodes that drive a character's ragdoll.
pub mod node;
/// Finding parts whose shapes overlap in the rest pose.
pub mod overlap;
/// The plugin that adds ragdolls to an app.
pub mod plugin;
/// Fitting a ragdoll asset to its skeleton and building it for the physics backend.
pub mod resolve;
pub mod sandbox;
/// What a character's body senses about itself: its support, centre of mass and coming impacts.
pub mod sensing;
/// The shot behaviour, which answers a bullet with the whole body.
pub mod shot;
/// The stagger behaviour, which makes a balancing body stumble.
pub mod stagger;
#[cfg(feature = "test-support")]
pub mod testing;
/// A character's body as the nodes of its animation graph read it.
pub mod view;

pub use asset::{
    BoneStrength, PartStrength, Ragdoll, RagdollJoint, RagdollMotor, RagdollPart, RagdollShape,
    Segment, Side, StrengthProfile,
};
pub use attributes::{MotorCommand, PhysicsAmount};
pub use balance::{BALANCE, BALANCE_PRIORITY, Balance, Hindrance};
pub use behaviour::{
    BehaviourEnded, BehaviourSet, BodyDrive, BodyState, BodyStateChanged, Claim, Outcome,
    PartCommand, PhysicalState, RunningBehaviour, describe_body,
};
pub use character::{
    CharacterRagdoll, PartWeight, PoseLost, RagdollBody, RagdollMode, RagdollRefused,
};
pub use dying::{BodySettled, Die, Dying, Settled};
pub use fall::{FALL, FALL_PRIORITY, Fall, FallReaction};
pub use generate::{
    BRACED, FLINCH, GenerateError, HumanoidChains, HumanoidSettings, LIMP, generate_humanoid,
};
pub use hit::{BodyHit, Flinch, FlinchRecord, HitKind, HitReaction};
pub use messages::{Hit, PendingImpulses};
pub use node::{FlinchNode, FlinchState, HoldNode, HoldState, LimpNode};
pub use overlap::PartOverlap;
pub use plugin::RagdollPlugin;
pub use resolve::{Landmarks, Limbs, RagdollError, ResolvedRagdoll};
pub use sandbox::{Trial, TrialReport, run_trial};
pub use sensing::{BodySense, Impact, MassPoint};
pub use shot::{SHOT, SHOT_PRIORITY, Shot, ShotReaction, Wound, Wounds};
pub use stagger::{STAGGER, STAGGER_PRIORITY, Stagger};
pub use view::{BodyInput, BodyView};

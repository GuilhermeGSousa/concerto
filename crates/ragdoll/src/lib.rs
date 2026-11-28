//! A character's physical body: the ragdoll asset and what is built from it.
//!
//! A [`CharacterRagdoll`] is driven bone by bone by the character's animation graph: nodes such as [`HoldNode`], [`LimpNode`], [`FlinchNode`],
//! [`DyingNode`], [`BalanceNode`], [`StaggerNode`], [`ShotNode`] and [`FallNode`] write a [`PhysicsAmount`] and a [`MotorCommand`] for each
//! bone in the graph's drive pass, and [`drive::drive_from_graph`] applies them each physics step. The graph hears
//! what happens to the body through the [`messages`] the crate registers.

/// The ragdoll asset: the parts, shapes, joints and strength profiles of one skeleton's body.
pub mod asset;
/// The pose attributes a graph drives a ragdoll with: how much of each bone physics draws, and what its motor is asked.
pub mod attributes;
/// The component that gives a character a ragdoll, and how the ragdoll follows its animation.
pub mod character;
/// Driving a ragdoll from its animation graph's physics attributes.
pub mod drive;
/// A character's death, and the settling and waking of its dead body.
pub mod dying;
/// How a fall plays out, for the [`FallNode`].
pub mod fall;
/// Generating a first-draft humanoid ragdoll from a retarget rig.
pub mod generate;
/// Blows and gunshots on a character's body.
pub mod hit;
mod ik;
/// The animation messages that tell a character's graph what happens to its body, and that its nodes raise, and how hits reach the graph.
pub mod messages;
/// Animation graph nodes that drive a character's ragdoll.
pub mod node;
/// Finding parts whose shapes overlap in the rest pose.
pub mod overlap;
/// The plugin that adds ragdolls to an app.
pub mod plugin;
/// What a character's body and its animation graph are doing, as text an agent can read.
pub mod report;
/// Fitting a ragdoll asset to its skeleton and building it for the physics backend.
pub mod resolve;
pub mod sandbox;
/// What a character's body senses about itself: its support, centre of mass and coming impacts.
pub mod sensing;
/// How a gunshot reaction plays out, for the [`ShotNode`], and the wounds it leaves.
pub mod shot;
/// How a stumble plays out, for the [`StaggerNode`].
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
pub use character::{CharacterRagdoll, PoseLost, RagdollBody, RagdollRefused};
pub use dying::{BodySettled, Dead, Die, Settled};
pub use fall::FallSettings;
pub use generate::{
    BRACED, FLINCH, GenerateError, HumanoidChains, HumanoidSettings, LIMP, generate_humanoid,
};
pub use hit::{BodyHit, HitKind};
pub use messages::{BodyDead, BodyDown, Death, Hit, PendingImpulses, Wounded};
pub use node::{
    BalanceNode, BalanceSettings, BalanceState, DyingNode, DyingState, FallNode, FallState,
    FlinchNode, FlinchState, Hindrance, HoldNode, HoldState, LimpNode, ShotNode, ShotState,
    StaggerNode, StaggerState,
};
pub use overlap::PartOverlap;
pub use plugin::RagdollPlugin;
pub use report::active_states;
pub use resolve::{Landmarks, Limbs, RagdollError, ResolvedRagdoll};
pub use sandbox::{Trial, TrialReport, run_trial};
pub use sensing::{BodySense, Impact, MassPoint};
pub use shot::{ShotSettings, Wound, Wounds};
pub use stagger::StaggerSettings;
pub use view::{BodyInput, BodyView};

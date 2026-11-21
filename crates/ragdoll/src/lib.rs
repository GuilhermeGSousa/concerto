//! A character's physical body: the ragdoll asset and what is built from it.

pub mod asset;
pub mod balance;
pub mod behaviour;
pub mod character;
pub mod generate;
pub mod hit;
mod ik;
pub mod overlap;
pub mod plugin;
pub mod resolve;
pub mod sandbox;
pub mod sensing;
pub mod shot;
pub mod stagger;
#[cfg(feature = "test-support")]
pub mod testing;

pub use asset::{
    BoneStrength, PartStrength, Ragdoll, RagdollJoint, RagdollMotor, RagdollPart, RagdollShape,
    Segment, Side, StrengthProfile,
};
pub use balance::{BALANCE, BALANCE_PRIORITY, Balance, Hindrance};
pub use behaviour::{
    BehaviourEnded, BehaviourSet, BodyDrive, BodyState, BodyStateChanged, Claim, Outcome,
    PartCommand, PhysicalState, RunningBehaviour, describe_body,
};
pub use character::{
    CharacterRagdoll, PartWeight, PoseLost, RagdollBody, RagdollMode, RagdollRefused,
};
pub use generate::{
    BRACED, FLINCH, GenerateError, HumanoidChains, HumanoidSettings, LIMP, generate_humanoid,
};
pub use hit::{BodyHit, Flinch, FlinchRecord, HitKind, HitReaction};
pub use overlap::PartOverlap;
pub use plugin::RagdollPlugin;
pub use resolve::{Landmarks, Limbs, RagdollError, ResolvedRagdoll};
pub use sandbox::{Trial, TrialReport, run_trial};
pub use sensing::{BodySense, Impact, MassPoint};
pub use shot::{SHOT, SHOT_PRIORITY, Shot, ShotReaction, Wound, Wounds};
pub use stagger::{STAGGER, STAGGER_PRIORITY, Stagger};

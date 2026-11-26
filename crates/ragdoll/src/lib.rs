//! A character's physical body: the ragdoll asset and what is built from it.

pub mod asset;
pub mod attributes;
pub mod balance;
pub mod behaviour;
pub mod character;
pub mod drive;
pub mod dying;
pub mod fall;
pub mod generate;
pub mod hit;
mod ik;
pub mod messages;
pub mod node;
pub mod overlap;
pub mod plugin;
pub mod resolve;
pub mod sandbox;
pub mod sensing;
pub mod shot;
pub mod stagger;
#[cfg(feature = "test-support")]
pub mod testing;
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
pub use fall::{FALL, FALL_PRIORITY, Fall, FallReaction, FallSettings};
pub use generate::{
    BRACED, FLINCH, GenerateError, HumanoidChains, HumanoidSettings, LIMP, generate_humanoid,
};
pub use hit::{BodyHit, Flinch, FlinchRecord, HitKind, HitReaction};
pub use messages::{BodyDead, BodyDown, Death, Hit, PendingImpulses, Wounded};
pub use node::{
    BalanceNode, BalanceSettings, BalanceState, FallNode, FallState, FlinchNode, HoldNode,
    LimpNode, ShotNode, ShotState, StaggerNode, StaggerState,
};
pub use overlap::PartOverlap;
pub use plugin::RagdollPlugin;
pub use resolve::{Landmarks, Limbs, RagdollError, ResolvedRagdoll};
pub use sandbox::{Trial, TrialReport, run_trial};
pub use sensing::{BodySense, Impact, MassPoint};
pub use shot::{SHOT, SHOT_PRIORITY, Shot, ShotReaction, ShotSettings, Wound, Wounds};
pub use stagger::{STAGGER, STAGGER_PRIORITY, Stagger, StaggerSettings};
pub use view::BodyView;

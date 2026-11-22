//! A character's physical body: the ragdoll asset and what is built from it.

pub mod asset;
pub mod character;
pub mod generate;
pub mod hit;
pub mod overlap;
pub mod plugin;
pub mod resolve;
pub mod sandbox;
#[cfg(feature = "test-support")]
pub mod testing;

pub use asset::{
    BoneStrength, PartStrength, Ragdoll, RagdollJoint, RagdollMotor, RagdollPart, RagdollShape,
    StrengthProfile,
};
pub use character::{
    CharacterRagdoll, PartWeight, PoseLost, RagdollBody, RagdollMode, RagdollRefused,
};
pub use generate::{
    BRACED, FLINCH, GenerateError, HumanoidChains, HumanoidSettings, LIMP, Segment,
    generate_humanoid,
};
pub use hit::{BodyHit, Flinch, FlinchRecord, HitReaction};
pub use overlap::PartOverlap;
pub use plugin::RagdollPlugin;
pub use resolve::{RagdollError, ResolvedRagdoll};
pub use sandbox::{Trial, TrialReport, run_trial};

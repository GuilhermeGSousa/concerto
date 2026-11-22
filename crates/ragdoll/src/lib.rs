//! A character's physical body: the ragdoll asset and what is built from it.

pub mod asset;
pub mod character;
pub mod generate;
pub mod overlap;
pub mod plugin;
pub mod resolve;
pub mod sandbox;
#[cfg(feature = "test-support")]
pub mod testing;

pub use asset::{Ragdoll, RagdollJoint, RagdollMotor, RagdollPart, RagdollShape};
pub use character::{CharacterRagdoll, PartWeight, RagdollBody, RagdollMode, RagdollRefused};
pub use generate::{GenerateError, HumanoidChains, HumanoidSettings, Segment, generate_humanoid};
pub use overlap::PartOverlap;
pub use plugin::RagdollPlugin;
pub use resolve::{RagdollError, ResolvedRagdoll};
pub use sandbox::{Trial, TrialReport, run_trial};

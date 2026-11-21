//! A character's physical body: the ragdoll asset and what is built from it.

pub mod asset;
pub mod plugin;
pub mod resolve;

pub use asset::{Ragdoll, RagdollJoint, RagdollMotor, RagdollPart, RagdollShape};
pub use plugin::RagdollPlugin;
pub use resolve::{RagdollError, ResolvedRagdoll};

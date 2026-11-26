//! A character's physical body: the ragdoll asset and what is built from it.

pub mod asset;
pub mod attributes;
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
pub use character::{CharacterRagdoll, PoseLost, RagdollBody, RagdollRefused};
pub use dying::{BodySettled, Dead, Die, Settled};
pub use fall::FallSettings;
pub use generate::{
    BRACED, FLINCH, GenerateError, HumanoidChains, HumanoidSettings, LIMP, generate_humanoid,
};
pub use hit::{BodyHit, HitKind};
pub use messages::{BodyDead, BodyDown, Death, Hit, PendingImpulses, Wounded};
pub use node::{
    BalanceNode, BalanceSettings, BalanceState, Dying, DyingNode, DyingState, FallNode, FallState,
    FlinchNode, Hindrance, HoldNode, LimpNode, ShotNode, ShotState, StaggerNode, StaggerState,
};
pub use overlap::PartOverlap;
pub use plugin::RagdollPlugin;
pub use resolve::{Landmarks, Limbs, RagdollError, ResolvedRagdoll};
pub use sandbox::{Trial, TrialReport, run_trial};
pub use sensing::{BodySense, Impact, MassPoint, describe_body};
pub use shot::{ShotSettings, Wound, Wounds};
pub use stagger::StaggerSettings;
pub use view::BodyView;

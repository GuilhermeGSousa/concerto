//! Procedural animation graph nodes for Concerto: foot placement, foot locking, look-at, aim and
//! procedural motion.
//!
//! Each node passes its first input through and then adjusts the pose. They live apart from
//! `concerto-animation` so that crate has no physics dependency; add
//! [`ProceduralAnimationPlugin`] to register them.

/// The aim node, which turns a bone chain so a muzzle points at a target.
pub mod aim;
/// The foot lock node, which holds planted feet still while the body moves over them.
pub mod foot_lock;
/// The foot placement node, which puts feet on the ground under them by ray casts.
pub mod foot_placement;
mod leg;
/// The look-at node, which turns a bone chain and then the eyes toward a target.
pub mod look_at;
/// The plugin that registers the procedural animation nodes.
pub mod plugin;
/// The procedural motion node, which adds waving or wandering rotations to masked bones.
pub mod procedural_motion;
#[cfg(test)]
mod testing;

pub use aim::{AimBone, AimNode, AimState, Muzzle};
pub use foot_lock::{FootLockLeg, FootLockNode, FootLockState, LegLock, LockPhase};
pub use foot_placement::{FootPlacementLeg, FootPlacementNode, IgnoredBody};
pub use look_at::{LookAtBone, LookAtEye, LookAtNode, LookAtState};
pub use plugin::ProceduralAnimationPlugin;
pub use procedural_motion::{
    ProceduralMotionChannel, ProceduralMotionNode, ProceduralMotionState, Waveform,
};

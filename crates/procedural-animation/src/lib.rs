pub mod aim;
pub mod foot_lock;
pub mod foot_placement;
mod leg;
pub mod look_at;
pub mod plugin;
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

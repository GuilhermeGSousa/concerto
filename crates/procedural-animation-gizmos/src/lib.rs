//! The debug drawing of procedural animation: `FootLockGizmos` draws each foot lock of a player in the running game.

mod gizmos;

pub use gizmos::{FootLockGizmos, ProceduralAnimationGizmosPlugin, draw_foot_locks};

#[cfg(test)]
mod tests;

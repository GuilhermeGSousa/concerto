//! Physics for the engine: backend-agnostic components, resources, and
//! systems over a compile-time-selected physics engine.
//!
//! The engine-facing API is shared code; everything backend-specific hides
//! behind the [`backend::PhysicsBackend`] trait. Native builds use Jolt
//! Physics (via the in-repo `jolt-ffi` bindings).

pub mod aabb;
pub mod backend;
pub mod body;
pub mod collider;
pub mod ground;
pub mod interpolation;
pub mod movement;
pub mod physics_pipeline;
pub mod physics_state;
pub mod plugin;
pub mod ray;
pub mod rigid_body;
pub mod shape;

mod simulation;

pub use simulation::step_simulation;

/// The physics engine compiled into this build. Exactly one exists per
/// build; this alias is the only place the choice is made: Rapier on wasm
/// (where Jolt's C++ cannot be compiled) or under `force-rapier`, Jolt
/// everywhere else.
#[cfg(any(target_arch = "wasm32", feature = "force-rapier"))]
pub type ActiveBackend = backend::rapier::RapierBackend;
#[cfg(all(
    not(target_arch = "wasm32"),
    not(feature = "force-rapier"),
    feature = "jolt"
))]
pub type ActiveBackend = backend::jolt::JoltBackend;

#[cfg(all(
    not(target_arch = "wasm32"),
    not(feature = "force-rapier"),
    not(feature = "jolt")
))]
compile_error!(
    "no physics backend selected: enable the `jolt` feature (default) or `force-rapier`"
);

//! Physics for the engine: backend-agnostic components, resources, and
//! systems over a compile-time-selected physics engine.
//!
//! The engine-facing API is shared code; everything backend-specific hides
//! behind the [`backend::PhysicsBackend`] trait. Native builds use Jolt
//! Physics (via the in-repo `jolt-ffi` bindings); wasm builds, and builds
//! with the `force-rapier` feature, use Rapier.

/// Axis-aligned bounds of a collision shape.
pub mod aabb;
pub mod backend;
/// The handle of a body in the simulation and how it is moving.
pub mod body;
/// Marking a body that something else moves.
pub mod carried;
/// The components that give an entity a physics body, and the systems that create it.
pub mod collider;
/// Contacts between solid bodies, as a component and as signals.
pub mod contact;
/// Probing what a body stands on each fixed step.
pub mod ground;
/// Smoothing body transforms between fixed steps.
pub mod interpolation;
/// Collision layers, which decide what collides with what and what a query sees.
pub mod layers;
/// The scratch the simulation steps with.
pub mod physics_pipeline;
/// The physics world resource, through which bodies are moved and queried.
pub mod physics_state;
/// The plugin that adds physics to an app.
pub mod plugin;
/// Filters and results of scene queries such as sphere casts.
pub mod query;
/// Ragdoll descriptions, their build errors, and the handle of a live ragdoll.
pub mod ragdoll;
/// The result of a raycast.
pub mod ray;
/// Dynamic and kinematic bodies and the axes they may move along.
pub mod rigid_body;
/// Trigger volumes and the signals they send.
pub mod sensor;
/// How the simulation divides each fixed step.
pub mod settings;
/// Collision shapes and mesh colliders.
pub mod shape;

mod simulation;

pub use simulation::step_simulation;

/// The physics engine compiled into this build.
///
/// Exactly one exists per build, and this alias is the only place the choice
/// is made: Rapier on wasm (where Jolt's C++ cannot be compiled) or under
/// `force-rapier`, Jolt everywhere else.
#[cfg(any(target_arch = "wasm32", feature = "force-rapier"))]
pub type ActiveBackend = backend::rapier::RapierBackend;
/// The physics engine compiled into this build.
///
/// Exactly one exists per build, and this alias is the only place the choice
/// is made: Rapier on wasm (where Jolt's C++ cannot be compiled) or under
/// `force-rapier`, Jolt everywhere else.
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

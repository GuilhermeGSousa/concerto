//! The debug drawing of physics: colliders coloured by how their body moves, each ragdoll's parts and joint limits, contacts, and what a `BodySense` perceives; and the component gizmos of the colliders a scene authors.

mod component;
mod gizmos;
mod ragdoll;
mod shapes;

pub use component::{
    BodySenseGizmo, BoxColliderGizmo, ColliderGizmo, ContactsGizmo, MeshColliderGizmo,
    RagdollGizmo, SensorGizmo,
};
pub use gizmos::{
    PhysicsGizmos, PhysicsGizmosPlugin, activity_color, draw_colliders, draw_contacts,
    draw_ragdolls, draw_senses,
};

#[cfg(test)]
mod tests;

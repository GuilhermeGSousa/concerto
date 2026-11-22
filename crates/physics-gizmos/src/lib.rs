//! The debug drawing of physics: colliders coloured by how their body moves, each ragdoll's parts and joint limits, and contacts; and the component gizmos of the colliders a scene authors.

mod component;
mod gizmos;
mod ragdoll;
mod shapes;

pub use component::{
    BoxColliderGizmo, ColliderGizmo, ContactsGizmo, MeshColliderGizmo, RagdollGizmo, SensorGizmo,
};
pub use gizmos::{
    PhysicsGizmos, PhysicsGizmosPlugin, activity_color, draw_colliders, draw_contacts,
    draw_ragdolls,
};

#[cfg(test)]
mod tests;

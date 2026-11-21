use concerto_debug_gizmos::{ComponentGizmo, DebugGizmos};
use concerto_ecs::{Entity, Query, Res};
use concerto_foundation::transform::GlobalTransform;
use concerto_mesh::Aabb;
use concerto_physics::{
    body::BodyId,
    collider::{BoxCollider, Collider, ColliderOffset},
    contact::Contacts,
    physics_state::PhysicsState,
    ragdoll::RagdollId,
    sensor::Sensor,
    shape::MeshCollider,
};
use glam::Vec3;

use crate::{
    gizmos::{SENSOR_COLOR, STATIC_COLOR, draw_collider, draw_contact_list, draw_ragdoll_of},
    shapes::LocalShape,
};

/// Draws a `BoxCollider`'s box carried by its entity's transform.
pub struct BoxColliderGizmo;

/// Draws the bounds of the mesh a `MeshCollider` entity collides as, carried by its transform.
pub struct MeshColliderGizmo;

/// Draws the shape a `Sensor` entity detects in, from its `Collider`, `BoxCollider` or mesh bounds.
pub struct SensorGizmo;

/// Draws a `Collider`'s shape, coloured by how its body moves once it has a body.
pub struct ColliderGizmo;

/// Draws the crosses and normals of a `Contacts`.
pub struct ContactsGizmo;

/// Draws the ragdoll an entity's `RagdollId` names: its parts and joint limits.
pub struct RagdollGizmo;

impl ComponentGizmo for ColliderGizmo {
    type Component = Collider;
    type Input = (
        Option<Res<'static, PhysicsState>>,
        Query<'static, 'static, (Option<&'static ColliderOffset>, Option<&'static BodyId>)>,
    );
    const NAME: &'static str = "Collider";

    fn draw(
        entity: Entity,
        component: &Collider,
        transform: &GlobalTransform,
        input: &(
            Option<Res<'_, PhysicsState>>,
            Query<'_, '_, (Option<&ColliderOffset>, Option<&BodyId>)>,
        ),
        gizmos: &mut DebugGizmos,
    ) {
        let (state, extras) = input;
        let (offset, body) = extras.get_entity(entity).unwrap_or((None, None));
        draw_collider(
            gizmos,
            component,
            transform,
            offset,
            body.zip(state.as_deref()),
        );
    }
}

impl ComponentGizmo for ContactsGizmo {
    type Component = Contacts;
    type Input = ();
    const NAME: &'static str = "Contacts";

    fn draw(
        _entity: Entity,
        component: &Contacts,
        _transform: &GlobalTransform,
        _input: &(),
        gizmos: &mut DebugGizmos,
    ) {
        draw_contact_list(gizmos, component);
    }
}

impl ComponentGizmo for RagdollGizmo {
    type Component = RagdollId;
    type Input = Option<Res<'static, PhysicsState>>;
    const NAME: &'static str = "Ragdoll";

    fn draw(
        _entity: Entity,
        component: &RagdollId,
        _transform: &GlobalTransform,
        state: &Option<Res<'_, PhysicsState>>,
        gizmos: &mut DebugGizmos,
    ) {
        if let Some(state) = state {
            draw_ragdoll_of(gizmos, state, *component);
        }
    }
}

impl ComponentGizmo for BoxColliderGizmo {
    type Component = BoxCollider;
    type Input = Query<'static, 'static, Option<&'static ColliderOffset>>;
    const NAME: &'static str = "Box collider";

    fn draw(
        entity: Entity,
        component: &BoxCollider,
        transform: &GlobalTransform,
        offsets: &Query<'_, '_, Option<&ColliderOffset>>,
        gizmos: &mut DebugGizmos,
    ) {
        let shape = LocalShape::Cuboid {
            center: Vec3::ZERO,
            half_extents: component.half_extents.abs(),
        };
        shape.draw_at(gizmos, transform, offset(offsets, entity), STATIC_COLOR);
    }
}

impl ComponentGizmo for MeshColliderGizmo {
    type Component = MeshCollider;
    type Input = Query<'static, 'static, (Option<&'static Aabb>, Option<&'static ColliderOffset>)>;
    const NAME: &'static str = "Mesh collider";

    fn draw(
        entity: Entity,
        _component: &MeshCollider,
        transform: &GlobalTransform,
        meshes: &Query<'_, '_, (Option<&Aabb>, Option<&ColliderOffset>)>,
        gizmos: &mut DebugGizmos,
    ) {
        let Some((Some(bounds), offset)) = meshes.get_entity(entity) else {
            return;
        };
        let offset = offset.map_or(Vec3::ZERO, |offset| offset.0);
        LocalShape::bounds(bounds.min, bounds.max).draw_at(gizmos, transform, offset, STATIC_COLOR);
    }
}

impl ComponentGizmo for SensorGizmo {
    type Component = Sensor;
    type Input = Query<
        'static,
        'static,
        (
            Option<&'static Collider>,
            Option<&'static BoxCollider>,
            Option<&'static Aabb>,
            Option<&'static ColliderOffset>,
        ),
    >;
    const NAME: &'static str = "Sensor";

    fn draw(
        entity: Entity,
        _component: &Sensor,
        transform: &GlobalTransform,
        shapes: &Query<
            '_,
            '_,
            (
                Option<&Collider>,
                Option<&BoxCollider>,
                Option<&Aabb>,
                Option<&ColliderOffset>,
            ),
        >,
        gizmos: &mut DebugGizmos,
    ) {
        let Some((collider, box_collider, bounds, offset)) = shapes.get_entity(entity) else {
            return;
        };
        let shape = if let Some(collider) = collider {
            LocalShape::of(collider.shape())
        } else if let Some(box_collider) = box_collider {
            LocalShape::Cuboid {
                center: Vec3::ZERO,
                half_extents: box_collider.half_extents.abs(),
            }
        } else if let Some(bounds) = bounds {
            LocalShape::bounds(bounds.min, bounds.max)
        } else {
            return;
        };
        let offset = offset.map_or(Vec3::ZERO, |offset| offset.0);
        shape.draw_at(gizmos, transform, offset, SENSOR_COLOR);
    }
}

fn offset(offsets: &Query<'_, '_, Option<&ColliderOffset>>, entity: Entity) -> Vec3 {
    offsets
        .get_entity(entity)
        .flatten()
        .map_or(Vec3::ZERO, |offset| offset.0)
}

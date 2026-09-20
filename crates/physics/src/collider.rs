use ecs::component::{Component, ComponentLifecycleCallback};
use ecs::{CommandQueue, Entity, Query, ResMut, Without};

use essential::transform::GlobalTransform;
use glam::Vec3;

use crate::body::BodyId;
use crate::interpolation::TransformInterpolation;
use crate::physics_state::PhysicsState;
use crate::rigid_body::RigidBody;
use crate::shape::{PhysicsShape, SharedPhysicsShape};

#[derive(Clone)]
pub struct Collider {
    shared_shape: SharedPhysicsShape,
}

impl Collider {
    pub fn from_shape(shape: SharedPhysicsShape) -> Self {
        Self {
            shared_shape: shape,
        }
    }

    /// A sphere collider of the given radius.
    pub fn sphere(radius: f32) -> Self {
        Self {
            shared_shape: SharedPhysicsShape::new(PhysicsShape::create_sphere_shape(radius)),
        }
    }

    /// A box collider (`width`/`height`/`length` are half-extents).
    pub fn cuboid(width: f32, height: f32, length: f32) -> Self {
        Self {
            shared_shape: SharedPhysicsShape::new(PhysicsShape::create_cuboid_shape(
                width, height, length,
            )),
        }
    }

    /// A capsule collider (see [`Collider::Capsule`]).
    pub fn capsule(half_height: f32, radius: f32) -> Self {
        Self {
            shared_shape: SharedPhysicsShape::new(PhysicsShape::create_capsule_shape(
                half_height,
                radius,
            )),
        }
    }

    pub fn bottom_offset(&self) -> f32 {
        -self.shared_shape.local_aabb().min().y
    }

    pub(crate) fn shape(&self) -> &PhysicsShape {
        &self.shared_shape
    }
}

/// Local offset of the collider's geometry relative to the entity origin.
/// Optional sibling of [`Collider`], read when the body is created.
#[derive(Component, Clone, Debug)]
pub struct ColliderOffset(pub Vec3);

impl ColliderOffset {
    /// Lifts the shape so its bottom touches the entity origin — e.g. a
    /// standing character whose transform (and skeleton root) sits at the
    /// feet.
    pub fn bottom_origin(collider: &Collider) -> Self {
        Self(Vec3::Y * collider.bottom_offset())
    }
}

impl Component for Collider {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        None
    }

    fn on_despawn() -> Option<ComponentLifecycleCallback> {
        Self::on_remove()
    }

    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            // Both removal and despawn run cleanup before the Collider is
            // dropped; its backend body is tracked by the companion `BodyId`.
            if let Some(&body) = world.get_component_for_entity::<BodyId>(context.entity) {
                if let Some(state) = world.get_resource_mut::<PhysicsState>() {
                    state.destroy_body(body);
                    state.unregister_body_entity(body);
                }

                world.remove_component::<BodyId>(context.entity);
                // Without a body there are no more fixed-step poses; left in
                // place the interpolator would keep rewriting the Transform
                // from stale history.
                if world
                    .get_component_for_entity::<TransformInterpolation>(context.entity)
                    .is_some()
                {
                    world.remove_component::<TransformInterpolation>(context.entity);
                }
            }
        })
    }
}

pub fn register_colliders(
    colliders: Query<
        (
            Entity,
            &GlobalTransform,
            &Collider,
            Option<&RigidBody>,
            Option<&ColliderOffset>,
        ),
        Without<BodyId>,
    >,
    mut physics_state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    for (entity, global_transform, collider, rigid_body, offset) in colliders.iter() {
        let transform = global_transform.to_transform();
        let body_id = physics_state.create_body(collider, &transform, rigid_body, offset);
        physics_state.register_body_entity(body_id, entity);
        cmd.insert(body_id, entity);

        if rigid_body.is_some() {
            cmd.insert(TransformInterpolation::from_transform(&transform), entity);
        }
    }
}

use concerto_ecs::{
    entity::{
        hierarchy::{ChildOf, Children},
        Entity,
    },
    query::{
        filter::{Changed, Without},
        Query,
    },
};
use std::collections::HashSet;

use crate::transform::{GlobalTransform, Transform};

type SimpleEntities<'world, 'state> = Query<
    'world,
    'state,
    (&'static Transform, &'static mut GlobalTransform),
    (Changed<Transform>, Without<ChildOf>, Without<Children>),
>;

pub fn update_simple_entities(roots: SimpleEntities<'_, '_>) {
    for (local_transform, mut global_transform) in roots.iter() {
        global_transform.set_matrix(local_transform.compute_matrix());
    }
}

pub fn propagate_global_transforms(
    roots: Query<(&Children, &mut GlobalTransform, &Transform), Without<ChildOf>>,
    transform_query: Query<(Entity, &mut GlobalTransform, &Transform, Option<&Children>)>,
) {
    let mut visited = HashSet::new();
    let mut stack = Vec::new();

    for (root_children, mut root_global_transform, root_local_transform) in roots.iter() {
        let root_matrix = root_local_transform.compute_matrix();
        root_global_transform.set_matrix(root_matrix);
        stack.extend(
            root_children
                .iter()
                .rev()
                .copied()
                .map(|child| (child, root_matrix)),
        );

        while let Some((entity, parent_matrix)) = stack.pop() {
            if !visited.insert(entity) {
                continue;
            }

            if let Some((_, mut global_transform, local_transform, children)) =
                transform_query.get_entity(entity)
            {
                let matrix = parent_matrix * local_transform.compute_matrix();
                global_transform.set_matrix(matrix);
                if let Some(children) = children {
                    stack.extend(children.iter().rev().copied().map(|child| (child, matrix)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        entity::hierarchy::{ChildOf, Children},
        IntoSystem, System, World,
    };
    use glam::Vec3;

    use super::propagate_global_transforms;
    use crate::transform::{GlobalTransform, Transform};

    #[test]
    fn propagates_a_ten_thousand_deep_hierarchy_without_recursion() {
        let mut world = World::new();
        world.register_component::<Transform>();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();

        let root = world.spawn(Transform::from_translation(Vec3::X));
        let mut leaf = root;
        for _ in 0..10_000 {
            let child = world.spawn(Transform::from_translation(Vec3::X));
            world.entity_mut(leaf).add_child(child);
            leaf = child;
        }

        let mut system = propagate_global_transforms.into_system();
        system.initialize(&mut world);
        system.run_and_apply(&mut world);

        assert_eq!(
            world
                .get_component_for_entity::<GlobalTransform>(leaf)
                .unwrap()
                .translation(),
            Vec3::new(10_001.0, 0.0, 0.0)
        );
    }
}

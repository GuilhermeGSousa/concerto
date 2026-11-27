use concerto_ecs::{
    entity::{
        hierarchy::{ChildOf, Children},
        Entity, EntityHashSet,
    },
    query::{
        change_detection::{DetectChanges, Ref},
        filter::{Changed, Or, With, Without},
        Query,
    },
};
use glam::Mat4;

use crate::transform::{GlobalTransform, Transform};

type SimpleEntities<'world, 'state> = Query<
    'world,
    'state,
    (&'static Transform, &'static mut GlobalTransform),
    (Changed<Transform>, Without<ChildOf>, Without<Children>),
>;

/// Writes the [`GlobalTransform`] of each entity with no parent and no children whose [`Transform`]
/// changed.
pub fn update_simple_entities(mut roots: SimpleEntities<'_, '_>) {
    for (local_transform, mut global_transform) in roots.iter_mut() {
        global_transform.set_if_neq(GlobalTransform::new(local_transform.compute_matrix()));
    }
}

struct Pending {
    entity: Entity,
    parent: Entity,
    parent_matrix: Mat4,
    parent_moved: bool,
}

fn push_children(
    stack: &mut Vec<Pending>,
    children: &Children,
    parent: Entity,
    parent_matrix: Mat4,
    parent_moved: bool,
) {
    stack.extend(children.iter().rev().map(|&entity| Pending {
        entity,
        parent,
        parent_matrix,
        parent_moved,
    }));
}

type HierarchyRoots<'world, 'state> = Query<
    'world,
    'state,
    (
        Entity,
        &'static Children,
        &'static mut GlobalTransform,
        &'static Transform,
    ),
    Without<ChildOf>,
>;

type HierarchyDescendants<'world, 'state> = Query<
    'world,
    'state,
    (
        &'static mut GlobalTransform,
        Ref<'static, Transform>,
        Ref<'static, ChildOf>,
        Option<&'static Children>,
    ),
    With<ChildOf>,
>;

type MovedDescendants<'world, 'state> =
    Query<'world, 'state, &'static ChildOf, Or<(Changed<Transform>, Changed<ChildOf>)>>;

fn ancestors_of_moved(
    moved: &MovedDescendants<'_, '_>,
    parents: &Query<&ChildOf>,
) -> EntityHashSet {
    let mut ancestors = EntityHashSet::default();
    for child_of in moved.iter() {
        let mut ancestor = child_of.parent();
        while ancestors.insert(ancestor) {
            match parents.get_entity(ancestor) {
                Some(child_of) => ancestor = child_of.parent(),
                None => break,
            }
        }
    }
    ancestors
}

fn propagate_to_descendant(
    pending: Pending,
    descendants: &mut HierarchyDescendants<'_, '_>,
    stack: &mut Vec<Pending>,
) {
    let Some((mut global_transform, local_transform, child_of, children)) =
        descendants.get_entity_mut(pending.entity)
    else {
        return;
    };
    if child_of.parent() != pending.parent {
        return;
    }

    if pending.parent_moved || local_transform.is_changed() || child_of.is_changed() {
        let matrix = pending.parent_matrix * local_transform.compute_matrix();
        global_transform.set_if_neq(GlobalTransform::new(matrix));
        if let Some(children) = children {
            push_children(stack, children, pending.entity, matrix, true);
        }
    } else if let Some(children) = children {
        let matrix = global_transform.matrix();
        push_children(stack, children, pending.entity, matrix, false);
    }
}

/// Writes the [`GlobalTransform`] of every entity in a hierarchy whose [`Transform`], or an
/// ancestor's, changed.
pub fn propagate_global_transforms(
    mut roots: HierarchyRoots<'_, '_>,
    mut descendants: HierarchyDescendants<'_, '_>,
    moved: MovedDescendants<'_, '_>,
    parents: Query<&ChildOf>,
) {
    let ancestors = ancestors_of_moved(&moved, &parents);
    let mut stack = Vec::new();

    for (root, root_children, mut root_global_transform, root_local_transform) in roots.iter_mut() {
        let root_matrix = root_local_transform.compute_matrix();
        let stamped = root_global_transform.is_changed();
        let root_moved =
            root_global_transform.set_if_neq(GlobalTransform::new(root_matrix)) || stamped;
        if !root_moved && !ancestors.contains(&root) {
            continue;
        }
        push_children(&mut stack, root_children, root, root_matrix, root_moved);

        while let Some(pending) = stack.pop() {
            propagate_to_descendant(pending, &mut descendants, &mut stack);
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        entity::hierarchy::{ChildOf, Children},
        query::filter::Changed,
        Entity, IntoSystem, System, World,
    };
    use glam::Vec3;
    use std::collections::HashSet;

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
        system.run_and_apply((), &mut world);

        assert_eq!(
            world
                .get_component_for_entity::<GlobalTransform>(leaf)
                .unwrap()
                .translation(),
            Vec3::new(10_001.0, 0.0, 0.0)
        );
    }

    #[test]
    fn repropagating_marks_only_the_global_transforms_that_moved() {
        let mut world = World::new();
        world.register_component::<Transform>();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();

        let root = world.spawn(Transform::from_translation(Vec3::X));
        let child = world.spawn(Transform::from_translation(Vec3::Y));
        let leaf = world.spawn(Transform::from_translation(Vec3::Z));
        let other = world.spawn(Transform::from_translation(Vec3::new(10.0, 0.0, 0.0)));
        world.entity_mut(root).add_child(child);
        world.entity_mut(child).add_child(leaf);

        let mut system = propagate_global_transforms.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);
        world.tick();
        system.run_and_apply((), &mut world);

        let changed = |world: &mut World| -> HashSet<Entity> {
            let mut query = world.query::<Entity, Changed<GlobalTransform>>();
            query.iter(world).collect()
        };
        let translation = |world: &World, entity: Entity| -> Vec3 {
            world
                .get_component_for_entity::<GlobalTransform>(entity)
                .unwrap()
                .translation()
        };
        let translate = |world: &mut World, entity: Entity, to: Vec3| {
            world
                .get_component_for_entity_mut::<Transform>(entity)
                .unwrap()
                .translation = to;
        };
        assert_eq!(changed(&mut world), HashSet::new());
        assert_eq!(translation(&world, leaf), Vec3::new(1.0, 1.0, 1.0));

        world.tick();
        translate(&mut world, root, Vec3::ZERO);
        system.run_and_apply((), &mut world);
        assert_eq!(changed(&mut world), HashSet::from([root, child, leaf]));
        assert_eq!(translation(&world, leaf), Vec3::new(0.0, 1.0, 1.0));

        world.tick();
        translate(&mut world, leaf, Vec3::new(0.0, 0.0, 2.0));
        system.run_and_apply((), &mut world);
        assert_eq!(changed(&mut world), HashSet::from([leaf]));
        assert_eq!(translation(&world, leaf), Vec3::new(0.0, 1.0, 2.0));

        world.tick();
        translate(&mut world, child, Vec3::new(0.0, 3.0, 0.0));
        system.run_and_apply((), &mut world);
        assert_eq!(changed(&mut world), HashSet::from([child, leaf]));
        assert_eq!(translation(&world, leaf), Vec3::new(0.0, 3.0, 2.0));

        world.tick();
        world.entity_mut(other).add_child(child);
        system.run_and_apply((), &mut world);
        assert_eq!(changed(&mut world), HashSet::from([child, leaf]));
        assert_eq!(translation(&world, child), Vec3::new(10.0, 3.0, 0.0));
        assert_eq!(translation(&world, leaf), Vec3::new(10.0, 3.0, 2.0));

        world.tick();
        let spawned = world.spawn(Transform::from_translation(Vec3::X));
        world.entity_mut(leaf).add_child(spawned);
        system.run_and_apply((), &mut world);
        assert_eq!(changed(&mut world), HashSet::from([spawned]));
        assert_eq!(translation(&world, spawned), Vec3::new(11.0, 3.0, 2.0));

        world.tick();
        system.run_and_apply((), &mut world);
        assert_eq!(changed(&mut world), HashSet::new());
    }
}

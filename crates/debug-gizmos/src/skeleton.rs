use std::collections::HashSet;

use concerto_ecs::{Entity, Query, component::scene::SceneEntityRef, entity::hierarchy::ChildOf};
use concerto_foundation::transform::GlobalTransform;
use concerto_mesh::{SkeletonComponent, SkeletonGizmo};

use crate::DebugGizmos;

pub(crate) fn draw_skeleton_gizmos(
    skeletons: Query<(&SkeletonGizmo, &SkeletonComponent)>,
    bones: Query<(&GlobalTransform, Option<&ChildOf>)>,
    mut gizmos: DebugGizmos,
) {
    for (gizmo, skeleton) in skeletons.iter() {
        let entities: Vec<Entity> = skeleton
            .bones()
            .iter()
            .filter_map(|bone| SceneEntityRef::entity(*bone))
            .collect();
        let members: HashSet<Entity> = entities.iter().copied().collect();
        for bone in entities {
            let Some((global, parent)) = bones.get_entity(bone) else {
                continue;
            };
            let joint = global.translation();
            gizmos.sphere(joint, gizmo.joint_size, gizmo.color);
            if gizmo.axes {
                gizmos.axes(&global.to_transform(), gizmo.joint_size * 4.0);
            }
            let Some(parent) = parent
                .map(ChildOf::parent)
                .filter(|parent| members.contains(parent))
            else {
                continue;
            };
            if let Some((parent_global, _)) = bones.get_entity(parent) {
                gizmos.line(parent_global.translation(), joint, gizmo.color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{IntoSystem, System, World};
    use glam::{Mat4, Vec3};

    use super::*;
    use crate::GizmoStorage;

    fn bone(world: &mut World, at: Vec3, parent: Option<Entity>) -> Entity {
        let global = GlobalTransform::new(Mat4::from_translation(at));
        match parent {
            Some(parent) => world.spawn((global, ChildOf::new(parent))),
            None => world.spawn(global),
        }
    }

    fn skeleton(world: &mut World, bones: &[Entity], gizmo: Option<SkeletonGizmo>) {
        let skeleton = SkeletonComponent::new(
            Default::default(),
            bones.iter().copied().map(SceneEntityRef::Entity).collect(),
            None,
        );
        match gizmo {
            Some(gizmo) => world.spawn((skeleton, gizmo)),
            None => world.spawn(skeleton),
        };
    }

    #[derive(Debug, PartialEq)]
    struct Segment {
        start: Vec3,
        end: Vec3,
    }

    fn segments(world: &mut World) -> Vec<Segment> {
        world.insert_resource(GizmoStorage::default());
        let mut system = draw_skeleton_gizmos.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
        world
            .get_resource::<GizmoStorage>()
            .unwrap()
            .vertices
            .chunks(2)
            .map(|pair| Segment {
                start: Vec3::from_array(pair[0].position),
                end: Vec3::from_array(pair[1].position),
            })
            .collect()
    }

    fn chain(world: &mut World) -> [Entity; 4] {
        let owner = world.spawn(GlobalTransform::new(Mat4::IDENTITY));
        let hip = bone(world, Vec3::new(0.0, 1.0, 0.0), Some(owner));
        let knee = bone(world, Vec3::new(0.0, 0.5, 0.0), Some(hip));
        let foot = bone(world, Vec3::new(0.0, 0.0, 0.1), Some(knee));
        [owner, hip, knee, foot]
    }

    fn draw(gizmo: Option<SkeletonGizmo>) -> Vec<Segment> {
        let mut world = World::default();
        let [_, hip, knee, foot] = chain(&mut world);
        skeleton(&mut world, &[hip, knee, foot], gizmo);
        segments(&mut world)
    }

    #[test]
    fn a_skeleton_gizmo_joins_each_bone_to_its_parent_bone_and_draws_what_it_asks_for() {
        let drawn = draw(Some(SkeletonGizmo::default()));
        let hip = Vec3::new(0.0, 1.0, 0.0);
        let knee = Vec3::new(0.0, 0.5, 0.0);
        let foot = Vec3::new(0.0, 0.0, 0.1);
        assert!(drawn.contains(&Segment {
            start: hip,
            end: knee
        }));
        assert!(drawn.contains(&Segment {
            start: knee,
            end: foot
        }));
        assert!(
            !drawn
                .iter()
                .any(|segment| segment.start == Vec3::ZERO || segment.end == Vec3::ZERO),
            "the hip's parent is not a bone, so no line reaches it"
        );
        let joint = drawn.len() - 2;
        assert_eq!(joint % 3, 0, "one sphere per joint");
        assert!(joint > 0);

        let with_axes = draw(Some(SkeletonGizmo {
            axes: true,
            ..Default::default()
        }));
        assert_eq!(with_axes.len(), drawn.len() + 3 * 3, "three axes per bone");

        assert!(draw(None).is_empty(), "no gizmo, no lines");

        let mut world = World::default();
        let [_, hip, _, _] = chain(&mut world);
        world.spawn((
            SkeletonComponent::new(
                Default::default(),
                vec![SceneEntityRef::Entity(hip), SceneEntityRef::Index(7)],
                None,
            ),
            SkeletonGizmo::default(),
        ));
        assert!(
            !segments(&mut world).is_empty(),
            "a bone not spawned yet is skipped and the spawned one still draws"
        );
    }
}

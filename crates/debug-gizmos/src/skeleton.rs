use std::collections::HashSet;

use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, Query,
    component::scene::SceneEntityRef,
    entity::hierarchy::ChildOf,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::transform::GlobalTransform;
use concerto_mesh::skeleton::SkeletonComponent;
use concerto_reflect::{Reflect, ReflectDefault};

use crate::DebugGizmos;

/// Draws the bones of the skeleton on this entity every frame: a line from each bone to its parent bone and a sphere at each joint.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct SkeletonGizmo {
    pub color: Color,
    pub joint_size: f32,
    pub axes: bool,
}

impl Default for SkeletonGizmo {
    fn default() -> Self {
        Self {
            color: Color::srgba(1.0, 0.8, 0.2, 1.0),
            joint_size: 0.02,
            axes: false,
        }
    }
}

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

    fn segments(world: &mut World) -> Vec<(Vec3, Vec3)> {
        world.insert_resource(GizmoStorage::default());
        let mut system = draw_skeleton_gizmos.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
        world
            .get_resource::<GizmoStorage>()
            .unwrap()
            .vertices
            .chunks(2)
            .map(|pair| {
                (
                    Vec3::from_array(pair[0].position),
                    Vec3::from_array(pair[1].position),
                )
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

    #[test]
    fn each_bone_is_joined_to_its_parent_bone_only() {
        let mut world = World::default();
        let [_, hip, knee, foot] = chain(&mut world);
        skeleton(
            &mut world,
            &[hip, knee, foot],
            Some(SkeletonGizmo::default()),
        );

        let segments = segments(&mut world);

        let hip_at = Vec3::new(0.0, 1.0, 0.0);
        let knee_at = Vec3::new(0.0, 0.5, 0.0);
        let foot_at = Vec3::new(0.0, 0.0, 0.1);
        assert!(segments.contains(&(hip_at, knee_at)));
        assert!(segments.contains(&(knee_at, foot_at)));
        assert!(
            !segments
                .iter()
                .any(|&(start, end)| start == Vec3::ZERO || end == Vec3::ZERO),
            "the hip's parent is not a bone, so no line reaches it"
        );
        let joint = segments.len() - 2;
        assert_eq!(joint % 3, 0, "one sphere per joint");
        assert!(joint > 0);
    }

    #[test]
    fn a_skeleton_without_the_gizmo_draws_nothing() {
        let mut world = World::default();
        let [_, hip, knee, foot] = chain(&mut world);
        skeleton(&mut world, &[hip, knee, foot], None);

        assert!(segments(&mut world).is_empty());
    }

    #[test]
    fn axes_add_three_lines_per_bone() {
        let mut world = World::default();
        let [_, hip, knee, foot] = chain(&mut world);
        skeleton(
            &mut world,
            &[hip, knee, foot],
            Some(SkeletonGizmo::default()),
        );
        let without = segments(&mut world).len();

        let mut world = World::default();
        let [_, hip, knee, foot] = chain(&mut world);
        skeleton(
            &mut world,
            &[hip, knee, foot],
            Some(SkeletonGizmo {
                axes: true,
                ..Default::default()
            }),
        );

        assert_eq!(segments(&mut world).len(), without + 3 * 3);
    }

    #[test]
    fn a_bone_that_is_not_spawned_yet_is_skipped() {
        let mut world = World::default();
        let [_, hip, _, _] = chain(&mut world);
        let skeleton_component = SkeletonComponent::new(
            Default::default(),
            vec![SceneEntityRef::Entity(hip), SceneEntityRef::Index(7)],
            None,
        );
        world.spawn((skeleton_component, SkeletonGizmo::default()));

        let segments = segments(&mut world);

        assert!(
            !segments.is_empty(),
            "the spawned bone still draws its joint"
        );
    }
}

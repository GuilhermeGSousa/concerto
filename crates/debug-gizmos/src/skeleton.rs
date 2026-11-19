use std::collections::HashSet;

use concerto_color::Color;
use concerto_ecs::{Entity, Query, component::scene::SceneEntityRef, entity::hierarchy::ChildOf};
use concerto_foundation::transform::GlobalTransform;
use concerto_mesh::SkeletonComponent;

use crate::DebugGizmos;

pub(crate) struct BoneStyle {
    pub(crate) color: Color,
    pub(crate) joint_size: f32,
    pub(crate) axes: bool,
}

pub(crate) fn draw_skeleton_gizmos(
    skeletons: Query<(&concerto_mesh::SkeletonGizmo, &SkeletonComponent)>,
    bones: Query<(&GlobalTransform, Option<&ChildOf>)>,
    mut gizmos: DebugGizmos,
) {
    for (gizmo, skeleton) in skeletons.iter() {
        let style = BoneStyle {
            color: gizmo.color,
            joint_size: gizmo.joint_size,
            axes: gizmo.axes,
        };
        draw_skeleton(skeleton, &bones, &style, &mut gizmos);
    }
}

pub(crate) fn draw_skeleton(
    skeleton: &SkeletonComponent,
    bones: &Query<(&GlobalTransform, Option<&ChildOf>)>,
    style: &BoneStyle,
    gizmos: &mut DebugGizmos,
) {
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
        gizmos.sphere(joint, style.joint_size, style.color);
        if style.axes {
            gizmos.axes(&global.to_transform(), style.joint_size * 4.0);
        }
        let Some(parent) = parent
            .map(ChildOf::parent)
            .filter(|parent| members.contains(parent))
        else {
            continue;
        };
        if let Some((parent_global, _)) = bones.get_entity(parent) {
            gizmos.line(parent_global.translation(), joint, style.color);
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{IntoSystem, System, World};
    use glam::{Mat4, Vec3};

    use super::*;
    use crate::GizmoStorage;
    use concerto_mesh::SkeletonGizmo;

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
    fn a_skeleton_gizmo_joins_each_bone_to_its_parent_bone() {
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
        assert!(draw(None).is_empty(), "no gizmo, no lines");
    }
}

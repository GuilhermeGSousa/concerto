use std::f32::consts::TAU;

use concerto_color::Color;
use concerto_debug_gizmos::DebugGizmos;
use concerto_physics::ragdoll::{
    BodyPose, JointSettings, PartSettings, PartShape, RagdollSettings,
};
use glam::{Quat, Vec3};

const FRAME_SIZE: f32 = 0.12;
const CONE_LENGTH: f32 = 0.2;
const CONE_RIBS: usize = 4;
const CONE_STEPS: usize = 24;
const TWIST_RADIUS: f32 = 0.1;
const SWING_COLOR: Color = Color::rgba(1.0, 0.8, 0.2, 1.0);
const TWIST_COLOR: Color = Color::rgba(0.9, 0.4, 1.0, 1.0);
const CURRENT_COLOR: Color = Color::rgba(1.0, 1.0, 1.0, 1.0);

pub(crate) fn draw_ragdoll(
    gizmos: &mut DebugGizmos,
    settings: &RagdollSettings,
    pose: &[BodyPose],
    colors: &[Color],
) {
    if pose.len() != settings.parts.len() {
        return;
    }
    for (index, part) in settings.parts.iter().enumerate() {
        let color = colors.get(index).copied().unwrap_or(Color::WHITE);
        draw_part(gizmos, part, pose[index], color);
        if let Some(parent) = part.parent {
            let frame = JointFrame {
                joint: &part.joint,
                position: pose[index].translation,
                parent_rotation: pose[parent].rotation
                    * settings.parts[parent].pose.rotation.inverse(),
                child_rotation: pose[index].rotation * part.pose.rotation.inverse(),
            };
            draw_joint(gizmos, &frame);
        }
    }
}

fn draw_part(gizmos: &mut DebugGizmos, part: &PartSettings, pose: BodyPose, color: Color) {
    let shape = pose.then(part.shape_offset);
    match part.shape {
        PartShape::Sphere { radius } => gizmos.sphere(shape.translation, radius, color),
        PartShape::Capsule {
            half_height,
            radius,
        } => gizmos.capsule(
            shape.translation,
            shape.rotation,
            half_height,
            radius,
            color,
        ),
        PartShape::Box { half_extents } => gizmos.cuboid(
            &concerto_foundation::transform::Transform::from_translation_rotation_scale(
                shape.translation,
                shape.rotation,
                half_extents * 2.0,
            ),
            color,
        ),
    }
}

pub(crate) struct JointFrame<'a> {
    pub(crate) joint: &'a JointSettings,
    pub(crate) position: Vec3,
    pub(crate) parent_rotation: Quat,
    pub(crate) child_rotation: Quat,
}

fn draw_joint(gizmos: &mut DebugGizmos, frame: &JointFrame) {
    let joint = frame.joint;
    let Some(twist) = joint.twist_axis.try_normalize() else {
        return;
    };
    let plane = (joint.plane_axis - twist * joint.plane_axis.dot(twist))
        .try_normalize()
        .unwrap_or_else(|| twist.any_orthonormal_vector());
    let normal = twist.cross(plane);
    let parent = |axis: Vec3| frame.parent_rotation * axis;
    let (twist_w, plane_w, normal_w) = (parent(twist), parent(plane), parent(normal));
    let origin = frame.position;

    gizmos.line(origin, origin + twist_w * FRAME_SIZE, Color::BLUE);
    gizmos.line(origin, origin + plane_w * FRAME_SIZE, Color::RED);
    gizmos.line(origin, origin + normal_w * FRAME_SIZE, Color::GREEN);

    let rim = |angle: f32| {
        let (sin, cos) = angle.sin_cos();
        let swing = Quat::from_scaled_axis(
            normal_w * joint.normal_half_cone * cos + plane_w * joint.plane_half_cone * sin,
        );
        origin + swing * twist_w * CONE_LENGTH
    };
    let mut previous = rim(0.0);
    for step in 1..=CONE_STEPS {
        let next = rim(TAU * step as f32 / CONE_STEPS as f32);
        gizmos.line(previous, next, SWING_COLOR);
        previous = next;
    }
    for rib in 0..CONE_RIBS {
        gizmos.line(
            origin,
            rim(TAU * rib as f32 / CONE_RIBS as f32),
            SWING_COLOR,
        );
    }

    let twist_origin = origin + twist_w * CONE_LENGTH * 0.5;
    let from_min = Quat::from_axis_angle(twist_w, joint.twist_min) * plane_w * TWIST_RADIUS;
    gizmos.line(twist_origin, twist_origin + from_min, TWIST_COLOR);
    gizmos.line(
        twist_origin,
        twist_origin + Quat::from_axis_angle(twist_w, joint.twist_max) * plane_w * TWIST_RADIUS,
        TWIST_COLOR,
    );
    gizmos.arc(
        twist_origin,
        twist_w,
        from_min,
        joint.twist_max - joint.twist_min,
        TWIST_COLOR,
    );

    let child = |axis: Vec3| frame.child_rotation * axis;
    gizmos.line(origin, origin + child(twist) * CONE_LENGTH, CURRENT_COLOR);
    gizmos.line(
        twist_origin,
        twist_origin + child(plane) * TWIST_RADIUS,
        CURRENT_COLOR,
    );
}

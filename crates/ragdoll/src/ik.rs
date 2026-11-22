use concerto_physics::ragdoll::{BodyPose, PartSettings};
use glam::{Quat, Vec3};

pub(crate) struct Reach {
    pub(crate) upper: Quat,
    pub(crate) lower: Quat,
}

pub(crate) struct Chain {
    pub(crate) upper: usize,
    pub(crate) lower: usize,
    pub(crate) end: usize,
}

pub(crate) fn reach(
    chain: &Chain,
    rest: &[BodyPose],
    carried: &[Quat],
    hip: Vec3,
    ankle: Vec3,
    forward: Vec3,
) -> Reach {
    let upper = rest[chain.lower].translation - rest[chain.upper].translation;
    let lower = rest[chain.end].translation - rest[chain.lower].translation;
    let upper_length = upper.length().max(1e-3);
    let lower_length = lower.length().max(1e-3);
    let to_ankle = ankle - hip;
    let distance = to_ankle.length().clamp(
        (upper_length - lower_length).abs() + 1e-3,
        (upper_length + lower_length) * 0.999,
    );
    let direction = to_ankle.try_normalize().unwrap_or(Vec3::NEG_Y);
    let cosine = ((upper_length * upper_length + distance * distance
        - lower_length * lower_length)
        / (2.0 * upper_length * distance))
        .clamp(-1.0, 1.0);
    let bend = (forward - direction * forward.dot(direction))
        .try_normalize()
        .unwrap_or_else(|| direction.any_orthonormal_vector());
    let knee = hip
        + direction * (upper_length * cosine)
        + bend * (upper_length * (1.0 - cosine * cosine).max(0.0).sqrt());
    let foot = hip + direction * distance;
    let upper_local = rest[chain.upper].rotation.inverse() * upper / upper_length;
    let lower_local = rest[chain.lower].rotation.inverse() * lower / lower_length;
    let thigh = (Quat::from_rotation_arc(
        carried[chain.upper] * upper_local,
        (knee - hip).try_normalize().unwrap_or(direction),
    ) * carried[chain.upper])
        .normalize();
    let shin_carried = thigh * (carried[chain.upper].inverse() * carried[chain.lower]);
    let shin = (Quat::from_rotation_arc(
        shin_carried * lower_local,
        (foot - knee).try_normalize().unwrap_or(direction),
    ) * shin_carried)
        .normalize();
    Reach {
        upper: thigh,
        lower: shin,
    }
}

pub(crate) fn reach_hinged(
    chain: &Chain,
    parts: &[PartSettings],
    rest: &[BodyPose],
    carried: &[Quat],
    root: Vec3,
    goal: Vec3,
    bend: Vec3,
) -> Reach {
    let aimed = reach(chain, rest, carried, root, goal, bend);
    let hinge = &parts[chain.lower].joint;
    if hinge.normal_half_cone.max(hinge.plane_half_cone) >= 0.1 {
        return aimed;
    }
    let upper_local = rest[chain.upper].rotation.inverse()
        * (rest[chain.lower].translation - rest[chain.upper].translation).normalize_or_zero();
    let lower_local = rest[chain.lower].rotation.inverse()
        * (rest[chain.end].translation - rest[chain.lower].translation).normalize_or_zero();
    let along = aimed.upper * upper_local;
    let fore = aimed.lower * lower_local;
    let flat = |vector: Vec3| (vector - along * vector.dot(along)).try_normalize();
    let (Some(axis), Some(bent)) = (
        flat(aimed.upper * (rest[chain.upper].rotation.inverse() * hinge.twist_axis)),
        along.cross(fore).try_normalize().and_then(flat),
    ) else {
        return aimed;
    };
    let upper = (Quat::from_rotation_arc(axis, bent) * aimed.upper).normalize();
    let hanging = upper * (carried[chain.upper].inverse() * carried[chain.lower]);
    let lower = (Quat::from_rotation_arc(hanging * lower_local, fore) * hanging).normalize();
    Reach { upper, lower }
}

pub(crate) struct Aimed {
    pub(crate) part: usize,
    pub(crate) target: Quat,
}

pub(crate) fn bent_spine(
    spine: &[usize],
    parts: &[PartSettings],
    animated: &[BodyPose],
    bend: Vec3,
) -> Vec<Aimed> {
    let angle = bend.length();
    if angle < 1e-4 || spine.is_empty() {
        return Vec::new();
    }
    let axis = bend / angle;
    let share = angle / spine.len() as f32;
    spine
        .iter()
        .enumerate()
        .filter_map(|(index, &part)| {
            let parent = parts[part].parent?;
            let bent = Quat::from_axis_angle(axis, share * (index + 1) as f32);
            let below = Quat::from_axis_angle(axis, share * index as f32);
            Some(Aimed {
                part,
                target: (animated[parent].rotation.inverse()
                    * below.inverse()
                    * bent
                    * animated[part].rotation)
                    .normalize(),
            })
        })
        .collect()
}

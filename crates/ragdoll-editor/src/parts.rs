use concerto_mesh::Skeleton;
use concerto_name::Name;
use concerto_ragdoll::{Ragdoll, RagdollJoint, RagdollPart, RagdollShape, Trial, run_trial};
use glam::{Quat, Vec3};

const DEFAULT_RADIUS: f32 = 0.05;
const DEFAULT_MASS: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShapeKind {
    Capsule,
    Box,
    Sphere,
}

impl ShapeKind {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        match text.to_ascii_lowercase().as_str() {
            "capsule" => Ok(Self::Capsule),
            "box" => Ok(Self::Box),
            "sphere" => Ok(Self::Sphere),
            other => Err(format!(
                "`{other}` is not a shape; use capsule, box or sphere."
            )),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PartEdit {
    pub(crate) remove: bool,
    pub(crate) shape: Option<ShapeKind>,
    pub(crate) radius: Option<f32>,
    pub(crate) half_height: Option<f32>,
    pub(crate) half_extents: Option<Vec3>,
    pub(crate) mass: Option<f32>,
    pub(crate) offset: Option<Vec3>,
    pub(crate) swing_normal: Option<f32>,
    pub(crate) swing_plane: Option<f32>,
    pub(crate) twist_min: Option<f32>,
    pub(crate) twist_max: Option<f32>,
    pub(crate) friction: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PartRow {
    pub(crate) bone: Name,
    pub(crate) parent: Option<Name>,
    pub(crate) depth: usize,
}

pub(crate) fn part_rows(ragdoll: &Ragdoll, skeleton: &Skeleton) -> Vec<PartRow> {
    let parent_of: Vec<Option<usize>> = ragdoll
        .parts
        .iter()
        .map(|part| {
            let mut bone = skeleton.bone_index(part.bone.0)?;
            while let Some(parent) = skeleton.parents[bone] {
                bone = usize::from(parent);
                if let Some(found) = ragdoll
                    .parts
                    .iter()
                    .position(|other| other.bone.0 == skeleton.names[bone])
                {
                    return Some(found);
                }
            }
            None
        })
        .collect();
    let mut order: Vec<usize> = (0..ragdoll.parts.len()).collect();
    order.sort_by_key(|part| {
        skeleton
            .bone_index(ragdoll.parts[*part].bone.0)
            .unwrap_or(usize::MAX)
    });
    let mut rows = Vec::with_capacity(order.len());
    let mut stack: Vec<(usize, usize)> = order
        .iter()
        .rev()
        .filter(|part| parent_of[**part].is_none())
        .map(|part| (*part, 0))
        .collect();
    while let Some((part, depth)) = stack.pop() {
        rows.push(PartRow {
            bone: ragdoll.parts[part].bone.0,
            parent: parent_of[part].map(|parent| ragdoll.parts[parent].bone.0),
            depth,
        });
        stack.extend(
            order
                .iter()
                .rev()
                .filter(|child| parent_of[**child] == Some(part))
                .map(|child| (*child, depth + 1)),
        );
    }
    rows
}

pub(crate) fn bones_without_part(ragdoll: &Ragdoll, skeleton: &Skeleton) -> Vec<Name> {
    skeleton
        .names
        .iter()
        .filter(|name| ragdoll.parts.iter().all(|part| part.bone.0 != **name))
        .copied()
        .collect()
}

pub(crate) fn shape_text(shape: &RagdollShape) -> String {
    match shape {
        RagdollShape::Capsule {
            half_height,
            radius,
        } => format!("capsule half_height {half_height:.3} radius {radius:.3}"),
        RagdollShape::Box { half_extents } => format!(
            "box half_extents [{:.3}, {:.3}, {:.3}]",
            half_extents.x, half_extents.y, half_extents.z
        ),
        RagdollShape::Sphere { radius } => format!("sphere radius {radius:.3}"),
    }
}

fn limits_text(joint: &RagdollJoint) -> String {
    format!(
        "swing_normal {:.2} swing_plane {:.2} twist {:.2}..{:.2} friction {:.2}",
        joint.swing_normal, joint.swing_plane, joint.twist_min, joint.twist_max, joint.friction
    )
}

pub(crate) fn part_text(part: &RagdollPart, row: &PartRow) -> String {
    let parent = row
        .parent
        .map_or_else(|| "root".to_owned(), |parent| format!("under {parent}"));
    let joint = if row.parent.is_some() {
        format!(" · {}", limits_text(&part.joint))
    } else {
        String::new()
    };
    format!(
        "{}{} · {parent} · {} · {:.2} kg{joint}",
        "  ".repeat(row.depth),
        part.bone.0,
        shape_text(&part.shape),
        part.mass
    )
}

pub(crate) fn parts_text(ragdoll: &Ragdoll, skeleton: &Skeleton) -> Vec<String> {
    part_rows(ragdoll, skeleton)
        .iter()
        .filter_map(|row| {
            ragdoll
                .part(row.bone.as_str())
                .map(|part| part_text(part, row))
        })
        .collect()
}

fn positive(name: &str, value: Option<f32>) -> Result<(), String> {
    match value {
        Some(value) if !(value.is_finite() && value > 0.0) => {
            Err(format!("{name} is {value}; give a size above 0 in metres."))
        }
        _ => Ok(()),
    }
}

fn angle(name: &str, value: Option<f32>) -> Result<(), String> {
    match value {
        Some(value) if !value.is_finite() => Err(format!("{name} is {value}; give radians.")),
        _ => Ok(()),
    }
}

fn started_part(skeleton: &Skeleton, bone: usize) -> RagdollPart {
    let name = skeleton.names[bone];
    let child = (0..skeleton.bone_count())
        .find(|child| skeleton.parents[*child].is_some_and(|parent| usize::from(parent) == bone));
    let Some(along) = child.map(|child| skeleton.rest_pose[child].translation) else {
        return RagdollPart::new(
            name.as_str(),
            RagdollShape::Sphere {
                radius: DEFAULT_RADIUS,
            },
        )
        .with_mass(DEFAULT_MASS);
    };
    let length = along.length().max(2.0 * DEFAULT_RADIUS);
    let direction = if along.length_squared() > 1e-8 {
        along.normalize()
    } else {
        Vec3::Y
    };
    RagdollPart::new(
        name.as_str(),
        RagdollShape::Capsule {
            half_height: (length / 2.0 - DEFAULT_RADIUS).max(0.01),
            radius: DEFAULT_RADIUS,
        },
    )
    .with_offset(along / 2.0)
    .with_rotation(Quat::from_rotation_arc(Vec3::Y, direction))
    .with_mass(DEFAULT_MASS)
    .with_joint(RagdollJoint::new(
        direction,
        direction.any_orthonormal_vector(),
    ))
}

fn reshaped(old: &RagdollShape, edit: &PartEdit) -> RagdollShape {
    let kind = edit.shape.unwrap_or(match old {
        RagdollShape::Capsule { .. } => ShapeKind::Capsule,
        RagdollShape::Box { .. } => ShapeKind::Box,
        RagdollShape::Sphere { .. } => ShapeKind::Sphere,
    });
    let old_radius = match old {
        RagdollShape::Capsule { radius, .. } | RagdollShape::Sphere { radius } => *radius,
        RagdollShape::Box { half_extents } => half_extents.min_element(),
    };
    let old_half_height = match old {
        RagdollShape::Capsule { half_height, .. } => *half_height,
        RagdollShape::Box { half_extents } => half_extents.y,
        RagdollShape::Sphere { radius } => *radius,
    };
    let radius = edit.radius.unwrap_or(old_radius);
    let half_height = edit.half_height.unwrap_or(old_half_height);
    match kind {
        ShapeKind::Capsule => RagdollShape::Capsule {
            half_height,
            radius,
        },
        ShapeKind::Sphere => RagdollShape::Sphere { radius },
        ShapeKind::Box => RagdollShape::Box {
            half_extents: edit.half_extents.unwrap_or(match old {
                RagdollShape::Box { half_extents } => *half_extents,
                _ => Vec3::new(radius, half_height, radius),
            }),
        },
    }
}

fn unfitting(ragdoll: &Ragdoll, skeleton: &Skeleton) -> String {
    match ragdoll.resolve(skeleton) {
        Ok(_) => String::new(),
        Err(error) => format!(" Warning: the ragdoll no longer fits its skeleton: {error}."),
    }
}

pub(crate) fn apply_edit(
    ragdoll: &mut Ragdoll,
    skeleton: &Skeleton,
    bone: &str,
    edit: &PartEdit,
) -> Result<String, String> {
    let Some(index) = skeleton.bone_index(Name::new(bone)) else {
        return Err(format!(
            "The skeleton has no bone `{bone}`; ragdoll_parts lists the bones that have a part."
        ));
    };
    positive("radius", edit.radius)?;
    positive("half_height", edit.half_height)?;
    positive("mass", edit.mass)?;
    if let Some(extents) = edit.half_extents {
        positive(
            "half_extents",
            extents.to_array().into_iter().reduce(f32::min),
        )?;
    }
    for (name, value) in [
        ("swing_normal", edit.swing_normal),
        ("swing_plane", edit.swing_plane),
        ("twist_min", edit.twist_min),
        ("twist_max", edit.twist_max),
        ("friction", edit.friction),
    ] {
        angle(name, value)?;
    }
    if edit.remove {
        let before = ragdoll.parts.len();
        ragdoll.parts.retain(|part| &*part.bone != bone);
        if ragdoll.parts.len() == before {
            return Err(format!(
                "`{bone}` has no part to remove; ragdoll_parts lists the parts."
            ));
        }
        return Ok(format!(
            "Removed the part on {bone}; {} parts left.{}",
            ragdoll.parts.len(),
            unfitting(ragdoll, skeleton)
        ));
    }
    let added = ragdoll.part(bone).is_none();
    if added {
        ragdoll.parts.push(started_part(skeleton, index));
    }
    let part = ragdoll
        .part_mut(bone)
        .expect("the part exists or was just added");
    part.shape = reshaped(&part.shape, edit);
    if let Some(mass) = edit.mass {
        part.mass = mass;
    }
    if let Some(offset) = edit.offset {
        part.offset = offset;
    }
    let joint = &mut part.joint;
    if let Some(value) = edit.swing_normal {
        joint.swing_normal = value;
    }
    if let Some(value) = edit.swing_plane {
        joint.swing_plane = value;
    }
    if let Some(value) = edit.twist_min {
        joint.twist_min = value;
    }
    if let Some(value) = edit.twist_max {
        joint.twist_max = value;
    }
    if let Some(value) = edit.friction {
        joint.friction = value;
    }
    let shown = parts_text(ragdoll, skeleton)
        .into_iter()
        .find(|line| line.trim_start().starts_with(bone))
        .unwrap_or_default();
    Ok(format!(
        "{} {}.{}",
        if added { "Added" } else { "Set" },
        shown.trim_start(),
        unfitting(ragdoll, skeleton)
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TrialAction {
    Drop { height: f32 },
    Poke { bone: String, impulse: Vec3 },
}

pub(crate) fn trial_report(
    ragdoll: &Ragdoll,
    skeleton: &Skeleton,
    action: &TrialAction,
    seconds: f32,
) -> Result<String, String> {
    if !(seconds.is_finite() && seconds > 0.0) {
        return Err(format!("seconds is {seconds}; give a duration above 0."));
    }
    let resolved = ragdoll.resolve(skeleton).map_err(|error| {
        format!("The ragdoll does not fit its skeleton: {error}. Fix it with set_ragdoll_part.")
    })?;
    let (trial, what) = match action {
        TrialAction::Drop { height } => (
            Trial::Drop { height: *height },
            format!("Drop from {height:.2} m"),
        ),
        TrialAction::Poke { bone, impulse } => {
            if resolved.part_of(bone).is_none() {
                return Err(format!(
                    "The ragdoll has no part on `{bone}`; ragdoll_parts lists the parts."
                ));
            }
            (
                Trial::Poke {
                    bone: Name::new(bone),
                    impulse: *impulse,
                },
                format!(
                    "Poke {bone} with [{:.1}, {:.1}, {:.1}] N·s",
                    impulse.x, impulse.y, impulse.z
                ),
            )
        }
    };
    let report = run_trial(&resolved, trial, seconds)
        .ok_or_else(|| "The physics backend could not run the trial.".to_owned())?;
    let mut lines = vec![format!("{what}, {seconds:.1} s.")];
    lines.push(match report.rested_after {
        Some(after) => format!("rest: came to rest after {after:.1} s"),
        None => format!("rest: still moving after {seconds:.1} s"),
    });
    lines.push(format!("peak speed: {:.1} m/s", report.peak_speed));
    lines.push(format!("widest joint gap: {:.3} m", report.peak_joint_gap));
    if report.overlaps.is_empty() {
        lines.push("interpenetration: none".to_owned());
    } else {
        lines.push(format!(
            "interpenetration: {} pair(s) overlap in the rest pose",
            report.overlaps.len()
        ));
        for pair in &report.overlaps {
            let name = |part| {
                resolved
                    .bone_name(part)
                    .map_or("?".to_owned(), |name| name.as_str().to_owned())
            };
            lines.push(format!(
                "  {} and {} reach {:.3} m into each other",
                name(pair.first),
                name(pair.second),
                pair.depth
            ));
        }
    }
    Ok(lines.join("\n"))
}

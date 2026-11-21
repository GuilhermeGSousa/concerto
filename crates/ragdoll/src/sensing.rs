use concerto_ecs::{Component, Entity, Query, ResMut, Without};
use concerto_foundation::time::Time;
use concerto_physics::{
    physics_state::PhysicsState,
    query::QueryFilter,
    ragdoll::{BodyPose, RagdollId},
};
use glam::Vec3;

use crate::{character::RagdollBody, dying::Settled, overlap::rounded_at};

const GRAVITY: f32 = 9.81;
const GROUND_NORMAL: f32 = 0.5;
const FOOT_REACH: f32 = 0.05;
const GROUND_REACH: f32 = 5.0;
const REST_SPEED: f32 = 0.2;
const REST_SECONDS: f32 = 0.5;
const IMPACT_SECONDS: f32 = 1.5;
const IMPACT_SAMPLES: usize = 6;

/// A mass at a place, moving.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MassPoint {
    pub mass: f32,
    pub position: Vec3,
    pub velocity: Vec3,
}

/// The total mass of `points` at their centre of mass, moving at its velocity.
pub fn centre_of_mass(points: &[MassPoint]) -> MassPoint {
    let mass: f32 = points.iter().map(|point| point.mass).sum();
    if mass <= 0.0 {
        return MassPoint::default();
    }
    MassPoint {
        mass,
        position: points
            .iter()
            .map(|point| point.position * point.mass)
            .sum::<Vec3>()
            / mass,
        velocity: points
            .iter()
            .map(|point| point.velocity * point.mass)
            .sum::<Vec3>()
            / mass,
    }
}

/// The spot on ground at height `ground` where a body with its centre of mass at `centre`, moving at `velocity`, would come to rest over its foot if it stepped there now: the ground under the centre plus the horizontal velocity times `sqrt(h / g)`.
pub fn capture_point(centre: Vec3, velocity: Vec3, ground: f32) -> Vec3 {
    let height = (centre.y - ground).max(0.0);
    let reach = (height / GRAVITY).sqrt();
    Vec3::new(
        centre.x + velocity.x * reach,
        ground,
        centre.z + velocity.z * reach,
    )
}

fn cross(origin: Vec3, a: Vec3, b: Vec3) -> f32 {
    (a.x - origin.x) * (b.z - origin.z) - (a.z - origin.z) * (b.x - origin.x)
}

/// The convex outline of `points` seen from above, going round once; heights are kept. Fewer than three distinct points, or points in a line, give what there is.
pub fn convex_hull(points: &[Vec3]) -> Vec<Vec3> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));
    sorted.dedup_by(|a, b| (a.x - b.x).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6);
    if sorted.len() < 3 {
        return sorted;
    }
    let mut hull: Vec<Vec3> = Vec::with_capacity(sorted.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Box<dyn Iterator<Item = &Vec3>> = if pass == 0 {
            Box::new(sorted.iter())
        } else {
            Box::new(sorted.iter().rev())
        };
        for point in ordered {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0
            {
                hull.pop();
            }
            hull.push(*point);
        }
        hull.pop();
    }
    hull
}

fn nearest_on_edge(a: Vec3, b: Vec3, point: Vec3) -> Vec3 {
    let edge = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
    let length = edge.length_squared();
    if length < 1e-12 {
        return a;
    }
    let along = (Vec3::new(point.x - a.x, 0.0, point.z - a.z).dot(edge) / length).clamp(0.0, 1.0);
    a + (b - a) * along
}

/// How far `point` is outside the convex `polygon` seen from above, as the horizontal vector from the polygon's nearest edge to the point; zero inside. A polygon of fewer than three points has no inside.
pub fn outside_by(polygon: &[Vec3], point: Vec3) -> Vec3 {
    if polygon.is_empty() {
        return Vec3::ZERO;
    }
    let inside = polygon.len() >= 3 && {
        let mut sign = 0.0f32;
        polygon.iter().enumerate().all(|(index, a)| {
            let turn = cross(*a, polygon[(index + 1) % polygon.len()], point);
            if turn.abs() < 1e-9 {
                return true;
            }
            if sign == 0.0 {
                sign = turn.signum();
            }
            turn.signum() == sign
        })
    };
    if inside {
        return Vec3::ZERO;
    }
    let nearest = (0..polygon.len())
        .map(|index| nearest_on_edge(polygon[index], polygon[(index + 1) % polygon.len()], point))
        .min_by(|a, b| {
            let da = (a.x - point.x).powi(2) + (a.z - point.z).powi(2);
            let db = (b.x - point.x).powi(2) + (b.z - point.z).powi(2);
            da.total_cmp(&db)
        })
        .unwrap_or(point);
    Vec3::new(point.x - nearest.x, 0.0, point.z - nearest.z)
}

/// Where and when a part will meet something on its current path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    pub point: Vec3,
    pub normal: Vec3,
    /// Seconds from now.
    pub seconds: f32,
}

/// What a character's body knows about itself, rewritten after each fixed step. Add it beside a `CharacterRagdoll` to have it kept; its ragdoll's parts then report their contacts.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct BodySense {
    /// The mass-weighted centre of the parts, with the body's mass and the centre's velocity.
    pub centre: MassPoint,
    /// The height of the ground under the supporting feet, or under the centre of mass when none supports.
    pub ground: f32,
    /// The foot parts that are on the ground.
    pub supporting: Vec<usize>,
    /// The convex outline, on the ground, of the feet that are on it.
    pub support: Vec<Vec3>,
    /// Where the body would have to step to come to rest; inside `support` it can recover without stepping.
    pub capture_point: Vec3,
    /// From the support's nearest edge to the capture point; zero while the capture point is inside.
    pub off_balance: Vec3,
    /// The angle between the pelvis-to-chest axis and up, in radians.
    pub lean: f32,
    /// The angular velocity of the chest, in rad/s.
    pub trunk_spin: Vec3,
    /// Seconds since any part last touched something that is not the body itself.
    pub airborne: f32,
    /// Where the chest will meet the ground on its current path, while no foot supports the body.
    pub chest_impact: Option<Impact>,
    /// Where the head will meet the ground on its current path, while no foot supports the body.
    pub head_impact: Option<Impact>,
    /// Whether every part has been slower than 0.2 m/s for half a second.
    pub at_rest: bool,
    still: f32,
    watching: Option<RagdollId>,
}

impl BodySense {
    pub(crate) fn disturb(&mut self) {
        self.still = 0.0;
        self.at_rest = false;
    }

    /// Whether the capture point is inside the support: the body can stay up without a step.
    pub fn balanced(&self) -> bool {
        self.support.len() >= 3 && self.off_balance == Vec3::ZERO
    }
}

fn ground_below(state: &PhysicsState, point: Vec3, reach: f32) -> Option<f32> {
    state
        .cast_ray(
            point + Vec3::Y * 0.1,
            Vec3::NEG_Y * (reach + 0.1),
            QueryFilter::default(),
        )
        .map(|hit| hit.point.y)
}

fn predicted_impact(state: &PhysicsState, from: Vec3, velocity: Vec3) -> Option<Impact> {
    let step = IMPACT_SECONDS / IMPACT_SAMPLES as f32;
    let at = |time: f32| from + velocity * time - Vec3::Y * (0.5 * GRAVITY * time * time);
    for sample in 0..IMPACT_SAMPLES {
        let start = sample as f32 * step;
        let origin = at(start);
        let path = at(start + step) - origin;
        if let Some(hit) = state.cast_ray(origin, path, QueryFilter::default()) {
            return Some(Impact {
                point: hit.point,
                normal: hit.normal,
                seconds: start + hit.fraction * step,
            });
        }
    }
    None
}

/// After each physics step, rewrites every `BodySense` from its ragdoll: centre of mass, support, capture point, lean, time in the air, predicted impacts and rest.
pub fn sense_bodies(
    characters: Query<(Entity, &RagdollId, &RagdollBody, &mut BodySense), Without<Settled>>,
    mut state: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut pose: Vec<BodyPose> = Vec::new();
    for (entity, id, body, mut sense) in characters.iter() {
        let bodies = state.ragdoll_bodies(*id).to_vec();
        if sense.watching != Some(*id) {
            for part in &bodies {
                state.set_contact_reporting(*part, true);
            }
            sense.watching = Some(*id);
        }
        state.ragdoll_pose(*id, &mut pose);
        let settings = &body.resolved().settings().parts;
        if pose.len() != settings.len() || bodies.len() != settings.len() {
            continue;
        }
        let landmarks = body.resolved().landmarks();
        let sense = &mut *sense;

        let velocities: Vec<Vec3> = bodies
            .iter()
            .map(|part| state.linear_velocity(*part))
            .collect();
        let points: Vec<MassPoint> = settings
            .iter()
            .enumerate()
            .map(|(part, settings)| MassPoint {
                mass: settings.mass,
                position: pose[part].then(settings.shape_offset).translation,
                velocity: velocities[part],
            })
            .collect();
        sense.centre = centre_of_mass(&points);

        let mut touching = false;
        let mut footprint = Vec::new();
        let mut heights = Vec::new();
        sense.supporting.clear();
        for (part, part_body) in bodies.iter().enumerate() {
            let contacts = state.contacts_of(*part_body);
            let grounded = contacts
                .iter()
                .find(|contact| contact.other != Some(entity) && contact.normal.y > GROUND_NORMAL);
            touching |= contacts.iter().any(|contact| contact.other != Some(entity));
            let foot = landmarks.feet().any(|foot| foot == part);
            if !foot {
                continue;
            }
            let shape = rounded_at(&settings[part], pose[part]);
            let lowest = shape.start.y.min(shape.end.y) - shape.radius;
            let ground = match grounded {
                Some(contact) => Some(contact.point.y),
                None if body.is_kinematic(part) => {
                    let under = (shape.start + shape.end) / 2.0;
                    ground_below(
                        &state,
                        under,
                        shape.radius + FOOT_REACH + (under.y - lowest),
                    )
                    .filter(|ground| lowest - ground <= FOOT_REACH)
                }
                None => None,
            };
            let Some(ground) = ground else {
                continue;
            };
            touching = true;
            sense.supporting.push(part);
            heights.push(ground);
            let along = Vec3::new(
                shape.end.x - shape.start.x,
                0.0,
                shape.end.z - shape.start.z,
            );
            let across = along
                .try_normalize()
                .map_or(Vec3::X, |along| Vec3::new(-along.z, 0.0, along.x))
                * shape.radius;
            let ahead = along.try_normalize().unwrap_or(Vec3::Z) * shape.radius;
            for corner in [
                shape.start - ahead + across,
                shape.start - ahead - across,
                shape.end + ahead + across,
                shape.end + ahead - across,
            ] {
                footprint.push(Vec3::new(corner.x, ground, corner.z));
            }
        }
        sense.support = convex_hull(&footprint);
        sense.ground = if heights.is_empty() {
            ground_below(&state, sense.centre.position, GROUND_REACH).unwrap_or(sense.ground)
        } else {
            heights.iter().sum::<f32>() / heights.len() as f32
        };
        sense.capture_point =
            capture_point(sense.centre.position, sense.centre.velocity, sense.ground);
        sense.off_balance = outside_by(&sense.support, sense.capture_point);
        sense.airborne = if touching { 0.0 } else { sense.airborne + dt };

        if let (Some(pelvis), Some(chest)) = (landmarks.pelvis, landmarks.chest) {
            let trunk = pose[chest].translation - pose[pelvis].translation;
            sense.lean = if trunk.length_squared() > 1e-8 {
                trunk.angle_between(Vec3::Y)
            } else {
                (pose[pelvis].rotation * Vec3::Y).angle_between(Vec3::Y)
            };
            sense.trunk_spin = state.angular_velocity(bodies[chest]);
        }
        let falling = sense.supporting.is_empty();
        sense.chest_impact = landmarks
            .chest
            .filter(|_| falling)
            .and_then(|chest| predicted_impact(&state, pose[chest].translation, velocities[chest]));
        sense.head_impact = landmarks
            .head
            .filter(|_| falling)
            .and_then(|head| predicted_impact(&state, pose[head].translation, velocities[head]));

        let slow = velocities
            .iter()
            .all(|velocity| velocity.length() < REST_SPEED);
        sense.still = if slow { sense.still + dt } else { 0.0 };
        sense.at_rest = sense.still >= REST_SECONDS;
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{MassPoint, capture_point, centre_of_mass, convex_hull, outside_by};

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn the_centre_of_mass_is_the_mass_weighted_mean() {
        struct Case {
            name: &'static str,
            points: Vec<MassPoint>,
            centre: MassPoint,
        }
        let at = |mass: f32, x: f32, speed: f32| MassPoint {
            mass,
            position: Vec3::new(x, 1.0, 0.0),
            velocity: Vec3::new(speed, 0.0, 0.0),
        };
        let cases = [
            Case {
                name: "nothing",
                points: vec![],
                centre: MassPoint::default(),
            },
            Case {
                name: "one mass",
                points: vec![at(3.0, 2.0, 1.0)],
                centre: at(3.0, 2.0, 1.0),
            },
            Case {
                name: "the heavier mass pulls the centre three to one",
                points: vec![at(3.0, 0.0, 0.0), at(1.0, 4.0, 4.0)],
                centre: at(4.0, 1.0, 1.0),
            },
            Case {
                name: "a massless point changes nothing",
                points: vec![at(2.0, 1.0, 1.0), at(0.0, 100.0, 100.0)],
                centre: at(2.0, 1.0, 1.0),
            },
        ];
        for case in cases {
            let centre = centre_of_mass(&case.points);
            assert!(
                (centre.mass - case.centre.mass).abs() < 1e-5
                    && close(centre.position, case.centre.position)
                    && close(centre.velocity, case.centre.velocity),
                "{}: {centre:?}",
                case.name
            );
        }
    }

    #[test]
    fn the_capture_point_leads_the_centre_by_its_velocity_times_the_pendulum_time() {
        struct Case {
            name: &'static str,
            centre: Vec3,
            velocity: Vec3,
            ground: f32,
            point: Vec3,
        }
        let cases = [
            Case {
                name: "a still body is captured under its centre",
                centre: Vec3::new(1.0, 1.0, 2.0),
                velocity: Vec3::ZERO,
                ground: 0.0,
                point: Vec3::new(1.0, 0.0, 2.0),
            },
            Case {
                name: "at a height of g metres the lead is the velocity itself",
                centre: Vec3::new(0.0, 9.81, 0.0),
                velocity: Vec3::new(2.0, 0.0, -1.0),
                ground: 0.0,
                point: Vec3::new(2.0, 0.0, -1.0),
            },
            Case {
                name: "height is measured from the ground and vertical speed does not count",
                centre: Vec3::new(0.0, 3.0, 0.0),
                velocity: Vec3::new(1.0, -7.0, 0.0),
                ground: 3.0 - 9.81 / 4.0,
                point: Vec3::new(0.5, 3.0 - 9.81 / 4.0, 0.0),
            },
            Case {
                name: "a centre below the ground has no lead",
                centre: Vec3::new(0.0, -1.0, 0.0),
                velocity: Vec3::new(5.0, 0.0, 0.0),
                ground: 0.0,
                point: Vec3::ZERO,
            },
        ];
        for case in cases {
            let point = capture_point(case.centre, case.velocity, case.ground);
            assert!(close(point, case.point), "{}: {point}", case.name);
        }
    }

    #[test]
    fn a_point_is_judged_against_the_convex_outline_of_the_feet() {
        let at = |x: f32, z: f32| Vec3::new(x, 0.0, z);
        let square = convex_hull(&[
            at(0.0, 0.0),
            at(1.0, 0.0),
            at(1.0, 1.0),
            at(0.0, 1.0),
            at(0.5, 0.5),
            at(0.5, 0.0),
            at(1.0, 1.0),
        ]);
        assert_eq!(
            square.len(),
            4,
            "inner, edge and repeated points drop out: {square:?}"
        );
        struct Case {
            name: &'static str,
            polygon: Vec<Vec3>,
            point: Vec3,
            outside: Vec3,
        }
        let cases = [
            Case {
                name: "inside",
                polygon: square.clone(),
                point: at(0.3, 0.7),
                outside: Vec3::ZERO,
            },
            Case {
                name: "on an edge",
                polygon: square.clone(),
                point: at(1.0, 0.4),
                outside: Vec3::ZERO,
            },
            Case {
                name: "beyond an edge",
                polygon: square.clone(),
                point: at(1.5, 0.5),
                outside: at(0.5, 0.0),
            },
            Case {
                name: "beyond a corner",
                polygon: square.clone(),
                point: at(-1.0, -1.0),
                outside: at(-1.0, -1.0),
            },
            Case {
                name: "a line has no inside",
                polygon: convex_hull(&[at(0.0, 0.0), at(1.0, 0.0)]),
                point: at(0.5, 0.25),
                outside: at(0.0, 0.25),
            },
            Case {
                name: "no support at all",
                polygon: Vec::new(),
                point: at(3.0, 3.0),
                outside: Vec3::ZERO,
            },
        ];
        for case in cases {
            let outside = outside_by(&case.polygon, case.point);
            assert!(close(outside, case.outside), "{}: {outside}", case.name);
        }
    }
}

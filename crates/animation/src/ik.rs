use concerto_mesh::Skeleton;
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};

const MIN_LENGTH_SQUARED: f32 = 1e-12;

const CLEAR_BEND: f32 = std::f32::consts::PI / 36.0;

/// Where a two-bone chain's hinge comes from, given in the mid bone's own frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub enum Hinge {
    /// No hinge: the solve keeps the animation's own bend plane.
    #[default]
    Off,
    /// The plane the chain's rest pose bends in, when it bends clearly.
    Rest,
    /// A fixed axis in the mid bone's frame.
    Axis { axis: Vec3 },
}

impl Hinge {
    /// The normalized axis of an `Axis` hinge, or `None` for `Off`, `Rest` and an axis too short or not finite.
    pub(crate) fn axis(&self) -> Option<Vec3> {
        match self {
            Hinge::Axis { axis } => (axis.is_finite()
                && axis.length_squared() > MIN_LENGTH_SQUARED)
                .then(|| axis.normalize()),
            Hinge::Off | Hinge::Rest => None,
        }
    }

    /// The hinge in the mid bone's own frame, or `None` when this setting gives none.
    ///
    /// # Panics
    ///
    /// For [`Hinge::Rest`], panics if `root`, `mid` or `tip` is not a bone of `skeleton`.
    pub fn local(&self, skeleton: &Skeleton, root: usize, mid: usize, tip: usize) -> Option<Vec3> {
        match self {
            Hinge::Off | Hinge::Axis { .. } => self.axis(),
            Hinge::Rest => {
                let root_position = rest_model(skeleton, root).position;
                let mid_model = rest_model(skeleton, mid);
                let tip_position = rest_model(skeleton, tip).position;
                let upper = mid_model.position - root_position;
                let lower = tip_position - mid_model.position;
                let bend = upper.angle_between(lower);
                if bend.is_nan() || bend < CLEAR_BEND {
                    return None;
                }
                let normal = unit(upper.cross(lower))?;
                Some(mid_model.rotation.inverse() * normal)
            }
        }
    }
}

struct RestModel {
    rotation: Quat,
    position: Vec3,
}

fn rest_model(skeleton: &Skeleton, bone: usize) -> RestModel {
    let mut rotation = Quat::IDENTITY;
    let mut position = Vec3::ZERO;
    let mut at = Some(bone);
    while let Some(index) = at {
        let local = &skeleton.rest_pose[index];
        position = local.translation + local.rotation * (local.scale * position);
        rotation = local.rotation * rotation;
        at = skeleton.parents[index].map(usize::from);
    }
    RestModel { rotation, position }
}

/// The positions of a two-bone chain's three joints, in one space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneChain {
    pub root: Vec3,
    pub mid: Vec3,
    pub tip: Vec3,
    /// The axis the mid joint bends about, in the chain's space, by the right-hand rule from the upper bone to the lower.
    ///
    /// It decides the bend where the chain is too straight to show its own; `None` leaves the
    /// solve as it was.
    pub hinge: Option<Vec3>,
}

/// The rotations that bring a chain's tip to its goal, in the chain's space.
///
/// Each joint's rotation in that space is pre-multiplied by its entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneSolution {
    pub root: Quat,
    pub mid: Quat,
}

impl TwoBoneSolution {
    /// The solution that leaves the chain as it is.
    pub const IDENTITY: TwoBoneSolution = TwoBoneSolution {
        root: Quat::IDENTITY,
        mid: Quat::IDENTITY,
    };
}

/// Solves a two-bone chain toward `target`, bending toward `pole` when given.
///
/// Reach past `chain length - softness` eases toward full length instead of snapping straight.
pub fn solve_two_bone(
    chain: &TwoBoneChain,
    target: Vec3,
    pole: Option<Vec3>,
    softness: f32,
) -> TwoBoneSolution {
    let a = chain.root;
    let upper = chain.mid - a;
    let lower = chain.tip - chain.mid;
    let l1 = upper.length();
    let l2 = lower.length();
    if !(l1.is_finite() && l2.is_finite())
        || l1 * l1 <= MIN_LENGTH_SQUARED
        || l2 * l2 <= MIN_LENGTH_SQUARED
    {
        return TwoBoneSolution::IDENTITY;
    }
    let u1 = upper / l1;
    let u2 = lower / l2;
    let full = l1 + l2;
    let shortest = (l1 - l2).abs();

    let reach = soft_reach((target - a).length(), full, shortest, softness);

    let chain_dir = unit(chain.tip - a).unwrap_or(u1);
    let own = unit(chain_dir.cross(upper));
    let hinge = chain
        .hinge
        .filter(|hinge| hinge.is_finite())
        .and_then(|hinge| unit(hinge - chain_dir * hinge.dot(chain_dir)))
        .map(|hinge| -hinge);
    let normal = match hinge {
        Some(hinge) => own.map_or(hinge, |own| {
            let bend = u1.dot(u2).clamp(-1.0, 1.0).acos();
            let weight = smoothstep(bend / CLEAR_BEND) * own.dot(hinge).clamp(0.0, 1.0);
            unit(own * weight + hinge * (1.0 - weight)).unwrap_or(hinge)
        }),
        None => own
            .or_else(|| pole.and_then(|pole| unit(chain_dir.cross(pole - a))))
            .unwrap_or_else(|| chain_dir.any_orthonormal_vector()),
    };

    let cos_a = if reach * reach <= MIN_LENGTH_SQUARED {
        1.0
    } else {
        (l1 * l1 + reach * reach - l2 * l2) / (2.0 * l1 * reach)
    };
    let angle_a = cos_a.clamp(-1.0, 1.0).acos();
    let upper_after = chain_dir * angle_a.cos() + normal.cross(chain_dir) * angle_a.sin();
    let lower_after = unit(chain_dir * reach - upper_after * l1).unwrap_or(u2);

    let mut root = arc(u1, upper_after, normal);
    let mut mid = arc(u2, lower_after, normal);

    let Some(target_dir) = unit(target - a) else {
        return TwoBoneSolution { root, mid };
    };
    let aim = arc(chain_dir, target_dir, normal);
    root = aim * root;
    mid = aim * mid;

    if let Some(pole) = pole {
        let across = |v: Vec3| unit(v - target_dir * v.dot(target_dir));
        if let (Some(mid_side), Some(pole_side)) = (across(root * upper), across(pole - a)) {
            let angle = target_dir
                .dot(mid_side.cross(pole_side))
                .atan2(mid_side.dot(pole_side));
            let swing = Quat::from_axis_angle(target_dir, angle);
            root = swing * root;
            mid = swing * mid;
        }
    }

    TwoBoneSolution { root, mid }
}

fn soft_reach(distance: f32, full: f32, shortest: f32, softness: f32) -> f32 {
    let softness = if softness.is_nan() {
        0.0
    } else {
        softness.clamp(0.0, full - shortest)
    };
    let start = full - softness;
    let eased = if softness > 0.0 && distance > start {
        start + softness * (1.0 - (-(distance - start) / softness).exp())
    } else {
        distance
    };
    eased.clamp(shortest, full)
}

fn arc(from: Vec3, to: Vec3, half_turn_axis: Vec3) -> Quat {
    let cross = from.cross(to);
    let half = Quat::from_xyzw(cross.x, cross.y, cross.z, 1.0 + from.dot(to));
    if half.length_squared() > MIN_LENGTH_SQUARED {
        half.normalize()
    } else {
        Quat::from_axis_angle(half_turn_axis, std::f32::consts::PI)
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn unit(v: Vec3) -> Option<Vec3> {
    (v.length_squared() > MIN_LENGTH_SQUARED).then(|| v.normalize())
}

#[cfg(test)]
mod tests {
    use concerto_mesh::{JointPose, Skeleton};
    use glam::{Mat4, Quat, Vec3};
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::{Hinge, TwoBoneChain, TwoBoneSolution, solve_two_bone};

    const L1: f32 = 1.0;
    const L2: f32 = 0.8;
    const FULL: f32 = L1 + L2;

    fn bent() -> TwoBoneChain {
        TwoBoneChain {
            root: Vec3::ZERO,
            mid: Vec3::new(0.0, L1, 0.0),
            tip: Vec3::new(L2, L1, 0.0),
            hinge: None,
        }
    }

    struct Solved {
        mid: Vec3,
        tip: Vec3,
    }

    fn apply(chain: &TwoBoneChain, solution: &TwoBoneSolution) -> Solved {
        let mid = chain.root + solution.root * (chain.mid - chain.root);
        let tip = mid + solution.mid * (chain.tip - chain.mid);
        Solved { mid, tip }
    }

    fn tip_distance(target: Vec3, softness: f32) -> f32 {
        let chain = bent();
        apply(&chain, &solve_two_bone(&chain, target, None, softness))
            .tip
            .distance(chain.root)
    }

    fn is_finite(solution: &TwoBoneSolution) -> bool {
        solution.root.is_finite() && solution.mid.is_finite()
    }

    fn any_vec3(range: f32) -> impl Strategy<Value = Vec3> {
        prop::array::uniform3(-range..range).prop_map(Vec3::from_array)
    }

    proptest! {
        #[test]
        fn the_tip_lies_toward_the_target_at_the_reach_the_chain_has(
            target in any_vec3(3.0).prop_filter("off the root", |target| target.length() > 1e-2),
            pole in prop::option::of(any_vec3(2.0)),
            softness in prop_oneof![Just(0.0f32), 0.0f32..10.0],
            hinge in prop::option::of(any_vec3(2.0)),
        ) {
            let chain = TwoBoneChain { hinge, ..bent() };

            let solution = solve_two_bone(&chain, target, pole, softness);

            prop_assert!(is_finite(&solution));
            let solved = apply(&chain, &solution);
            let distance = solved.tip.distance(chain.root);
            prop_assert!((solved.mid.distance(chain.root) - L1).abs() < 1e-4);
            prop_assert!(distance <= FULL + 1e-4, "{distance}");
            prop_assert!(distance >= (L1 - L2).abs() - 1e-4, "{distance}");
            prop_assert!(
                (solved.tip - chain.root).normalize().distance(target.normalize()) < 1e-3,
                "tip {} toward target {target}",
                solved.tip
            );
            let hard = target.length().clamp((L1 - L2).abs(), FULL);
            if softness == 0.0 || target.length() <= FULL - softness.min(FULL - (L1 - L2).abs()) - 1e-3 {
                prop_assert!((distance - hard).abs() < 1e-3, "{distance} != {hard}");
            }
        }
    }

    fn bend_toward(dir: Vec3, degrees: f32) -> TwoBoneChain {
        let axis = Vec3::Y.cross(dir).normalize();
        let half = degrees.to_radians() / 2.0;
        let mid = Quat::from_axis_angle(axis, half) * Vec3::Y * L1;
        TwoBoneChain {
            root: Vec3::ZERO,
            mid,
            tip: mid + Quat::from_axis_angle(axis, -half) * Vec3::Y * L2,
            hinge: None,
        }
    }

    struct HingeCase {
        name: &'static str,
        chain: TwoBoneChain,
    }

    #[test]
    fn a_straight_or_overstraight_chain_bends_about_its_hinge_and_turns_smoothly_into_its_own_bend()
    {
        let goal = Vec3::new(0.0, 0.9 * FULL, 0.0);
        let hinged = |chain: TwoBoneChain| TwoBoneChain {
            hinge: Some(Vec3::Z),
            ..chain
        };
        let solve = |chain: &TwoBoneChain| apply(chain, &solve_two_bone(chain, goal, None, 0.0));
        let straight = solve(&hinged(bend_toward(Vec3::X, 0.0)));
        assert!(straight.mid.z.abs() < 1e-4, "mid {}", straight.mid);
        let cases = [
            HingeCase {
                name: "straight",
                chain: bend_toward(Vec3::X, 0.0),
            },
            HingeCase {
                name: "bent backward",
                chain: bend_toward(Vec3::NEG_X, 2.0),
            },
            HingeCase {
                name: "bent sideways",
                chain: bend_toward(Vec3::Z, 2.0),
            },
            HingeCase {
                name: "a hair bent, half sideways",
                chain: bend_toward(Vec3::new(1.0, 0.0, 1.0).normalize(), 0.02),
            },
        ];

        for case in cases {
            let solved = solve(&hinged(case.chain));

            assert!(
                solved.tip.distance(goal) < 1e-4,
                "{}: tip {}",
                case.name,
                solved.tip
            );
            assert!(solved.mid.x > 0.0, "{}: mid {}", case.name, solved.mid);
            assert!(
                solved.mid.distance(straight.mid) < 1e-3,
                "{}: mid {} beside {}",
                case.name,
                solved.mid,
                straight.mid
            );
        }

        let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let skeleton = |knee_degrees: f32| {
            let pose = |translation: Vec3, rotation: Quat| JointPose {
                translation,
                rotation,
                scale: Vec3::ONE,
            };
            Skeleton::new(
                vec![Mat4::IDENTITY; 3],
                vec![None, Some(0), Some(1)],
                vec!["root".into(), "mid".into(), "tip".into()],
                vec![
                    pose(Vec3::ZERO, turned),
                    pose(
                        Vec3::Y * L1,
                        Quat::from_rotation_x(knee_degrees.to_radians()),
                    ),
                    pose(Vec3::Y * L2, Quat::IDENTITY),
                ],
                vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            )
            .unwrap()
        };
        let straight_rest = skeleton(0.0);
        let bent_rest = skeleton(20.0);
        let mid_rotation = turned * Quat::from_rotation_x(20f32.to_radians());
        let mid_position = turned * Vec3::Y * L1;
        let tip_position = mid_position + mid_rotation * Vec3::Y * L2;
        let model_hinge = mid_position.cross(tip_position - mid_position).normalize();
        let local = |hinge: Hinge, skeleton: &Skeleton| hinge.local(skeleton, 0, 1, 2);
        assert_eq!(local(Hinge::Off, &bent_rest), None);
        assert_eq!(local(Hinge::Axis { axis: Vec3::ZERO }, &bent_rest), None);
        assert_eq!(
            local(
                Hinge::Axis {
                    axis: Vec3::new(0.0, 0.0, 2.0)
                },
                &bent_rest
            ),
            Some(Vec3::Z)
        );
        assert_eq!(local(Hinge::Rest, &straight_rest), None);
        let rest = local(Hinge::Rest, &bent_rest).expect("a bent rest chain has a hinge");
        assert!(
            (mid_rotation * rest).distance(model_hinge) < 1e-4,
            "{} beside {model_hinge}",
            mid_rotation * rest
        );

        let mut previous = None;
        for step in 0..=260 {
            let degrees = -3.0 + 0.05 * step as f32;
            let mid = solve(&hinged(bend_toward(Vec3::X, degrees))).mid;
            if let Some(previous) = previous {
                assert!(
                    mid.distance(previous) < 0.002,
                    "{degrees}: mid {mid} after {previous}"
                );
            }
            previous = Some(mid);
        }
        let clear = bend_toward(Vec3::X, 10.0);
        let hingeless = solve(&clear).mid;
        let hinged_mid = solve(&hinged(clear)).mid;
        assert!(
            hinged_mid.distance(hingeless) < 1e-4,
            "{hinged_mid} != {hingeless}"
        );
    }

    #[test]
    fn the_reach_grows_continuously_with_the_target_distance_and_softness_keeps_it_short_of_full_length()
     {
        for softness in [0.0, 0.3] {
            let direction = Vec3::new(0.4, 1.0, 0.2).normalize();
            let step = 0.005;
            let mut previous = tip_distance(direction * 0.5, softness);
            for i in 1..=400 {
                let target_distance = 0.5 + step * i as f32;
                let distance = tip_distance(direction * target_distance, softness);
                assert!(
                    distance >= previous - 1e-5,
                    "softness {softness}: {distance} after {previous}"
                );
                assert!(
                    distance - previous <= step + 1e-5,
                    "softness {softness}: {distance} after {previous}"
                );
                if softness > 0.0 {
                    assert!(distance < FULL, "{distance} at {target_distance}");
                    if target_distance > FULL {
                        assert!(distance > previous, "{distance} after {previous}");
                    }
                }
                previous = distance;
            }
        }
    }

    #[test]
    fn the_mid_bends_toward_the_pole_or_without_one_stays_on_its_side_of_the_input_plane() {
        let chain = bent();
        let target = Vec3::new(1.1, 0.2, 0.0);

        let unpoled = apply(&chain, &solve_two_bone(&chain, target, None, 0.0));

        assert!(unpoled.tip.distance(target) < 1e-4);
        assert!(unpoled.mid.z.abs() < 1e-4, "mid {}", unpoled.mid);
        let side = (chain.tip - chain.root).cross(chain.mid - chain.root);
        assert!(
            (target - chain.root)
                .cross(unpoled.mid - chain.root)
                .dot(side)
                > 0.0
        );

        let pole = Vec3::new(0.5, -1.0, -0.7);
        let poled = apply(&chain, &solve_two_bone(&chain, target, Some(pole), 0.0));

        assert!(poled.tip.distance(target) < 1e-4);
        let normal = target.cross(pole).normalize();
        assert!(poled.mid.dot(normal).abs() < 1e-4, "mid {}", poled.mid);
        let axis = target.normalize();
        let mid_across = poled.mid - axis * poled.mid.dot(axis);
        let pole_across = pole - axis * pole.dot(axis);
        assert!(mid_across.dot(pole_across) > 0.0, "mid {}", poled.mid);
    }

    enum Outcome {
        Reaches(Vec3),
        Finite,
        Identity,
    }

    struct Degenerate {
        name: &'static str,
        chain: TwoBoneChain,
        target: Vec3,
        pole: Option<Vec3>,
        softness: f32,
        outcome: Outcome,
    }

    #[test]
    fn a_degenerate_chain_or_target_gives_finite_rotations_and_a_broken_chain_none() {
        let target = Vec3::new(1.0, 0.5, 0.0);
        let hair_off_line =
            Quat::from_rotation_z(3e-4) * (bent().tip - bent().root).normalize() * 1.2;
        let mut nan_mid = bent();
        nan_mid.mid.x = f32::NAN;
        let case = |name, chain, target, pole, outcome| Degenerate {
            name,
            chain,
            target,
            pole,
            softness: 0.0,
            outcome,
        };
        let cases = [
            case(
                "a straight chain",
                TwoBoneChain {
                    root: Vec3::ZERO,
                    mid: Vec3::new(0.0, L1, 0.0),
                    tip: Vec3::new(0.0, FULL, 0.0),
                    hinge: None,
                },
                Vec3::new(0.5, 1.0, 0.0),
                None,
                Outcome::Reaches(Vec3::new(0.5, 1.0, 0.0)),
            ),
            case(
                "a target at the root",
                bent(),
                Vec3::ZERO,
                None,
                Outcome::Finite,
            ),
            case(
                "a pole on the line to the target",
                bent(),
                target,
                Some(target * 0.5),
                Outcome::Reaches(target),
            ),
            case(
                "a target a hair off the chain's line",
                bent(),
                hair_off_line,
                None,
                Outcome::Reaches(hair_off_line),
            ),
            Degenerate {
                softness: 0.2,
                ..case(
                    "a joint that is NaN",
                    nan_mid,
                    Vec3::X,
                    None,
                    Outcome::Identity,
                )
            },
        ];

        for case in cases {
            let solution = solve_two_bone(&case.chain, case.target, case.pole, case.softness);

            match case.outcome {
                Outcome::Reaches(goal) => {
                    assert!(is_finite(&solution), "{}", case.name);
                    let tip = apply(&case.chain, &solution).tip;
                    assert!(tip.distance(goal) < 1e-5, "{}: tip {tip}", case.name);
                }
                Outcome::Finite => assert!(is_finite(&solution), "{}", case.name),
                Outcome::Identity => {
                    assert_eq!(solution, TwoBoneSolution::IDENTITY, "{}", case.name)
                }
            }
        }
    }
}

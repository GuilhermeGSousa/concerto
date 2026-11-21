use glam::{Quat, Vec3};

const MIN_LENGTH_SQUARED: f32 = 1e-12;

/// The positions of a two-bone chain's three joints, in one space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneChain {
    pub root: Vec3,
    pub mid: Vec3,
    pub tip: Vec3,
}

/// The rotations that bring a chain's tip to its goal, in the chain's space: each joint's rotation in that space is pre-multiplied by its entry.
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

/// Solves a two-bone chain toward `target`, bending toward `pole` when given; reach past `chain length - softness` eases toward full length.
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
    let normal = unit(chain_dir.cross(upper))
        .or_else(|| pole.and_then(|pole| unit(chain_dir.cross(pole - a))))
        .unwrap_or_else(|| chain_dir.any_orthonormal_vector());

    let angle_a = chain_dir.dot(u1).clamp(-1.0, 1.0).acos();
    let angle_b = (-u1).dot(u2).clamp(-1.0, 1.0).acos();
    let cos_a = if reach * reach <= MIN_LENGTH_SQUARED {
        1.0
    } else {
        (l1 * l1 + reach * reach - l2 * l2) / (2.0 * l1 * reach)
    };
    let cos_b = (l1 * l1 + l2 * l2 - reach * reach) / (2.0 * l1 * l2);
    let turn_a = cos_a.clamp(-1.0, 1.0).acos() - angle_a;
    let turn_b = cos_b.clamp(-1.0, 1.0).acos() - angle_b;

    let mut root = Quat::from_axis_angle(normal, turn_a);
    let mut mid = Quat::from_axis_angle(normal, turn_a + turn_b);

    let Some(target_dir) = unit(target - a) else {
        return TwoBoneSolution { root, mid };
    };
    let aim = Quat::from_rotation_arc(chain_dir, target_dir);
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

fn unit(v: Vec3) -> Option<Vec3> {
    (v.length_squared() > MIN_LENGTH_SQUARED).then(|| v.normalize())
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use proptest::prelude::*;

    use super::{TwoBoneChain, TwoBoneSolution, solve_two_bone};

    const L1: f32 = 1.0;
    const L2: f32 = 0.8;
    const FULL: f32 = L1 + L2;

    fn bent() -> TwoBoneChain {
        TwoBoneChain {
            root: Vec3::ZERO,
            mid: Vec3::new(0.0, L1, 0.0),
            tip: Vec3::new(L2, L1, 0.0),
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
        ) {
            let chain = bent();

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
                    assert!(tip.distance(goal) < 1e-4, "{}: tip {tip}", case.name);
                }
                Outcome::Finite => assert!(is_finite(&solution), "{}", case.name),
                Outcome::Identity => {
                    assert_eq!(solution, TwoBoneSolution::IDENTITY, "{}", case.name)
                }
            }
        }
    }
}

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
    use glam::{Quat, Vec3};

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

    fn apply(chain: &TwoBoneChain, solution: &TwoBoneSolution) -> (Vec3, Vec3) {
        let mid = chain.root + solution.root * (chain.mid - chain.root);
        let tip = mid + solution.mid * (chain.tip - chain.mid);
        (mid, tip)
    }

    fn tip_distance(target: Vec3, softness: f32) -> f32 {
        let chain = bent();
        let (_, tip) = apply(&chain, &solve_two_bone(&chain, target, None, softness));
        tip.distance(chain.root)
    }

    fn is_finite(solution: &TwoBoneSolution) -> bool {
        solution.root.is_finite() && solution.mid.is_finite()
    }

    #[test]
    fn a_target_within_reach_is_reached_without_changing_bone_lengths() {
        let chain = bent();
        let target = Vec3::new(1.2, 0.5, 0.3);

        let (mid, tip) = apply(&chain, &solve_two_bone(&chain, target, None, 0.0));

        assert!(tip.distance(target) < 1e-4, "tip {tip}, target {target}");
        assert!((mid.distance(chain.root) - L1).abs() < 1e-4);
        assert!((tip.distance(mid) - L2).abs() < 1e-4);
    }

    #[test]
    fn a_target_past_full_length_straightens_the_chain_toward_it() {
        let chain = bent();
        let target = Vec3::new(3.0, 1.0, 0.5);

        let (_, tip) = apply(&chain, &solve_two_bone(&chain, target, None, 0.0));

        let expected = chain.root + target.normalize() * FULL;
        assert!(
            tip.distance(expected) < 1e-4,
            "tip {tip}, expected {expected}"
        );
    }

    #[test]
    fn reach_does_not_pop_at_full_extension() {
        let direction = Vec3::new(1.0, 0.3, -0.2).normalize();
        let step = 0.005;
        let mut previous = tip_distance(direction * 1.5, 0.0);
        for i in 1..=120 {
            let distance = tip_distance(direction * (1.5 + step * i as f32), 0.0);
            assert!(distance >= previous - 1e-5, "{distance} after {previous}");
            assert!(
                distance - previous <= step + 1e-5,
                "{distance} after {previous}"
            );
            previous = distance;
        }
    }

    #[test]
    fn softness_eases_reach_toward_full_length() {
        let softness = 0.3;
        let direction = Vec3::new(0.4, 1.0, 0.2).normalize();
        let step = 0.01;
        let mut previous = tip_distance(direction * 0.5, softness);
        for i in 1..=200 {
            let target_distance = 0.5 + step * i as f32;
            let distance = tip_distance(direction * target_distance, softness);
            assert!(distance < FULL, "{distance} at {target_distance}");
            if target_distance <= FULL - softness - 1e-3 {
                let hard = tip_distance(direction * target_distance, 0.0);
                assert!((distance - hard).abs() < 1e-4, "{distance} vs {hard}");
            }
            if target_distance > FULL {
                assert!(distance > previous, "{distance} after {previous}");
            }
            assert!(
                (distance - previous).abs() <= step + 1e-5,
                "{distance} after {previous}"
            );
            previous = distance;
        }
    }

    #[test]
    fn softness_past_full_length_stays_within_the_chain_range() {
        let chain = bent();
        for target_distance in [0.0, 0.1, 0.5, 1.0, 1.8, 3.0] {
            let target = Vec3::new(0.6, 0.8, 0.0) * target_distance;
            let solution = solve_two_bone(&chain, target, None, 10.0);
            assert!(is_finite(&solution));
            let (_, tip) = apply(&chain, &solution);
            let distance = tip.distance(chain.root);
            assert!(distance <= FULL + 1e-4, "{distance} at {target_distance}");
            assert!(
                distance >= (L1 - L2).abs() - 1e-4,
                "{distance} at {target_distance}"
            );
        }
    }

    #[test]
    fn a_target_inside_the_shortest_reach_puts_the_tip_at_that_reach() {
        let chain = bent();
        let target = Vec3::new(0.05, 0.08, 0.02);

        let (_, tip) = apply(&chain, &solve_two_bone(&chain, target, None, 0.0));

        let expected = chain.root + target.normalize() * (L1 - L2).abs();
        assert!(
            tip.distance(expected) < 1e-3,
            "tip {tip}, expected {expected}"
        );
    }

    #[test]
    fn without_a_pole_the_mid_stays_on_its_side_of_the_input_plane() {
        let chain = bent();
        let target = Vec3::new(1.1, 0.2, 0.0);

        let (mid, tip) = apply(&chain, &solve_two_bone(&chain, target, None, 0.0));

        assert!(tip.distance(target) < 1e-4);
        assert!(mid.z.abs() < 1e-4, "mid {mid}");
        let side = (chain.tip - chain.root).cross(chain.mid - chain.root);
        assert!((target - chain.root).cross(mid - chain.root).dot(side) > 0.0);
    }

    #[test]
    fn a_pole_on_the_other_side_bends_the_mid_toward_it() {
        let chain = bent();
        let target = Vec3::new(1.1, 0.2, 0.0);
        let pole = Vec3::new(0.5, -1.0, -0.7);

        let (mid, tip) = apply(&chain, &solve_two_bone(&chain, target, Some(pole), 0.0));

        assert!(tip.distance(target) < 1e-4);
        let normal = target.cross(pole).normalize();
        assert!(mid.dot(normal).abs() < 1e-4, "mid {mid}");
        let axis = target.normalize();
        let mid_across = mid - axis * mid.dot(axis);
        let pole_across = pole - axis * pole.dot(axis);
        assert!(mid_across.dot(pole_across) > 0.0, "mid {mid}");
    }

    #[test]
    fn a_fully_folded_chain_unfolds_to_reach_its_target() {
        let folded = TwoBoneChain {
            root: Vec3::ZERO,
            mid: Vec3::new(0.0, L1, 0.0),
            tip: Vec3::ZERO,
        };
        let target = Vec3::new(0.5, 0.5, 0.2);

        let solution = solve_two_bone(&folded, target, None, 0.0);

        assert!(is_finite(&solution));
        let (mid, tip) = apply(&folded, &solution);
        assert!(tip.distance(target) < 1e-4, "tip {tip}");
        assert!((mid.length() - L1).abs() < 1e-4, "mid {mid}");
    }

    #[test]
    fn a_chain_with_a_joint_that_is_not_finite_is_left_as_it_is() {
        let mut broken = bent();
        broken.mid.x = f32::NAN;
        assert_eq!(
            solve_two_bone(&broken, Vec3::X, None, 0.2),
            TwoBoneSolution::IDENTITY
        );

        let mut far = bent();
        far.tip = Vec3::splat(f32::INFINITY);
        assert_eq!(
            solve_two_bone(&far, Vec3::X, None, 0.2),
            TwoBoneSolution::IDENTITY
        );
    }

    #[test]
    fn degenerate_inputs_give_finite_rotations() {
        let straight = TwoBoneChain {
            root: Vec3::ZERO,
            mid: Vec3::new(0.0, L1, 0.0),
            tip: Vec3::new(0.0, FULL, 0.0),
        };
        let target = Vec3::new(0.5, 1.0, 0.0);
        let solution = solve_two_bone(&straight, target, None, 0.0);
        assert!(is_finite(&solution));
        assert!(apply(&straight, &solution).1.distance(target) < 1e-4);

        let chain = bent();
        let pole = Vec3::new(0.0, 0.0, 1.0);
        assert!(is_finite(&solve_two_bone(&chain, Vec3::ZERO, None, 0.0)));
        assert!(is_finite(&solve_two_bone(
            &chain,
            Vec3::ZERO,
            Some(pole),
            0.0
        )));

        let target = Vec3::new(1.0, 0.5, 0.0);
        let solution = solve_two_bone(&chain, target, Some(target * 0.5), 0.0);
        assert!(is_finite(&solution));
        assert!(apply(&chain, &solution).1.distance(target) < 1e-4);

        let zero_length = TwoBoneChain {
            root: Vec3::ZERO,
            mid: Vec3::ZERO,
            tip: Vec3::new(L2, 0.0, 0.0),
        };
        let solution = solve_two_bone(&zero_length, target, Some(pole), 0.3);
        assert_eq!(solution.root, Quat::IDENTITY);
        assert_eq!(solution.mid, Quat::IDENTITY);
    }
}

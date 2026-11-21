use concerto_name::Name;
use concerto_ragdoll::{BALANCE, Balance, BodyState, Outcome, STAGGER, Stagger};
use glam::Vec3;

use crate::balance::Stage;

#[derive(Debug)]
struct Stumbled {
    balance: Option<Outcome>,
    stagger: Option<Outcome>,
    steps: u32,
    reach: f32,
    state: BodyState,
}

fn stumbled(stagger: Option<Stagger>, impulse: Vec3) -> Stumbled {
    let mut stage = Stage::new();
    stage.insert(Balance::default());
    stage.run(15);
    stage.push("spine_03", impulse);
    if let Some(stagger) = stagger {
        stage.insert(stagger);
    }
    let mut steps = 0;
    let mut stepping = false;
    let mut reach: f32 = 0.0;
    let held = ["hand_l", "hand_r"].map(|hand| stage.part(hand) - stage.part("spine_03"));
    for _ in 0..300 {
        if stage
            .ended()
            .iter()
            .any(|ended| ended.behaviour == Name::from(BALANCE))
        {
            break;
        }
        let chest = stage.part("spine_03");
        for (hand, held) in ["hand_l", "hand_r"].into_iter().zip(held) {
            reach = reach.max((stage.part(hand) - chest - held).length());
        }
        if let Some(balance) = stage.balance() {
            if balance.stepping() && !stepping {
                steps += 1;
            }
            stepping = balance.stepping();
        }
        stage.run(1);
    }
    stage.run(1);
    let outcome = |name: &str| {
        stage
            .ended()
            .iter()
            .find(|ended| ended.behaviour == Name::from(name))
            .map(|ended| ended.outcome)
    };
    Stumbled {
        balance: outcome(BALANCE),
        stagger: outcome(STAGGER),
        steps,
        reach,
        state: stage.state(),
    }
}

#[test]
fn a_stagger_stumbles_further_than_balance_with_arms_out_and_a_severe_one_falls() {
    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let push = direction * 70.0;
        let alone = stumbled(None, push);
        assert_eq!(alone.balance, Some(Outcome::Succeeded), "{alone:?}");

        let mild = stumbled(Some(Stagger::new(direction, 0.4)), push);
        assert_eq!(
            (mild.balance, mild.stagger, mild.state),
            (
                Some(Outcome::Succeeded),
                Some(Outcome::Succeeded),
                BodyState::Animated
            ),
            "a mild stagger towards {direction} ends standing, with balance's outcome: {mild:?}"
        );
        assert!(
            mild.steps > alone.steps,
            "a mild stagger towards {direction} takes more steps than balance alone: {mild:?} against {alone:?}"
        );
        assert!(
            mild.reach > alone.reach + 0.1,
            "the hands travel further from where the chest held them in a stagger towards {direction}: {mild:?} against {alone:?}"
        );

        let severe = stumbled(Some(Stagger::new(direction, 1.0)), push);
        assert_eq!(
            (severe.balance, severe.stagger, severe.state),
            (
                Some(Outcome::GaveUp),
                Some(Outcome::GaveUp),
                BodyState::Falling
            ),
            "a severe stagger towards {direction} cannot be recovered: {severe:?}"
        );
    }
}

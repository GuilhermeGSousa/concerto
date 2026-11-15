use crate::book::Casebook;
use crate::case::{
    Ask, Case, Condition, Dialogue, DoorId, DoorState, GameTime, KeyId, NpcId, Phase, PlacementId,
    Role, TimeWindow, Zone,
};
use crate::casefile::CaseData;

/// Whether `condition` holds at `now` for an NPC in `phase`, given what the players know.
pub fn holds(condition: &Condition, now: GameTime, phase: Phase, book: &Casebook) -> bool {
    condition.window.is_none_or(|window| window.contains(now))
        && condition.phase.is_none_or(|wanted| wanted == phase)
        && book.knows_all(&condition.knows)
}

/// Something a player can ask an NPC now, and the answer they would get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub topic: usize,
    pub response: usize,
}

/// The topics of `dialogue` on offer: fixed questions first, then facts the players know and people, each with the first response whose condition holds.
pub fn offers(
    dialogue: &Dialogue,
    now: GameTime,
    phase: Phase,
    book: &Casebook,
    carried: Option<PlacementId>,
) -> Vec<Offer> {
    let offer = |topic: usize| {
        dialogue.topics[topic]
            .responses
            .iter()
            .position(|response| holds(&response.when, now, phase, book))
            .map(|response| Offer { topic, response })
    };
    let questions = dialogue
        .topics
        .iter()
        .enumerate()
        .filter(|(_, topic)| matches!(topic.ask, Ask::Question(_)));
    let about = dialogue
        .topics
        .iter()
        .enumerate()
        .filter(|(_, topic)| match topic.ask {
            Ask::Question(_) => false,
            Ask::Fact(fact) => book.knows(fact),
            Ask::Person(_) => true,
            Ask::Item(item) => carried == Some(item),
        });
    questions
        .chain(about)
        .filter_map(|(index, _)| offer(index))
        .collect()
}

/// What a topic is called in the menu.
pub fn ask_label(case: &Case, ask: Ask) -> String {
    match ask {
        Ask::Question(text) => case.strings.get(text).as_str().to_string(),
        Ask::Fact(fact) => format!(
            "Ask about: {}",
            case.strings.get(case.facts.get(fact).text).as_str()
        ),
        Ask::Person(npc) => format!("Ask about {}", name_of(case, npc)),
        Ask::Item(item) => format!(
            "Show the {}",
            case.strings
                .get(case.placements.get(item).name)
                .as_str()
                .to_lowercase()
        ),
    }
}

/// A character's name.
pub fn name_of(case: &Case, npc: NpcId) -> &str {
    case.strings.get(case.characters.get(npc).name).as_str()
}

/// A door as this case has it: the level's door with the case's override.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorRule {
    pub enabled: bool,
    pub zone: Zone,
    /// The state each night starts with.
    pub state: DoorState,
    pub key: Option<KeyId>,
    pub locked: Vec<TimeWindow>,
    /// The Staff's key ring opens it.
    pub staff_opens: bool,
}

/// The rule for `door` in this case.
pub fn door_rule(data: &CaseData, door: DoorId) -> DoorRule {
    let level = data.level.door(door);
    let over = data.case.doors.override_for(door);
    DoorRule {
        enabled: over.and_then(|over| over.enabled).unwrap_or(true),
        zone: over.and_then(|over| over.zone).unwrap_or(level.zone),
        state: over.and_then(|over| over.state).unwrap_or(level.state),
        key: over.and_then(|over| over.key).or_else(|| {
            level
                .key
                .as_deref()
                .and_then(|key| data.catalog.keys.id(key))
        }),
        locked: over.map(|over| over.locked.clone()).unwrap_or_default(),
        staff_opens: data.case.doors.staff_opens.contains(&door),
    }
}

/// What a private door finds when a player tries it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Privacy {
    /// The player stands outside a private room the door leads into.
    pub entering: bool,
    /// That room is the player's own.
    pub own: bool,
    /// A character is in it.
    pub someone_in: bool,
    /// Its bell flag is up on the board.
    pub rung: bool,
}

/// Why a door stays shut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A service door, and the Guest is trying it.
    ServiceOnly,
    /// It is locked and the player has neither its key nor the key ring for it.
    Locked,
    /// Someone's room with nobody in it, and the Guest is trying it without its key.
    NobodyIn,
    /// Someone is in their room, and the Staff is trying it without being rung for.
    Occupied,
}

impl Refusal {
    /// The line the player reads.
    pub fn text(self) -> &'static str {
        match self {
            Refusal::ServiceOnly => "Staff only. A guest has no business back there.",
            Refusal::Locked => "Locked.",
            Refusal::NobodyIn => "Nobody answers. It is not your room to walk into.",
            Refusal::Occupied => "Someone is in. The staff wait to be rung for.",
        }
    }
}

/// Whether a player in `role`, carrying `carried`, may work a door whose lock is `locked` at `now`. A private room lets the Guest in while someone is there to answer, and the Staff while nobody is, or when it rang.
pub fn may_pass(
    rule: &DoorRule,
    locked: bool,
    role: Role,
    carried: Option<KeyId>,
    now: GameTime,
    at_the_door: Privacy,
) -> Result<(), Refusal> {
    if rule.zone == Zone::Service && role == Role::Guest {
        return Err(Refusal::ServiceOnly);
    }
    let has_key = rule.key.is_some() && rule.key == carried;
    if rule.zone == Zone::Private && at_the_door.entering && !at_the_door.own {
        match role {
            Role::Guest if !at_the_door.someone_in && !has_key => return Err(Refusal::NobodyIn),
            Role::Staff if at_the_door.someone_in && !at_the_door.rung => {
                return Err(Refusal::Occupied);
            }
            _ => {}
        }
    }
    let ring = role == Role::Staff && rule.staff_opens;
    let barred = locked || rule.locked.iter().any(|window| window.contains(now));
    if barred && !has_key && !ring {
        return Err(Refusal::Locked);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DoorRule, Offer, Privacy, Refusal, door_rule, may_pass, offers};
    use crate::book::Casebook;
    use crate::case::{Ask, DoorState, GameTime, Phase, Role, Zone};
    use crate::casefile::CaseData;

    fn at(hour: u16, minute: u16) -> GameTime {
        GameTime::from_hm(hour, minute).unwrap()
    }

    #[test]
    fn topics_follow_the_hour_and_what_the_players_know() {
        let data = CaseData::test();
        let case = &data.case;
        let npc = |name: &str| case.characters.id(name).unwrap();
        let fact = |name: &str| case.facts.id(name).unwrap();
        let mut book = Casebook::default();

        let corwin = case.dialogue(npc("npc_02")).unwrap();
        assert_eq!(
            offers(corwin, at(18, 0), Phase::Before, &book, None),
            [
                Offer {
                    topic: 0,
                    response: 1
                },
                Offer {
                    topic: 1,
                    response: 1
                }
            ]
        );
        book.grant(fact("f_03"));
        assert_eq!(
            offers(corwin, at(21, 0), Phase::Before, &book, None),
            [
                Offer {
                    topic: 0,
                    response: 0
                },
                Offer {
                    topic: 1,
                    response: 0
                }
            ],
            "a later hour and a known fact pick the earlier responses"
        );

        let ashby = case.dialogue(npc("npc_01")).unwrap();
        let before: Vec<usize> =
            offers(ashby, at(18, 0), Phase::Before, &Casebook::default(), None)
                .iter()
                .map(|offer| offer.topic)
                .collect();
        assert_eq!(before, [0, 2], "a fact nobody found is not on offer");
        book.grant(fact("f_01"));
        let after: Vec<usize> = offers(ashby, at(18, 0), Phase::Before, &book, None)
            .iter()
            .map(|offer| offer.topic)
            .collect();
        assert_eq!(after, [0, 1, 2]);

        let cufflink = case.placements.id("p_cufflink").unwrap();
        let shown = offers(ashby, at(18, 0), Phase::Before, &book, Some(cufflink));
        let item = shown.last().unwrap();
        assert_eq!(
            ashby.topics[item.topic].ask,
            Ask::Item(cufflink),
            "an item topic is on offer only while the player carries the item"
        );
        assert_eq!(shown.len(), after.len() + 1);
    }

    #[test]
    fn doors_follow_zones_keys_the_key_ring_and_time_windows() {
        let data = CaseData::test();
        let door = |name: &str| data.catalog.doors.id(name).unwrap();
        let key = data.catalog.keys.id("k_service");
        let service = door_rule(&data, door("d_service"));
        let public = door_rule(&data, door("d_ab"));

        assert_eq!(service.state, DoorState::Locked);
        assert!(service.staff_opens);
        assert_eq!(service.key, key);
        let locked = service.state == DoorState::Locked;
        assert_eq!(
            may_pass(
                &service,
                locked,
                Role::Guest,
                key,
                at(18, 0),
                Privacy::default()
            ),
            Err(Refusal::ServiceOnly),
            "a service door refuses the Guest even with its key"
        );
        assert_eq!(
            may_pass(
                &service,
                locked,
                Role::Staff,
                None,
                at(18, 0),
                Privacy::default()
            ),
            Ok(())
        );

        assert_eq!(
            may_pass(
                &public,
                false,
                Role::Guest,
                None,
                at(23, 29),
                Privacy::default()
            ),
            Ok(())
        );
        assert_eq!(
            may_pass(
                &public,
                false,
                Role::Guest,
                None,
                at(23, 30),
                Privacy::default()
            ),
            Err(Refusal::Locked),
            "the case locks it for the last half hour"
        );
        assert_eq!(
            may_pass(
                &public,
                true,
                Role::Staff,
                None,
                at(18, 0),
                Privacy::default()
            ),
            Err(Refusal::Locked),
            "the key ring opens only what the case lists"
        );

        struct Knock {
            role: Role,
            at_the_door: Privacy,
            answer: Result<(), Refusal>,
            why: &'static str,
        }
        let private = DoorRule {
            zone: Zone::Private,
            ..public.clone()
        };
        let outside = Privacy {
            entering: true,
            ..Privacy::default()
        };
        let knocks = [
            Knock {
                role: Role::Guest,
                at_the_door: outside,
                answer: Err(Refusal::NobodyIn),
                why: "a guest does not walk into an empty room that is not theirs",
            },
            Knock {
                role: Role::Guest,
                at_the_door: Privacy {
                    someone_in: true,
                    ..outside
                },
                answer: Ok(()),
                why: "whoever is in lets a guest in",
            },
            Knock {
                role: Role::Guest,
                at_the_door: Privacy {
                    own: true,
                    ..outside
                },
                answer: Ok(()),
                why: "a guest's own room is theirs to enter",
            },
            Knock {
                role: Role::Guest,
                at_the_door: Privacy::default(),
                answer: Ok(()),
                why: "nobody is kept from leaving",
            },
            Knock {
                role: Role::Staff,
                at_the_door: outside,
                answer: Ok(()),
                why: "the staff do an empty room",
            },
            Knock {
                role: Role::Staff,
                at_the_door: Privacy {
                    someone_in: true,
                    ..outside
                },
                answer: Err(Refusal::Occupied),
                why: "the staff do not walk in on anyone",
            },
            Knock {
                role: Role::Staff,
                at_the_door: Privacy {
                    someone_in: true,
                    rung: true,
                    ..outside
                },
                answer: Ok(()),
                why: "unless the room rang for them",
            },
        ];
        for knock in knocks {
            assert_eq!(
                may_pass(
                    &private,
                    false,
                    knock.role,
                    None,
                    at(18, 0),
                    knock.at_the_door
                ),
                knock.answer,
                "{}",
                knock.why
            );
        }
        assert_eq!(
            may_pass(&private, false, Role::Guest, key, at(18, 0), outside),
            Err(Refusal::NobodyIn),
            "a key to another door does not help"
        );
        let keyed = DoorRule {
            key,
            ..private.clone()
        };
        assert_eq!(
            may_pass(&keyed, true, Role::Guest, key, at(18, 0), outside),
            Ok(()),
            "a guest holding a room's key lets themselves in"
        );
    }
}

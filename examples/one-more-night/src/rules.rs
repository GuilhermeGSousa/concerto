use crate::book::Casebook;
use crate::case::{
    Ask, Case, Condition, Dialogue, DoorId, DoorState, GameTime, KeyId, NpcId, Phase, Role,
    TimeWindow, Zone,
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
pub fn offers(dialogue: &Dialogue, now: GameTime, phase: Phase, book: &Casebook) -> Vec<Offer> {
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

/// Why a door stays shut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A service door, and the Guest is trying it.
    ServiceOnly,
    /// It is locked and the player has neither its key nor the key ring for it.
    Locked,
}

impl Refusal {
    /// The line the player reads.
    pub fn text(self) -> &'static str {
        match self {
            Refusal::ServiceOnly => "Staff only. A guest has no business back there.",
            Refusal::Locked => "Locked.",
        }
    }
}

/// Whether a player in `role`, carrying `carried`, may work a door whose lock is `locked` at `now`.
pub fn may_pass(
    rule: &DoorRule,
    locked: bool,
    role: Role,
    carried: Option<KeyId>,
    now: GameTime,
) -> Result<(), Refusal> {
    if rule.zone == Zone::Service && role == Role::Guest {
        return Err(Refusal::ServiceOnly);
    }
    let has_key = rule.key.is_some() && rule.key == carried;
    let ring = role == Role::Staff && rule.staff_opens;
    let barred = locked || rule.locked.iter().any(|window| window.contains(now));
    if barred && !has_key && !ring {
        return Err(Refusal::Locked);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Offer, Refusal, ask_label, door_rule, may_pass, offers};
    use crate::book::Casebook;
    use crate::case::{DoorState, GameTime, Phase, Role};
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
            offers(corwin, at(18, 0), Phase::Before, &book),
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
            offers(corwin, at(21, 0), Phase::Before, &book),
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
        let before: Vec<usize> = offers(ashby, at(18, 0), Phase::Before, &Casebook::default())
            .iter()
            .map(|offer| offer.topic)
            .collect();
        assert_eq!(before, [0, 2], "a fact nobody found is not on offer");
        book.grant(fact("f_01"));
        let after: Vec<usize> = offers(ashby, at(18, 0), Phase::Before, &book)
            .iter()
            .map(|offer| offer.topic)
            .collect();
        assert_eq!(after, [0, 1, 2]);
        assert_eq!(ask_label(case, ashby.topics[2].ask), "Ask about Mr Corwin");
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
            may_pass(&service, locked, Role::Guest, key, at(18, 0)),
            Err(Refusal::ServiceOnly),
            "a service door refuses the Guest even with its key"
        );
        assert_eq!(
            may_pass(&service, locked, Role::Staff, None, at(18, 0)),
            Ok(())
        );

        assert_eq!(
            may_pass(&public, false, Role::Guest, None, at(23, 29)),
            Ok(())
        );
        assert_eq!(
            may_pass(&public, false, Role::Guest, None, at(23, 30)),
            Err(Refusal::Locked),
            "the case locks it for the last half hour"
        );
        assert_eq!(
            may_pass(&public, true, Role::Staff, None, at(18, 0)),
            Err(Refusal::Locked),
            "the key ring opens only what the case lists"
        );
    }
}

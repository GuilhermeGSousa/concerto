use concerto::ecs::{Component, Resource};
use glam::Vec3;

use crate::book::Casebook;
use crate::case::{
    DoorId, DoorState, GameTime, Id, Interaction, KeyId, NpcId, PlacementId, Role, RoomId,
};
use crate::casefile::{CaseData, room_title};
use crate::players::PLAYER_COUNT;
use crate::rules::{door_rule, may_pass, name_of};
use crate::session::{Effect, Session};

/// How many stops the dumbwaiter serves.
pub const LIFT_STOPS: u8 = 3;

/// Something a player can look at and use.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interactable {
    Npc(NpcId),
    Placement(PlacementId),
    Door(DoorId),
    BellPull(RoomId),
    BellBoard,
    Gong,
    Bed(u8),
    Lift(u8),
}

/// A door as it stands now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DoorNow {
    pub open: bool,
    pub locked: bool,
}

/// Where a carried item is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemPlace {
    /// On its anchor, where the night starts it.
    Home,
    Carried(u8),
    Dropped(Vec3),
    /// In the dumbwaiter's car.
    Lift,
}

/// Everything in the house that a night changes and the next night puts back.
#[derive(Resource, Clone, Debug)]
pub struct House {
    pub doors: Vec<DoorNow>,
    /// A raised flag on the bell board for each room.
    pub bells: Vec<bool>,
    pub items: Vec<ItemPlace>,
    /// The stop the dumbwaiter's car is at.
    pub lift_at: u8,
    /// The characters stand at their places for the gathering.
    pub gathered: bool,
    /// A character is passing through the door, which stands open for them.
    pub held_open: Vec<bool>,
}

impl House {
    /// The house as each night starts.
    pub fn new(data: &CaseData) -> Self {
        Self {
            doors: data
                .level
                .doors()
                .map(|door| {
                    let state = door_rule(data, door).state;
                    DoorNow {
                        open: state == DoorState::Open,
                        locked: state == DoorState::Locked,
                    }
                })
                .collect(),
            bells: vec![false; data.catalog.rooms.len()],
            items: vec![ItemPlace::Home; data.case.placements.len()],
            lift_at: 0,
            gathered: false,
            held_open: vec![false; data.level.doors().count()],
        }
    }

    fn key_carried(session: &Session, data: &CaseData, player: u8) -> Option<KeyId> {
        let placement = session.seats[player as usize].carried?;
        match data.case.placements.get(placement).interaction {
            Interaction::Carry { key } => key,
            _ => None,
        }
    }

    /// Whether `door` stands open, by a player's hand or for a character passing.
    pub fn stands_open(&self, door: DoorId) -> bool {
        self.doors[door.index()].open || self.held_open[door.index()]
    }

    /// The item in the dumbwaiter's car.
    pub fn lift_item(&self) -> Option<PlacementId> {
        self.items
            .iter()
            .position(|place| *place == ItemPlace::Lift)
            .map(PlacementId::from_index)
    }

    /// Whether `placement` can be looked at where it is: nobody carries it and it is not in the car.
    pub fn in_the_open(&self, placement: PlacementId) -> bool {
        matches!(
            self.items[placement.index()],
            ItemPlace::Home | ItemPlace::Dropped(_)
        )
    }

    /// `player` puts down what they carry at `at`.
    pub fn put_down(&mut self, session: &mut Session, player: u8, at: Vec3) {
        if let Some(placement) = session.seats[player as usize].carried.take() {
            self.items[placement.index()] = ItemPlace::Dropped(at);
        }
    }

    /// `player` uses what they look at. `busy` says a character is in an entry that cannot be interrupted.
    pub fn use_target(
        &mut self,
        session: &mut Session,
        book: &mut Casebook,
        data: &CaseData,
        player: u8,
        target: Interactable,
        now: GameTime,
        busy: bool,
    ) -> Vec<Effect> {
        let seat = player as usize;
        let role = session.role(player);
        let mut effects = Vec::new();
        match target {
            Interactable::Npc(npc) => {
                if self.gathered {
                    session.tell(player, "Everyone is waiting for you to speak.");
                } else if data.case.characters.get(npc).questioned_by == role {
                    session.talk(player, npc, data, busy);
                } else {
                    let name = name_of(&data.case, npc);
                    session.tell(
                        player,
                        match role {
                            Role::Guest => {
                                format!("{name} is staff, and will not gossip with a guest.")
                            }
                            Role::Staff => format!("{name} does not speak to the staff."),
                        },
                    );
                }
            }
            Interactable::Placement(placement) => {
                match data.case.placements.get(placement).interaction {
                    Interaction::Scenery => {}
                    Interaction::Inspect { .. } | Interaction::Document { .. } => {
                        session.examine(player, placement, book, data)
                    }
                    Interaction::Carry { .. } => {
                        if session.seats[seat].carried.is_some() {
                            session.tell(player, "Your hands are full.");
                        } else if self.in_the_open(placement) {
                            self.items[placement.index()] = ItemPlace::Carried(player);
                            session.seats[seat].carried = Some(placement);
                        }
                    }
                }
            }
            Interactable::Door(door) => {
                let rule = door_rule(data, door);
                let state = &mut self.doors[door.index()];
                if !rule.enabled {
                    return effects;
                }
                let key = Self::key_carried(session, data, player);
                match may_pass(&rule, state.locked, role, key, now) {
                    Ok(()) => {
                        state.locked = false;
                        state.open = !state.open;
                    }
                    Err(refusal) => session.tell(player, refusal.text()),
                }
            }
            Interactable::BellPull(room) => {
                self.bells[room.index()] = true;
                session.tell(player, "You pull the bell cord.");
                let title = room_title(data.catalog.rooms.name(room));
                for other in 0..PLAYER_COUNT {
                    if session.role(other) == Role::Staff && other != player {
                        session.tell(other, format!("A bell rings: {title}."));
                    }
                }
            }
            Interactable::BellBoard => {
                let raised: Vec<&str> = data
                    .catalog
                    .rooms
                    .ids()
                    .filter(|room| self.bells[room.index()])
                    .map(|room| room_title(data.catalog.rooms.name(room)))
                    .collect();
                if raised.is_empty() {
                    session.tell(player, "No flag is up on the bell board.");
                } else {
                    session.tell(player, format!("Flags up: {}.", raised.join(", ")));
                    if role == Role::Staff {
                        self.bells.fill(false);
                    }
                }
            }
            Interactable::Gong => {
                if book.finale_ready(&data.case) {
                    self.gathered = true;
                    session.start_finale(book, data);
                } else {
                    session.tell(
                        player,
                        "The gong would gather the house. You cannot yet say what happened.",
                    );
                }
            }
            Interactable::Bed(owner) => {
                if owner != player {
                    session.tell(player, "That is not your bed.");
                } else {
                    let in_bed = !session.seats[seat].in_bed;
                    session.seats[seat].in_bed = in_bed;
                    session.tell(
                        player,
                        if in_bed {
                            "You lie down. The night ends when you both sleep."
                        } else {
                            "You get up."
                        },
                    );
                    let mut present = session.present_players().peekable();
                    if present.peek().is_some()
                        && present.all(|player| session.seats[player as usize].in_bed)
                    {
                        effects.push(Effect::EndNight);
                    }
                }
            }
            Interactable::Lift(stop) => {
                if self.lift_at != stop {
                    self.lift_at = stop;
                    session.tell(
                        player,
                        "The dumbwaiter rattles along its shaft and stops here.",
                    );
                } else if let Some(placement) = session.seats[seat].carried {
                    if self.lift_item().is_some() {
                        session.tell(player, "The car already holds something.");
                    } else {
                        session.seats[seat].carried = None;
                        self.items[placement.index()] = ItemPlace::Lift;
                    }
                } else if let Some(placement) = self.lift_item() {
                    self.items[placement.index()] = ItemPlace::Carried(player);
                    session.seats[seat].carried = Some(placement);
                } else {
                    session.tell(player, "The car is empty.");
                }
            }
        }
        effects
    }

    /// The word the prompt shows for `target`.
    pub fn prompt(
        &self,
        session: &Session,
        data: &CaseData,
        player: u8,
        target: Interactable,
    ) -> String {
        match target {
            Interactable::Npc(npc) => format!("Talk to {}", name_of(&data.case, npc)),
            Interactable::Placement(placement) => {
                let row = data.case.placements.get(placement);
                let verb = match row.interaction {
                    Interaction::Scenery => "",
                    Interaction::Inspect { .. } => "Inspect ",
                    Interaction::Document { .. } => "Read ",
                    Interaction::Carry { .. } => "Take ",
                };
                format!("{verb}{}", data.text(row.name))
            }
            Interactable::Door(door) => {
                if self.doors[door.index()].open {
                    "Close".into()
                } else {
                    "Open".into()
                }
            }
            Interactable::BellPull(_) => "Ring".into(),
            Interactable::BellBoard => "Read the bell board".into(),
            Interactable::Gong => "Strike the gong".into(),
            Interactable::Bed(_) => {
                if session.seats[player as usize].in_bed {
                    "Get up".into()
                } else {
                    "Sleep".into()
                }
            }
            Interactable::Lift(stop) => {
                if self.lift_at != stop {
                    "Call the dumbwaiter".into()
                } else if session.seats[player as usize].carried.is_some() {
                    "Send".into()
                } else {
                    "Take".into()
                }
            }
        }
    }
}

/// Ends the night: the house goes back to six o'clock, the casebook counts the night, and the card for the next one shows.
pub fn end_night(house: &mut House, session: &mut Session, book: &mut Casebook, data: &CaseData) {
    let new_facts = book.fresh().to_vec();
    book.end_night();
    *house = House::new(data);
    for seat in &mut session.seats {
        seat.carried = None;
    }
    session.show_card(book.loops() + 1, new_facts);
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{House, Interactable, ItemPlace, end_night};
    use crate::book::Casebook;
    use crate::case::{GameTime, Id};
    use crate::casefile::CaseData;
    use crate::session::{Effect, Mode, Session};

    struct Night {
        data: CaseData,
        house: House,
        session: Session,
        book: Casebook,
    }

    impl Night {
        fn new() -> Self {
            let data = CaseData::test();
            let mut session = Session::default();
            session.seats[0].present = true;
            session.seats[1].present = true;
            Self {
                house: House::new(&data),
                data,
                session,
                book: Casebook::default(),
            }
        }

        fn uses(&mut self, player: u8, target: Interactable) -> Vec<Effect> {
            self.house.use_target(
                &mut self.session,
                &mut self.book,
                &self.data,
                player,
                target,
                GameTime::from_hm(20, 0).unwrap(),
                false,
            )
        }

        fn told(&self, player: u8) -> &str {
            self.session.seats[player as usize]
                .told
                .as_ref()
                .map_or("", |told| told.text.as_str())
        }
    }

    #[test]
    fn roles_decide_who_talks_to_whom_and_who_passes_a_service_door() {
        let mut n = Night::new();
        let npc =
            |n: &Night, name: &str| Interactable::Npc(n.data.case.characters.id(name).unwrap());
        let service = n.data.catalog.doors.id("d_service").unwrap();

        let hobbs = npc(&n, "npc_03");
        n.uses(0, hobbs);
        assert!(
            n.session.is_walking(0),
            "the Guest gets nothing from the staff"
        );
        assert!(n.told(0).starts_with("Hobbs is staff"));
        n.uses(1, hobbs);
        assert!(matches!(n.session.mode(1), Mode::Talking { .. }));

        n.uses(0, Interactable::Door(service));
        assert!(n.told(0).starts_with("Staff only"));
        assert!(!n.house.doors[service.index()].open);
        n.session.seats[1].mode = Mode::Walking;
        n.uses(1, Interactable::Door(service));
        let door = n.house.doors[service.index()];
        assert!(
            door.open && !door.locked,
            "the key ring unlocks and opens it"
        );

        n.uses(1, Interactable::Door(service));
        assert!(!n.house.stands_open(service));
        n.house.held_open[service.index()] = true;
        assert!(
            n.house.stands_open(service),
            "a closed door stands open while a character passes"
        );
    }

    #[test]
    fn an_item_is_carried_sent_through_the_dumbwaiter_and_put_down() {
        let mut n = Night::new();
        let cup = n.data.case.placements.id("p_cup").unwrap();
        let key = n.data.case.placements.id("p_key").unwrap();

        n.uses(0, Interactable::Placement(cup));
        assert_eq!(n.session.seats[0].carried, Some(cup));
        n.uses(0, Interactable::Placement(key));
        assert_eq!(n.told(0), "Your hands are full.");

        n.uses(0, Interactable::Lift(2));
        assert_eq!(n.house.lift_at, 2, "the first use calls the car");
        n.uses(0, Interactable::Lift(2));
        assert_eq!(n.house.lift_item(), Some(cup));
        assert_eq!(n.session.seats[0].carried, None);

        n.uses(1, Interactable::Lift(0));
        n.uses(1, Interactable::Lift(0));
        assert_eq!(n.session.seats[1].carried, Some(cup));
        n.house
            .put_down(&mut n.session, 1, Vec3::new(1.0, 0.8, -7.0));
        assert_eq!(
            n.house.items[cup.index()],
            ItemPlace::Dropped(Vec3::new(1.0, 0.8, -7.0))
        );
    }

    #[test]
    fn a_bell_raises_its_flag_until_the_staff_reads_the_board() {
        let mut n = Night::new();
        let study = n.data.catalog.rooms.id("room_b").unwrap();

        n.uses(0, Interactable::BellPull(study));
        assert_eq!(n.told(1), "A bell rings: Study.");
        n.uses(0, Interactable::BellBoard);
        assert_eq!(n.told(0), "Flags up: Study.");
        assert!(
            n.house.bells[study.index()],
            "the Guest reading it drops nothing"
        );
        n.uses(1, Interactable::BellBoard);
        assert!(!n.house.bells[study.index()]);
        n.uses(1, Interactable::BellBoard);
        assert_eq!(n.told(1), "No flag is up on the bell board.");
    }

    #[test]
    fn both_players_in_bed_end_the_night_and_the_house_starts_over() {
        let mut n = Night::new();
        let cup = n.data.case.placements.id("p_cup").unwrap();
        let fact = n.data.case.facts.id("f_01").unwrap();
        n.book.grant(fact);
        n.uses(0, Interactable::Placement(cup));

        n.uses(0, Interactable::Bed(1));
        assert_eq!(n.told(0), "That is not your bed.");
        assert!(n.uses(0, Interactable::Bed(0)).is_empty());
        assert_eq!(n.uses(1, Interactable::Bed(1)), [Effect::EndNight]);

        end_night(&mut n.house, &mut n.session, &mut n.book, &n.data);
        assert_eq!(n.house.items[cup.index()], ItemPlace::Home);
        assert_eq!(n.session.seats[0].carried, None);
        assert!(!n.session.seats[0].in_bed);
        let card = n.session.card.clone().unwrap();
        assert_eq!(card.night, 2);
        assert_eq!(card.new_facts, [fact]);
        assert!(n.book.knows(fact), "what was learned stays");
    }

    #[test]
    fn the_gong_gathers_the_house_only_once_the_players_can_say_what_happened() {
        let mut n = Night::new();

        n.uses(0, Interactable::Gong);
        assert!(n.session.finale.is_none());
        assert!(n.told(0).starts_with("The gong would gather"));
    }
}

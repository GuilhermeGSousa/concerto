use std::fmt;

use serde::Deserialize;

use crate::{
    ActivityId, AnchorId, ConvId, DeductionId, DoorId, FactId, GameTime, KeyId, NpcId, PlacementId,
    PortraitId, PropId, RoomId, StrId, Table, TermBankId, TermId, TimeWindow,
};

/// A loaded case: every file parsed and every reference resolved into a typed id.
#[derive(Debug)]
pub struct Case {
    pub title: StrId,
    pub strings: Table<StrId, Text>,
    pub characters: Table<NpcId, Character>,
    pub schedules: Vec<Schedule>,
    pub conversations: Table<ConvId, Conversation>,
    pub dialogues: Vec<Dialogue>,
    pub facts: Table<FactId, Fact>,
    pub terms: Table<TermId, Term>,
    pub term_banks: Table<TermBankId, TermBank>,
    pub deductions: Table<DeductionId, Deduction>,
    pub finale: Finale,
    pub placements: Table<PlacementId, Placement>,
    pub doors: Doors,
}

impl Case {
    pub fn schedule(&self, npc: NpcId) -> Option<&Schedule> {
        self.schedules.iter().find(|schedule| schedule.npc == npc)
    }

    pub fn dialogue(&self, npc: NpcId) -> Option<&Dialogue> {
        self.dialogues.iter().find(|dialogue| dialogue.npc == npc)
    }
}

/// A string of the case; its `Debug` output hides the text.
#[derive(Clone, PartialEq, Eq)]
pub struct Text(String);

impl Text {
    pub(crate) fn new(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Text(..)")
    }
}

/// Which player a rule is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum Role {
    Guest,
    Staff,
}

/// Whether a schedule entry or a line belongs before or after the discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum Phase {
    Before,
    After,
}

/// The casebook category of a fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum Category {
    Person,
    Place,
    Time,
    Object,
    Event,
}

/// How a fact is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum Source {
    Seen,
    Heard,
    Read,
    Told,
}

/// The state a door starts each loop in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum DoorState {
    Open,
    Closed,
    Locked,
}

/// Who may pass a door.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum Zone {
    Public,
    Service,
    Private,
}

/// A person of the case, with or without a schedule.
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub name: StrId,
    pub portrait: PortraitId,
    pub questioned_by: Role,
}

/// Where one character is through the night.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    pub npc: NpcId,
    pub entries: Vec<ScheduleEntry>,
}

/// One stop of a schedule.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleEntry {
    pub start: GameTime,
    pub anchor: AnchorId,
    pub activity: ActivityId,
    pub facing: Option<f32>,
    pub phase: Phase,
    pub critical: bool,
    pub conversation: Option<ConvId>,
}

/// A conversation between characters that players overhear.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub participants: Vec<NpcId>,
    pub private: bool,
    pub lines: Vec<SpokenLine>,
    pub cover: Vec<SpokenLine>,
}

/// One subtitle, who says it, how long it stays up and what hearing it grants.
#[derive(Clone, Debug, PartialEq)]
pub struct SpokenLine {
    pub speaker: NpcId,
    pub text: StrId,
    pub seconds: f32,
    pub grants: Vec<FactId>,
}

/// What one character answers when a player talks to them.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialogue {
    pub npc: NpcId,
    pub brush_off: StrId,
    pub no_answer: StrId,
    pub topics: Vec<Topic>,
}

/// One thing a player can ask, and the answers to it in order of preference.
#[derive(Clone, Debug, PartialEq)]
pub struct Topic {
    pub ask: Ask,
    pub responses: Vec<Response>,
}

/// What a topic is about: a fixed question, a casebook fact or a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Question(StrId),
    Fact(FactId),
    Person(NpcId),
}

/// An answer, the conditions it needs and the facts it grants.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub when: Condition,
    pub lines: Vec<StrId>,
    pub grants: Vec<FactId>,
}

/// What must hold for a response to be given; an empty condition always holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Condition {
    pub window: Option<TimeWindow>,
    pub phase: Option<Phase>,
    pub knows: Vec<FactId>,
}

/// One entry of the casebook.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    pub text: StrId,
    pub category: Category,
    pub source: Source,
    pub people: Vec<NpcId>,
    pub place: Option<RoomId>,
    pub time: Option<GameTime>,
    pub object: Option<PlacementId>,
    pub unlocks_terms: Vec<TermId>,
}

/// A word a blank can be filled with.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub text: StrId,
}

/// The terms one kind of blank picks from.
#[derive(Clone, Debug, PartialEq)]
pub struct TermBank {
    pub terms: Vec<TermId>,
}

/// A sentence with blanks that both players complete.
#[derive(Clone, Debug, PartialEq)]
pub struct Deduction {
    pub template: StrId,
    pub blanks: Vec<Blank>,
    pub appears_with: Vec<FactId>,
    pub prerequisite_for_finale: bool,
    pub unlocks: Vec<DeductionId>,
}

/// One blank of a deduction: the bank it picks from and the right term.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blank {
    pub bank: TermBankId,
    pub answer: TermId,
}

/// The gathering that ends the game.
#[derive(Clone, Debug, PartialEq)]
pub struct Finale {
    pub staging: Vec<Staging>,
    pub statements: Vec<Statement>,
    pub ending: Vec<StrId>,
}

/// Where one character stands during the finale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Staging {
    pub npc: NpcId,
    pub anchor: AnchorId,
}

/// One final statement and how the room reacts when it is right.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub deduction: DeductionId,
    pub reaction: Vec<SpokenLine>,
}

/// A prop the case puts on an anchor.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub prop: PropId,
    pub anchor: AnchorId,
    pub name: StrId,
    pub interaction: Interaction,
}

/// What a player can do with a placed prop.
#[derive(Clone, Debug, PartialEq)]
pub enum Interaction {
    Scenery,
    Inspect {
        hotspots: Vec<Hotspot>,
    },
    Document {
        pages: Vec<StrId>,
        grants: Vec<FactId>,
    },
    Carry {
        key: Option<KeyId>,
    },
}

/// A point of an inspected prop that a player steps to.
#[derive(Clone, Debug, PartialEq)]
pub struct Hotspot {
    pub label: StrId,
    pub grants: Vec<FactId>,
}

/// What the case changes about the level's doors.
#[derive(Clone, Debug, PartialEq)]
pub struct Doors {
    pub staff_opens: Vec<DoorId>,
    pub overrides: Vec<DoorOverride>,
}

impl Doors {
    pub fn override_for(&self, door: DoorId) -> Option<&DoorOverride> {
        self.overrides.iter().find(|entry| entry.door == door)
    }
}

/// The fields of one door the case replaces; `None` keeps the level's value.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorOverride {
    pub door: DoorId,
    pub enabled: Option<bool>,
    pub state: Option<DoorState>,
    pub key: Option<KeyId>,
    pub zone: Option<Zone>,
    pub locked: Vec<TimeWindow>,
}

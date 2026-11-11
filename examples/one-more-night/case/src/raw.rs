use std::fmt;

use serde::Deserialize;
use serde::de::{MapAccess, Visitor};

use crate::{Category, DoorState, GameTime, Phase, Role, Source, TimeWindow, Zone};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub title: String,
    #[serde(default)]
    pub discovery: Option<GameTime>,
}

pub struct StringEntry {
    pub id: String,
    pub text: String,
}

pub struct Strings(pub Vec<StringEntry>);

impl<'de> Deserialize<'de> for Strings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;

        impl<'de> Visitor<'de> for Entries {
            type Value = Strings;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map from string id to text")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strings, A::Error> {
                let mut entries = Vec::new();
                while let Some(id) = map.next_key::<String>()? {
                    let text = map.next_value::<String>()?;
                    entries.push(StringEntry { id, text });
                }
                Ok(Strings(entries))
            }
        }

        deserializer.deserialize_map(Entries)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub portrait: String,
    pub questioned_by: Role,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub npc: String,
    pub entries: Vec<ScheduleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleEntry {
    pub start: GameTime,
    pub anchor: String,
    pub activity: String,
    #[serde(default)]
    pub facing: Option<f32>,
    pub phase: Phase,
    #[serde(default)]
    pub critical: bool,
    #[serde(default)]
    pub conversation: Option<String>,
    #[serde(default)]
    pub rings: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub id: String,
    pub participants: Vec<String>,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub earshot: Option<f32>,
    pub lines: Vec<SpokenLine>,
    #[serde(default)]
    pub cover: Vec<SpokenLine>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpokenLine {
    pub speaker: String,
    pub text: String,
    pub seconds: f32,
    #[serde(default)]
    pub grants: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialogue {
    pub npc: String,
    pub brush_off: String,
    pub no_answer: String,
    pub topics: Vec<Topic>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Topic {
    pub ask: Ask,
    pub responses: Vec<Response>,
}

#[derive(Deserialize)]
pub enum Ask {
    Question(String),
    Fact(String),
    Person(String),
    Item(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    #[serde(default)]
    pub when: Condition,
    pub lines: Vec<String>,
    #[serde(default)]
    pub grants: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    #[serde(default)]
    pub window: Option<TimeWindow>,
    #[serde(default)]
    pub phase: Option<Phase>,
    #[serde(default)]
    pub knows: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub id: String,
    pub text: String,
    pub category: Category,
    pub source: Source,
    #[serde(default)]
    pub people: Vec<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub time: Option<GameTime>,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub unlocks_terms: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terms {
    pub terms: Vec<Term>,
    pub banks: Vec<TermBank>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub id: String,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermBank {
    pub id: String,
    pub terms: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deduction {
    pub id: String,
    pub template: String,
    pub blanks: Vec<Blank>,
    #[serde(default)]
    pub appears_with: Vec<String>,
    #[serde(default)]
    pub prerequisite_for_finale: bool,
    #[serde(default)]
    pub unlocks: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blank {
    pub bank: String,
    pub answer: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finale {
    pub staging: Vec<Staging>,
    pub statements: Vec<Statement>,
    pub ending: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Staging {
    pub npc: String,
    pub anchor: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Statement {
    pub deduction: String,
    #[serde(default)]
    pub reaction: Vec<SpokenLine>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub id: String,
    pub prop: String,
    pub anchor: String,
    pub name: String,
    pub interaction: Interaction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Interaction {
    Scenery,
    Inspect {
        hotspots: Vec<Hotspot>,
    },
    Document {
        pages: Vec<String>,
        #[serde(default)]
        grants: Vec<String>,
    },
    Carry {
        #[serde(default)]
        key: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hotspot {
    pub label: String,
    #[serde(default)]
    pub grants: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Features {
    pub on: Vec<String>,
    pub off: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Doors {
    #[serde(default)]
    pub staff_opens: Vec<String>,
    #[serde(default)]
    pub overrides: Vec<DoorOverride>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoorOverride {
    pub door: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub state: Option<DoorState>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub zone: Option<Zone>,
    #[serde(default)]
    pub locked: Vec<TimeWindow>,
}

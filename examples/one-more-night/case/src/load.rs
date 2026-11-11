use std::collections::{HashMap, HashSet};
use std::path::Path;

use ron::extensions::Extensions;
use serde::de::DeserializeOwned;

use crate::{
    Ask, Blank, Case, CaseError, Catalog, Character, Condition, Conversation, Deduction, Dialogue,
    DoorOverride, Doors, Fact, Features, Finale, GameTime, Hotspot, Id, Interaction, Names, NpcId,
    Phase, Placement, Problem, Response, Schedule, ScheduleEntry, SpokenLine, Staging, Statement,
    StrId, Table, Term, TermBank, Text, Topic, ids::is_id, raw,
};

/// The schema version this crate reads; `case.ron` must carry it.
pub const FORMAT_VERSION: u32 = 1;

const CASE: &str = "case";
const STRINGS: &str = "strings";
const CHARACTERS: &str = "characters";
const SCHEDULES: &str = "schedules";
const CONVERSATIONS: &str = "conversations";
const DIALOGUE: &str = "dialogue";
const FACTS: &str = "facts";
const TERMS: &str = "terms";
const DEDUCTIONS: &str = "deductions";
const FINALE: &str = "finale";
const PLACEMENTS: &str = "placements";
const DOORS: &str = "doors";
const FEATURES: &str = "features";

/// The files of a case directory, without their `.ron` extension.
pub const FILES: [&str; 13] = [
    CASE,
    STRINGS,
    CHARACTERS,
    SCHEDULES,
    CONVERSATIONS,
    DIALOGUE,
    FACTS,
    TERMS,
    DEDUCTIONS,
    FINALE,
    PLACEMENTS,
    DOORS,
    FEATURES,
];

/// The text of a case's files, held in memory.
#[derive(Clone, Default)]
pub struct CaseFiles {
    files: HashMap<String, String>,
}

impl CaseFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the file `name`, given without its extension.
    pub fn with_file(mut self, name: impl Into<String>, text: impl Into<String>) -> Self {
        self.files.insert(name.into(), text.into());
        self
    }

    pub fn text(&self, name: &str) -> Option<&str> {
        self.files.get(name).map(String::as_str)
    }

    /// Reads the case files a directory holds; one it lacks is reported when the case loads.
    pub fn read_dir(dir: &Path) -> Result<Self, CaseError> {
        let mut files = Self::new();
        for file in FILES {
            match std::fs::read_to_string(dir.join(format!("{file}.ron"))) {
                Ok(text) => files = files.with_file(file, text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(CaseError::Unreadable { file }),
            }
        }
        Ok(files)
    }
}

impl Case {
    /// Parses every file and resolves every reference against the case and `catalog`.
    pub fn load(files: &CaseFiles, catalog: &Catalog) -> Result<Case, CaseError> {
        let manifest: raw::Manifest = parse(files, CASE)?;
        if manifest.version != FORMAT_VERSION {
            return Err(CaseError::UnsupportedVersion {
                file: CASE,
                found: manifest.version,
            });
        }
        let strings: raw::Strings = parse(files, STRINGS)?;
        let characters: Vec<raw::Character> = parse(files, CHARACTERS)?;
        let schedules: Vec<raw::Schedule> = parse(files, SCHEDULES)?;
        let conversations: Vec<raw::Conversation> = parse(files, CONVERSATIONS)?;
        let dialogues: Vec<raw::Dialogue> = parse(files, DIALOGUE)?;
        let facts: Vec<raw::Fact> = parse(files, FACTS)?;
        let terms: raw::Terms = parse(files, TERMS)?;
        let deductions: Vec<raw::Deduction> = parse(files, DEDUCTIONS)?;
        let finale: raw::Finale = parse(files, FINALE)?;
        let placements: Vec<raw::Placement> = parse(files, PLACEMENTS)?;
        let doors: raw::Doors = parse(files, DOORS)?;
        let features: raw::Features = parse(files, FEATURES)?;

        let names = CaseNames {
            catalog,
            strings: Names::build(STRINGS, strings.0.iter().map(|row| row.id.as_str()))?,
            characters: Names::build(CHARACTERS, characters.iter().map(|row| row.id.as_str()))?,
            conversations: Names::build(
                CONVERSATIONS,
                conversations.iter().map(|row| row.id.as_str()),
            )?,
            facts: Names::build(FACTS, facts.iter().map(|row| row.id.as_str()))?,
            terms: Names::build(TERMS, terms.terms.iter().map(|row| row.id.as_str()))?,
            term_banks: Names::build(TERMS, terms.banks.iter().map(|row| row.id.as_str()))?,
            deductions: Names::build(DEDUCTIONS, deductions.iter().map(|row| row.id.as_str()))?,
            placements: Names::build(PLACEMENTS, placements.iter().map(|row| row.id.as_str()))?,
        };

        let title = find(
            &names.strings,
            &manifest.title,
            &At::new(CASE, "case", CASE),
        )?;
        let characters = characters
            .iter()
            .map(|row| names.character(row))
            .collect::<Result<Vec<_>, _>>()?;
        let conversations = conversations
            .iter()
            .map(|row| names.conversation(row))
            .collect::<Result<Vec<_>, _>>()?;
        let schedules = schedules
            .iter()
            .map(|row| names.schedule(row, &conversations))
            .collect::<Result<Vec<_>, _>>()?;
        unique(
            SCHEDULES,
            "schedule",
            &names,
            schedules.iter().map(|row| row.npc),
        )?;
        let dialogues = dialogues
            .iter()
            .map(|row| names.dialogue(row))
            .collect::<Result<Vec<_>, _>>()?;
        unique(
            DIALOGUE,
            "dialogue",
            &names,
            dialogues.iter().map(|row| row.npc),
        )?;
        let facts = facts
            .iter()
            .map(|row| names.fact(row))
            .collect::<Result<Vec<_>, _>>()?;
        let term_rows = terms
            .terms
            .iter()
            .map(|row| names.term(row))
            .collect::<Result<Vec<_>, _>>()?;
        let term_banks = terms
            .banks
            .iter()
            .map(|row| names.term_bank(row))
            .collect::<Result<Vec<_>, _>>()?;
        let deductions = deductions
            .iter()
            .map(|row| names.deduction(row, &term_banks, &strings))
            .collect::<Result<Vec<_>, _>>()?;
        let finale = names.finale(&finale)?;
        let placements = placements
            .iter()
            .map(|row| names.placement(row))
            .collect::<Result<Vec<_>, _>>()?;
        let doors = names.doors(&doors)?;
        let features = names.features(&features)?;
        let texts = strings
            .0
            .into_iter()
            .map(|row| Text::new(row.text))
            .collect();

        Ok(Case {
            title,
            discovery: manifest.discovery,
            strings: Table::new(names.strings, texts),
            characters: Table::new(names.characters, characters),
            schedules,
            conversations: Table::new(names.conversations, conversations),
            dialogues,
            facts: Table::new(names.facts, facts),
            terms: Table::new(names.terms, term_rows),
            term_banks: Table::new(names.term_banks, term_banks),
            deductions: Table::new(names.deductions, deductions),
            finale,
            placements: Table::new(names.placements, placements),
            doors,
            features,
        })
    }

    /// Loads the case in a directory of `.ron` files.
    pub fn load_dir(dir: &Path, catalog: &Catalog) -> Result<Case, CaseError> {
        Case::load(&CaseFiles::read_dir(dir)?, catalog)
    }
}

fn parse<T: DeserializeOwned>(files: &CaseFiles, file: &'static str) -> Result<T, CaseError> {
    let text = files.text(file).ok_or(CaseError::MissingFile { file })?;
    ron::Options::default()
        .with_default_extension(Extensions::IMPLICIT_SOME)
        .from_str(text)
        .map_err(|error| CaseError::Syntax {
            file,
            line: error.span.start.line,
            column: error.span.start.col,
        })
}

struct At<'a> {
    file: &'static str,
    kind: &'static str,
    id: &'a str,
}

impl<'a> At<'a> {
    fn new(file: &'static str, kind: &'static str, id: &'a str) -> Self {
        Self { file, kind, id }
    }

    fn owner(&self) -> String {
        let id = if is_id(self.id) { self.id } else { "?" };
        format!("{} `{id}`", self.kind)
    }

    fn invalid(&self, problem: Problem) -> CaseError {
        CaseError::Invalid {
            file: self.file,
            owner: self.owner(),
            problem,
        }
    }
}

fn gaps(template: &str) -> usize {
    template
        .split(|c| c != '_')
        .filter(|run| run.len() >= 3)
        .count()
}

fn find<I: Id>(names: &Names<I>, name: &str, at: &At) -> Result<I, CaseError> {
    if !is_id(name) {
        return Err(CaseError::NotAnId {
            file: at.file,
            owner: at.owner(),
            kind: I::KIND,
        });
    }
    names.id(name).ok_or_else(|| CaseError::Unknown {
        file: at.file,
        owner: at.owner(),
        kind: I::KIND,
        id: name.to_string(),
    })
}

fn find_all<I: Id>(names: &Names<I>, list: &[String], at: &At) -> Result<Vec<I>, CaseError> {
    list.iter().map(|name| find(names, name, at)).collect()
}

fn find_optional<I: Id>(
    names: &Names<I>,
    name: &Option<String>,
    at: &At,
) -> Result<Option<I>, CaseError> {
    name.as_deref()
        .map(|name| find(names, name, at))
        .transpose()
}

fn unique(
    file: &'static str,
    kind: &'static str,
    names: &CaseNames,
    npcs: impl Iterator<Item = NpcId>,
) -> Result<(), CaseError> {
    let mut seen = HashSet::new();
    for npc in npcs {
        if !seen.insert(npc) {
            return Err(CaseError::Duplicate {
                file,
                kind,
                id: names.characters.name(npc).to_string(),
            });
        }
    }
    Ok(())
}

struct CaseNames<'a> {
    catalog: &'a Catalog,
    strings: Names<StrId>,
    characters: Names<NpcId>,
    conversations: Names<crate::ConvId>,
    facts: Names<crate::FactId>,
    terms: Names<crate::TermId>,
    term_banks: Names<crate::TermBankId>,
    deductions: Names<crate::DeductionId>,
    placements: Names<crate::PlacementId>,
}

impl CaseNames<'_> {
    fn character(&self, row: &raw::Character) -> Result<Character, CaseError> {
        let at = At::new(CHARACTERS, "character", &row.id);
        Ok(Character {
            name: find(&self.strings, &row.name, &at)?,
            portrait: find(&self.catalog.portraits, &row.portrait, &at)?,
            questioned_by: row.questioned_by,
        })
    }

    fn schedule(
        &self,
        row: &raw::Schedule,
        conversations: &[Conversation],
    ) -> Result<Schedule, CaseError> {
        let at = At::new(SCHEDULES, "schedule", &row.npc);
        let npc = find(&self.characters, &row.npc, &at)?;
        let entries = row
            .entries
            .iter()
            .map(|entry| {
                Ok(ScheduleEntry {
                    start: entry.start,
                    anchor: find(&self.catalog.anchors, &entry.anchor, &at)?,
                    activity: find(&self.catalog.activities, &entry.activity, &at)?,
                    facing: entry.facing,
                    phase: entry.phase,
                    critical: entry.critical,
                    conversation: find_optional(&self.conversations, &entry.conversation, &at)?,
                    rings: entry
                        .rings
                        .as_ref()
                        .map(|facts| find_all(&self.facts, facts, &at))
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<ScheduleEntry>, CaseError>>()?;
        if entries
            .iter()
            .any(|entry| entry.facing.is_some_and(|facing| !facing.is_finite()))
        {
            return Err(at.invalid(Problem::NumberOutOfRange));
        }
        let outsider = entries
            .iter()
            .filter_map(|entry| entry.conversation)
            .any(|id| !conversations[id.index()].participants.contains(&npc));
        if outsider {
            return Err(at.invalid(Problem::NotInConversation));
        }
        for phase in [Phase::Before, Phase::After] {
            let starts: Vec<GameTime> = entries
                .iter()
                .filter(|entry| entry.phase == phase)
                .map(|entry| entry.start)
                .collect();
            if starts.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(at.invalid(Problem::EntriesOutOfOrder));
            }
        }
        Ok(Schedule { npc, entries })
    }

    fn spoken_lines(
        &self,
        rows: &[raw::SpokenLine],
        at: &At,
    ) -> Result<Vec<SpokenLine>, CaseError> {
        rows.iter()
            .map(|row| {
                if !(row.seconds >= 0.0 && row.seconds.is_finite()) {
                    return Err(at.invalid(Problem::NegativeDuration));
                }
                Ok(SpokenLine {
                    speaker: find(&self.characters, &row.speaker, at)?,
                    text: find(&self.strings, &row.text, at)?,
                    seconds: row.seconds,
                    grants: find_all(&self.facts, &row.grants, at)?,
                })
            })
            .collect()
    }

    fn conversation(&self, row: &raw::Conversation) -> Result<Conversation, CaseError> {
        let at = At::new(CONVERSATIONS, "conversation", &row.id);
        if row
            .earshot
            .is_some_and(|earshot| !(earshot >= 0.0 && earshot.is_finite()))
        {
            return Err(at.invalid(Problem::NumberOutOfRange));
        }
        let conversation = Conversation {
            participants: find_all(&self.characters, &row.participants, &at)?,
            private: row.private,
            earshot: row.earshot,
            lines: self.spoken_lines(&row.lines, &at)?,
            cover: self.spoken_lines(&row.cover, &at)?,
        };
        let outsider = conversation
            .lines
            .iter()
            .chain(&conversation.cover)
            .any(|line| !conversation.participants.contains(&line.speaker));
        if outsider {
            return Err(at.invalid(Problem::SpeakerNotParticipant));
        }
        Ok(conversation)
    }

    fn dialogue(&self, row: &raw::Dialogue) -> Result<Dialogue, CaseError> {
        let at = At::new(DIALOGUE, "dialogue", &row.npc);
        let topics = row
            .topics
            .iter()
            .map(|topic| {
                let ask = match &topic.ask {
                    raw::Ask::Question(text) => Ask::Question(find(&self.strings, text, &at)?),
                    raw::Ask::Fact(fact) => Ask::Fact(find(&self.facts, fact, &at)?),
                    raw::Ask::Person(npc) => Ask::Person(find(&self.characters, npc, &at)?),
                    raw::Ask::Item(item) => Ask::Item(find(&self.placements, item, &at)?),
                };
                let responses = topic
                    .responses
                    .iter()
                    .map(|response| {
                        Ok(Response {
                            when: Condition {
                                window: response.when.window,
                                phase: response.when.phase,
                                knows: find_all(&self.facts, &response.when.knows, &at)?,
                            },
                            lines: find_all(&self.strings, &response.lines, &at)?,
                            grants: find_all(&self.facts, &response.grants, &at)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CaseError>>()?;
                Ok(Topic { ask, responses })
            })
            .collect::<Result<Vec<_>, CaseError>>()?;
        Ok(Dialogue {
            npc: find(&self.characters, &row.npc, &at)?,
            brush_off: find(&self.strings, &row.brush_off, &at)?,
            no_answer: find(&self.strings, &row.no_answer, &at)?,
            topics,
        })
    }

    fn fact(&self, row: &raw::Fact) -> Result<Fact, CaseError> {
        let at = At::new(FACTS, "fact", &row.id);
        Ok(Fact {
            text: find(&self.strings, &row.text, &at)?,
            category: row.category,
            source: row.source,
            people: find_all(&self.characters, &row.people, &at)?,
            place: find_optional(&self.catalog.rooms, &row.place, &at)?,
            time: row.time,
            object: find_optional(&self.placements, &row.object, &at)?,
            unlocks_terms: find_all(&self.terms, &row.unlocks_terms, &at)?,
        })
    }

    fn term(&self, row: &raw::Term) -> Result<Term, CaseError> {
        let at = At::new(TERMS, "term", &row.id);
        Ok(Term {
            text: find(&self.strings, &row.text, &at)?,
        })
    }

    fn term_bank(&self, row: &raw::TermBank) -> Result<TermBank, CaseError> {
        let at = At::new(TERMS, "term bank", &row.id);
        Ok(TermBank {
            terms: find_all(&self.terms, &row.terms, &at)?,
        })
    }

    fn deduction(
        &self,
        row: &raw::Deduction,
        banks: &[TermBank],
        strings: &raw::Strings,
    ) -> Result<Deduction, CaseError> {
        let at = At::new(DEDUCTIONS, "deduction", &row.id);
        let blanks = row
            .blanks
            .iter()
            .map(|blank| {
                let resolved = Blank {
                    bank: find(&self.term_banks, &blank.bank, &at)?,
                    answer: find(&self.terms, &blank.answer, &at)?,
                };
                if !banks[resolved.bank.index()]
                    .terms
                    .contains(&resolved.answer)
                {
                    return Err(at.invalid(Problem::AnswerNotInBank));
                }
                Ok(resolved)
            })
            .collect::<Result<Vec<_>, CaseError>>()?;
        let template = find(&self.strings, &row.template, &at)?;
        if gaps(&strings.0[template.index()].text) != blanks.len() {
            return Err(at.invalid(Problem::BlankCountMismatch));
        }
        Ok(Deduction {
            template,
            blanks,
            appears_with: find_all(&self.facts, &row.appears_with, &at)?,
            prerequisite_for_finale: row.prerequisite_for_finale,
            unlocks: find_all(&self.deductions, &row.unlocks, &at)?,
        })
    }

    fn finale(&self, row: &raw::Finale) -> Result<Finale, CaseError> {
        let at = At::new(FINALE, "finale", FINALE);
        let staging = row
            .staging
            .iter()
            .map(|place| {
                Ok(Staging {
                    npc: find(&self.characters, &place.npc, &at)?,
                    anchor: find(&self.catalog.anchors, &place.anchor, &at)?,
                })
            })
            .collect::<Result<Vec<Staging>, CaseError>>()?;
        let mut staged = HashSet::new();
        if !staging.iter().all(|place| staged.insert(place.npc)) {
            return Err(at.invalid(Problem::StagedTwice));
        }
        let statements = row
            .statements
            .iter()
            .map(|statement| {
                Ok(Statement {
                    deduction: find(&self.deductions, &statement.deduction, &at)?,
                    reaction: self.spoken_lines(&statement.reaction, &at)?,
                })
            })
            .collect::<Result<Vec<_>, CaseError>>()?;
        Ok(Finale {
            staging,
            statements,
            ending: find_all(&self.strings, &row.ending, &at)?,
        })
    }

    fn placement(&self, row: &raw::Placement) -> Result<Placement, CaseError> {
        let at = At::new(PLACEMENTS, "placement", &row.id);
        let interaction = match &row.interaction {
            raw::Interaction::Scenery => Interaction::Scenery,
            raw::Interaction::Inspect { hotspots } => Interaction::Inspect {
                hotspots: hotspots
                    .iter()
                    .map(|hotspot| {
                        Ok(Hotspot {
                            label: find(&self.strings, &hotspot.label, &at)?,
                            grants: find_all(&self.facts, &hotspot.grants, &at)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CaseError>>()?,
            },
            raw::Interaction::Document { pages, grants } => Interaction::Document {
                pages: find_all(&self.strings, pages, &at)?,
                grants: find_all(&self.facts, grants, &at)?,
            },
            raw::Interaction::Carry { key } => Interaction::Carry {
                key: find_optional(&self.catalog.keys, key, &at)?,
            },
        };
        Ok(Placement {
            prop: find(&self.catalog.props, &row.prop, &at)?,
            anchor: find(&self.catalog.anchors, &row.anchor, &at)?,
            name: find(&self.strings, &row.name, &at)?,
            interaction,
        })
    }

    fn features(&self, row: &raw::Features) -> Result<Features, CaseError> {
        let at = At::new(FEATURES, "features", FEATURES);
        let on = find_all(&self.catalog.features, &row.on, &at)?;
        let off = find_all(&self.catalog.features, &row.off, &at)?;
        let mut set = HashSet::new();
        for feature in on.iter().chain(&off) {
            if !set.insert(*feature) {
                return Err(CaseError::Duplicate {
                    file: FEATURES,
                    kind: "feature setting",
                    id: self.catalog.features.name(*feature).to_string(),
                });
            }
        }
        if let Some(missing) = self.catalog.features.ids().find(|id| !set.contains(id)) {
            let name = self.catalog.features.name(missing);
            return Err(At::new(FEATURES, "feature", name).invalid(Problem::FeatureNotSet));
        }
        Ok(Features { on })
    }

    fn doors(&self, row: &raw::Doors) -> Result<Doors, CaseError> {
        let at = At::new(DOORS, "doors", DOORS);
        let mut seen = HashSet::new();
        let overrides = row
            .overrides
            .iter()
            .map(|entry| {
                let at = At::new(DOORS, "door", &entry.door);
                let door = find(&self.catalog.doors, &entry.door, &at)?;
                if !seen.insert(door) {
                    return Err(CaseError::Duplicate {
                        file: DOORS,
                        kind: "door override",
                        id: entry.door.clone(),
                    });
                }
                Ok(DoorOverride {
                    door,
                    enabled: entry.enabled,
                    state: entry.state,
                    key: find_optional(&self.catalog.keys, &entry.key, &at)?,
                    zone: entry.zone,
                    locked: entry.locked.clone(),
                })
            })
            .collect::<Result<Vec<_>, CaseError>>()?;
        Ok(Doors {
            staff_opens: find_all(&self.catalog.doors, &row.staff_opens, &at)?,
            overrides,
        })
    }
}

use concerto::ecs::Resource;

use crate::book::{Casebook, Verdict};
use crate::case::{
    DeductionId, FactId, GameTime, Interaction, NpcId, Phase, PlacementId, Role, StrId,
};
use crate::casefile::{CaseData, room_title};
use crate::players::PLAYER_COUNT;
use crate::rules::{ask_label, name_of, offers};

const SEATS: usize = PLAYER_COUNT as usize;

/// A button press a screen answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Book,
}

/// A page of the casebook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    People,
    Timeline,
    Places,
    Objects,
    Deductions,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::People,
        Page::Timeline,
        Page::Places,
        Page::Objects,
        Page::Deductions,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::People => "People",
            Page::Timeline => "Timeline",
            Page::Places => "Places",
            Page::Objects => "Objects",
            Page::Deductions => "Deductions",
        }
    }

    fn step(self, forward: bool) -> Page {
        let at = Self::ALL.iter().position(|page| *page == self).unwrap_or(0);
        let step = if forward { 1 } else { Self::ALL.len() - 1 };
        Self::ALL[(at + step) % Self::ALL.len()]
    }
}

/// Lines an NPC is saying to a player, one at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saying {
    pub lines: Vec<StrId>,
    pub at: usize,
    /// The talk ends with the last line.
    pub leaves: bool,
}

/// What a player is doing.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Walking,
    Book {
        page: Page,
        row: usize,
        editing: Option<DeductionId>,
        blank: usize,
    },
    Talking {
        npc: NpcId,
        row: usize,
        saying: Option<Saying>,
    },
    Inspecting {
        placement: PlacementId,
        spot: usize,
    },
    Reading {
        placement: PlacementId,
        page: usize,
    },
    Finale {
        blank: usize,
    },
    Card,
}

/// Where the gathering stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinaleStage {
    Asking,
    Reaction { line: usize },
    Ending { card: usize },
}

/// The gathering in the drawing room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finale {
    pub statement: usize,
    pub stage: FinaleStage,
}

/// The card between two nights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub night: u32,
    pub new_facts: Vec<FactId>,
}

/// A line shown to both players for a few seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub seconds: f32,
}

/// One player's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seat {
    pub mode: Mode,
    /// Whether a device has joined for this player.
    pub present: bool,
    pub carried: Option<PlacementId>,
    pub in_bed: bool,
    /// What the screen last said about a lock, a door or a refusal.
    pub message: Option<String>,
    /// A line for this player alone, and the seconds it still shows.
    pub told: Option<Notice>,
}

/// Something a press asks of the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    EndNight,
}

/// What a player's screen shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenText {
    pub title: String,
    pub lines: Vec<String>,
    pub cursor: Option<usize>,
    pub hint: String,
}

impl ScreenText {
    /// The screen as one text of at most `rows` lines, scrolled to keep the cursor in view and marking its line.
    pub fn laid_out(&self, rows: usize) -> String {
        let at = self
            .cursor
            .unwrap_or(0)
            .min(self.lines.len().saturating_sub(1));
        let first = at.saturating_sub(rows.saturating_sub(1));
        let mut text = self.title.clone();
        text.push('\n');
        for (index, line) in self.lines.iter().enumerate().skip(first).take(rows) {
            let mark = if self.cursor == Some(index) {
                "> "
            } else {
                "  "
            };
            text.push('\n');
            text.push_str(mark);
            text.push_str(line);
        }
        if !self.hint.is_empty() {
            text.push_str("\n\n");
            text.push_str(&self.hint);
        }
        text
    }
}

/// The night's shared state: who plays whom, what each player is doing, the gathering and the night card.
#[derive(Resource, Clone, Debug)]
pub struct Session {
    pub roles: [Role; SEATS],
    pub seats: [Seat; SEATS],
    pub finale: Option<Finale>,
    pub card: Option<Card>,
    pub notices: Vec<Notice>,
    pub finished: bool,
    /// The shared pause: the clock and both players hold still.
    pub paused: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            roles: [Role::Guest, Role::Staff],
            seats: Default::default(),
            finale: None,
            card: None,
            notices: Vec::new(),
            finished: false,
            paused: false,
        }
    }
}

fn role_title(role: Role) -> &'static str {
    match role {
        Role::Guest => "the Guest",
        Role::Staff => "the Staff",
    }
}

fn clock_text(time: GameTime) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

impl Session {
    pub fn role(&self, player: u8) -> Role {
        self.roles[player as usize]
    }

    pub fn role_title(&self, player: u8) -> &'static str {
        role_title(self.role(player))
    }

    pub fn mode(&self, player: u8) -> &Mode {
        &self.seats[player as usize].mode
    }

    pub fn is_walking(&self, player: u8) -> bool {
        *self.mode(player) == Mode::Walking
    }

    fn present(&self) -> [bool; SEATS] {
        std::array::from_fn(|seat| self.seats[seat].present)
    }

    /// Shows `text` to both players for a few seconds.
    pub fn notice(&mut self, text: impl Into<String>) {
        self.notices.push(Notice {
            text: text.into(),
            seconds: 5.0,
        });
    }

    /// Shows `text` to `player` alone for a few seconds.
    pub fn tell(&mut self, player: u8, text: impl Into<String>) {
        self.seats[player as usize].told = Some(Notice {
            text: text.into(),
            seconds: 4.0,
        });
    }

    /// Ages the notices and each player's own line by `seconds` and drops the old ones.
    pub fn age_notices(&mut self, seconds: f32) {
        for notice in &mut self.notices {
            notice.seconds -= seconds;
        }
        self.notices.retain(|notice| notice.seconds > 0.0);
        for seat in &mut self.seats {
            if let Some(told) = &mut seat.told {
                told.seconds -= seconds;
            }
            if seat.told.as_ref().is_some_and(|told| told.seconds <= 0.0) {
                seat.told = None;
            }
        }
    }

    /// The seats with a player in them.
    pub fn present_players(&self) -> impl Iterator<Item = u8> + '_ {
        (0..PLAYER_COUNT).filter(|player| self.seats[*player as usize].present)
    }

    /// The character `player` is talking to.
    pub fn talking_to(&self, player: u8) -> Option<NpcId> {
        match self.mode(player) {
            Mode::Talking { npc, .. } => Some(*npc),
            _ => None,
        }
    }

    /// Grants `facts` to both players and says which were new.
    pub fn grant(&mut self, book: &mut Casebook, data: &CaseData, facts: &[FactId]) {
        for fact in facts {
            if book.grant(*fact) {
                let text = data.text(data.case.facts.get(*fact).text);
                self.notice(format!("New fact: {text}"));
            }
        }
    }

    /// Opens the casebook for `player`.
    pub fn open_book(&mut self, player: u8) {
        let seat = &mut self.seats[player as usize];
        seat.message = None;
        seat.mode = Mode::Book {
            page: Page::People,
            row: 0,
            editing: None,
            blank: 0,
        };
    }

    /// Starts `player` talking to `npc`; a character who is `busy` says one line and the talk ends.
    pub fn talk(&mut self, player: u8, npc: NpcId, data: &CaseData, busy: bool) {
        let saying = data
            .case
            .dialogue(npc)
            .filter(|_| busy)
            .map(|dialogue| Saying {
                lines: vec![dialogue.brush_off],
                at: 0,
                leaves: true,
            });
        self.seats[player as usize].mode = Mode::Talking {
            npc,
            row: 0,
            saying,
        };
    }

    /// Brings `placement` up for `player` to look over or read, granting what its first view gives.
    pub fn examine(
        &mut self,
        player: u8,
        placement: PlacementId,
        book: &mut Casebook,
        data: &CaseData,
    ) {
        match &data.case.placements.get(placement).interaction {
            Interaction::Inspect { hotspots } => {
                self.seats[player as usize].mode = Mode::Inspecting { placement, spot: 0 };
                if let Some(first) = hotspots.first() {
                    self.grant(book, data, &first.grants);
                }
            }
            Interaction::Document { pages, grants } => {
                self.seats[player as usize].mode = Mode::Reading { placement, page: 0 };
                if pages.len() <= 1 {
                    self.grant(book, data, grants);
                }
            }
            _ => {}
        }
    }

    /// Gathers the players for the finale, at its first statement not yet confirmed.
    pub fn start_finale(&mut self, book: &Casebook, data: &CaseData) {
        let statements = &data.case.finale.statements;
        let statement = statements
            .iter()
            .position(|statement| !book.is_confirmed(statement.deduction));
        self.finale = Some(match statement {
            Some(statement) => Finale {
                statement,
                stage: FinaleStage::Asking,
            },
            None => Finale {
                statement: statements.len(),
                stage: FinaleStage::Ending { card: 0 },
            },
        });
        for seat in &mut self.seats {
            seat.message = None;
            seat.mode = Mode::Finale { blank: 0 };
        }
    }

    /// Shows the card for the night about to begin.
    pub fn show_card(&mut self, night: u32, new_facts: Vec<FactId>) {
        self.finale = None;
        self.card = Some(Card { night, new_facts });
        for seat in &mut self.seats {
            seat.message = None;
            seat.in_bed = false;
            seat.mode = Mode::Card;
        }
    }

    fn everyone_walks(&mut self) {
        for seat in &mut self.seats {
            seat.mode = Mode::Walking;
        }
    }

    fn judged(&mut self, verdict: Verdict) {
        for seat in &mut self.seats {
            seat.message = Some(verdict.text().to_string());
        }
    }

    fn row_count(&self, player: u8, book: &Casebook, data: &CaseData, now: GameTime) -> usize {
        self.view(player, book, data, now, Phase::Before)
            .map_or(0, |screen| screen.lines.len())
    }

    /// Applies `press` to `player`'s screen. A player who is walking has none, so nothing happens.
    pub fn press(
        &mut self,
        player: u8,
        press: Press,
        book: &mut Casebook,
        data: &CaseData,
        now: GameTime,
        phase: Phase,
    ) -> Vec<Effect> {
        let seat = player as usize;
        let present = self.present();
        let rows = self.row_count(player, book, data, now);
        let step = |row: usize, down: bool| {
            if down {
                (row + 1).min(rows.saturating_sub(1))
            } else {
                row.saturating_sub(1)
            }
        };
        let mut effects = Vec::new();
        match self.seats[seat].mode.clone() {
            Mode::Walking => {}
            Mode::Book {
                page,
                row,
                editing,
                blank,
            } => {
                let visible = book.visible_deductions(&data.case);
                let mut next = Mode::Book {
                    page,
                    row,
                    editing,
                    blank,
                };
                match (editing, press) {
                    (_, Press::Book) | (None, Press::Back) => next = Mode::Walking,
                    (Some(_), Press::Back) => {
                        next = Mode::Book {
                            page,
                            row,
                            editing: None,
                            blank: 0,
                        }
                    }
                    (Some(deduction), Press::Up | Press::Down) => {
                        let blanks = data.case.deductions.get(deduction).blanks.len();
                        let blank = if press == Press::Down {
                            (blank + 1).min(blanks.saturating_sub(1))
                        } else {
                            blank.saturating_sub(1)
                        };
                        next = Mode::Book {
                            page,
                            row,
                            editing,
                            blank,
                        };
                    }
                    (Some(deduction), Press::Left | Press::Right) => {
                        book.cycle(&data.case, player, deduction, blank, press == Press::Right);
                    }
                    (Some(deduction), Press::Confirm) => {
                        if let Some(verdict) =
                            book.toggle_lock(&data.case, player, deduction, present)
                        {
                            self.judged(verdict);
                        }
                    }
                    (None, Press::Left | Press::Right) => {
                        next = Mode::Book {
                            page: page.step(press == Press::Right),
                            row: 0,
                            editing: None,
                            blank: 0,
                        }
                    }
                    (None, Press::Up | Press::Down) => {
                        next = Mode::Book {
                            page,
                            row: step(row, press == Press::Down),
                            editing,
                            blank,
                        }
                    }
                    (None, Press::Confirm) => {
                        if page == Page::Deductions
                            && let Some(deduction) = visible.get(row)
                        {
                            self.seats[seat].message = None;
                            next = Mode::Book {
                                page,
                                row,
                                editing: Some(*deduction),
                                blank: 0,
                            };
                        }
                    }
                }
                self.seats[seat].mode = next;
            }
            Mode::Talking { npc, row, saying } => {
                let dialogue = data.case.dialogue(npc);
                let on_offer = dialogue
                    .map(|dialogue| offers(dialogue, now, phase, book))
                    .unwrap_or_default();
                let mut next = Mode::Talking {
                    npc,
                    row,
                    saying: saying.clone(),
                };
                match (saying, press) {
                    (Some(saying), Press::Confirm | Press::Back) => {
                        let at = saying.at + 1;
                        next = if at < saying.lines.len() && press == Press::Confirm {
                            Mode::Talking {
                                npc,
                                row,
                                saying: Some(Saying { at, ..saying }),
                            }
                        } else if saying.leaves {
                            Mode::Walking
                        } else {
                            Mode::Talking {
                                npc,
                                row,
                                saying: None,
                            }
                        };
                    }
                    (None, Press::Back) => next = Mode::Walking,
                    (None, Press::Up | Press::Down) => {
                        next = Mode::Talking {
                            npc,
                            row: step(row, press == Press::Down),
                            saying: None,
                        }
                    }
                    (None, Press::Confirm) => match (dialogue, on_offer.get(row)) {
                        (Some(dialogue), Some(offer)) => {
                            let response = &dialogue.topics[offer.topic].responses[offer.response];
                            self.grant(book, data, &response.grants);
                            next = Mode::Talking {
                                npc,
                                row,
                                saying: Some(Saying {
                                    lines: response.lines.clone(),
                                    at: 0,
                                    leaves: false,
                                }),
                            };
                        }
                        _ => next = Mode::Walking,
                    },
                    _ => {}
                }
                self.seats[seat].mode = next;
            }
            Mode::Inspecting { placement, spot } => match press {
                Press::Back | Press::Confirm => self.seats[seat].mode = Mode::Walking,
                Press::Up | Press::Down | Press::Left | Press::Right => {
                    if let Interaction::Inspect { hotspots } =
                        &data.case.placements.get(placement).interaction
                    {
                        let forward = matches!(press, Press::Down | Press::Right);
                        let spot = if forward {
                            (spot + 1).min(hotspots.len().saturating_sub(1))
                        } else {
                            spot.saturating_sub(1)
                        };
                        self.seats[seat].mode = Mode::Inspecting { placement, spot };
                        if let Some(hotspot) = hotspots.get(spot) {
                            self.grant(book, data, &hotspot.grants);
                        }
                    }
                }
                Press::Book => {}
            },
            Mode::Reading { placement, page } => {
                if let Interaction::Document { pages, grants } =
                    &data.case.placements.get(placement).interaction
                {
                    match press {
                        Press::Back => self.seats[seat].mode = Mode::Walking,
                        Press::Confirm | Press::Right | Press::Down => {
                            if page + 1 < pages.len() {
                                self.seats[seat].mode = Mode::Reading {
                                    placement,
                                    page: page + 1,
                                };
                                if page + 2 == pages.len() {
                                    self.grant(book, data, grants);
                                }
                            } else if press == Press::Confirm {
                                self.seats[seat].mode = Mode::Walking;
                            }
                        }
                        Press::Left | Press::Up => {
                            self.seats[seat].mode = Mode::Reading {
                                placement,
                                page: page.saturating_sub(1),
                            }
                        }
                        Press::Book => {}
                    }
                }
            }
            Mode::Finale { blank } => {
                let Some(finale) = self.finale else {
                    self.seats[seat].mode = Mode::Walking;
                    return effects;
                };
                let statements = &data.case.finale.statements;
                let next_statement = |book: &Casebook, after: usize| {
                    (after..statements.len())
                        .find(|index| !book.is_confirmed(statements[*index].deduction))
                };
                let advance = |book: &Casebook, after: usize| match next_statement(book, after) {
                    Some(statement) => Finale {
                        statement,
                        stage: FinaleStage::Asking,
                    },
                    None => Finale {
                        statement: statements.len(),
                        stage: FinaleStage::Ending { card: 0 },
                    },
                };
                match (finale.stage, press) {
                    (FinaleStage::Asking, Press::Up | Press::Down) => {
                        let blanks = data
                            .case
                            .deductions
                            .get(statements[finale.statement].deduction)
                            .blanks
                            .len();
                        let blank = if press == Press::Down {
                            (blank + 1).min(blanks.saturating_sub(1))
                        } else {
                            blank.saturating_sub(1)
                        };
                        self.seats[seat].mode = Mode::Finale { blank };
                    }
                    (FinaleStage::Asking, Press::Left | Press::Right) => {
                        let deduction = statements[finale.statement].deduction;
                        book.cycle(&data.case, player, deduction, blank, press == Press::Right);
                    }
                    (FinaleStage::Asking, Press::Confirm) => {
                        let statement = &statements[finale.statement];
                        match book.toggle_lock(&data.case, player, statement.deduction, present) {
                            Some(Verdict::Correct) => {
                                for seat in &mut self.seats {
                                    seat.message = None;
                                    seat.mode = Mode::Finale { blank: 0 };
                                }
                                self.finale = Some(if statement.reaction.is_empty() {
                                    advance(book, finale.statement + 1)
                                } else {
                                    Finale {
                                        statement: finale.statement,
                                        stage: FinaleStage::Reaction { line: 0 },
                                    }
                                });
                            }
                            Some(Verdict::Disagree) => self.judged(Verdict::Disagree),
                            Some(_) => {
                                self.finale = None;
                                self.everyone_walks();
                                self.notice("That is not what happened. The night ends.");
                                effects.push(Effect::EndNight);
                            }
                            None => {}
                        }
                    }
                    (FinaleStage::Reaction { line }, Press::Confirm) => {
                        let reaction = &statements[finale.statement].reaction;
                        self.finale = Some(if line + 1 < reaction.len() {
                            Finale {
                                statement: finale.statement,
                                stage: FinaleStage::Reaction { line: line + 1 },
                            }
                        } else {
                            advance(book, finale.statement + 1)
                        });
                    }
                    (FinaleStage::Ending { card }, Press::Confirm) => {
                        if card + 1 < data.case.finale.ending.len() {
                            self.finale = Some(Finale {
                                statement: finale.statement,
                                stage: FinaleStage::Ending { card: card + 1 },
                            });
                        } else {
                            self.finale = None;
                            self.finished = true;
                            self.everyone_walks();
                            self.notice("The End");
                        }
                    }
                    _ => {}
                }
            }
            Mode::Card => match press {
                Press::Confirm => {
                    self.card = None;
                    self.everyone_walks();
                }
                Press::Book => self.roles.swap(0, 1),
                _ => {}
            },
        }
        effects
    }

    fn facts_where(
        book: &Casebook,
        data: &CaseData,
        keep: impl Fn(&crate::case::Fact) -> bool,
    ) -> Vec<String> {
        book.facts()
            .iter()
            .map(|fact| data.case.facts.get(*fact))
            .filter(|fact| keep(fact))
            .map(|fact| format!("  {}", data.text(fact.text)))
            .collect()
    }

    fn book_lines(page: Page, book: &Casebook, data: &CaseData, player: u8) -> Vec<String> {
        let case = &data.case;
        let mut lines = Vec::new();
        match page {
            Page::People => {
                for npc in case.characters.ids() {
                    lines.push(name_of(case, npc).to_string());
                    lines.extend(Self::facts_where(book, data, |fact| {
                        fact.people.contains(&npc)
                    }));
                }
            }
            Page::Timeline => {
                let rooms: Vec<_> = data.catalog.rooms.ids().collect();
                let mut header = String::from("       ");
                for room in &rooms {
                    header.push_str(&format!(
                        "{:<18}",
                        room_title(data.catalog.rooms.name(*room))
                    ));
                }
                lines.push(header);
                for hour in 18..24u16 {
                    let mut row = format!("{hour:02}:00  ");
                    for room in &rooms {
                        let count = book
                            .facts()
                            .iter()
                            .map(|fact| case.facts.get(*fact))
                            .filter(|fact| {
                                fact.place == Some(*room)
                                    && fact.time.is_some_and(|time| time.hour() == hour)
                            })
                            .count();
                        let cell = match count {
                            0 => ".".to_string(),
                            count => format!("{count} noted"),
                        };
                        row.push_str(&format!("{cell:<18}"));
                    }
                    lines.push(row);
                }
                let mut timed: Vec<_> = book
                    .facts()
                    .iter()
                    .map(|fact| case.facts.get(*fact))
                    .filter(|fact| fact.place.is_some() && fact.time.is_some())
                    .collect();
                timed.sort_by_key(|fact| fact.time);
                for fact in timed {
                    let (Some(time), Some(room)) = (fact.time, fact.place) else {
                        continue;
                    };
                    lines.push(format!(
                        "{}  {}: {}",
                        clock_text(time),
                        room_title(data.catalog.rooms.name(room)),
                        data.text(fact.text)
                    ));
                }
            }
            Page::Places => {
                for room in data.catalog.rooms.ids() {
                    lines.push(room_title(data.catalog.rooms.name(room)).to_string());
                    lines.extend(Self::facts_where(book, data, |fact| {
                        fact.place == Some(room)
                    }));
                }
            }
            Page::Objects => {
                for placement in case.placements.ids() {
                    let facts =
                        Self::facts_where(book, data, |fact| fact.object == Some(placement));
                    if !facts.is_empty() {
                        lines.push(data.text(case.placements.get(placement).name).to_string());
                        lines.extend(facts);
                    }
                }
                if lines.is_empty() {
                    lines.push("Nothing noted about any object yet.".into());
                }
            }
            Page::Deductions => {
                for deduction in book.visible_deductions(case) {
                    let mark = if book.is_confirmed(deduction) {
                        " (confirmed)"
                    } else if book.draft(case, player, deduction).locked {
                        " (locked)"
                    } else {
                        ""
                    };
                    lines.push(format!("{}{mark}", book.sentence(case, player, deduction)));
                }
                if lines.is_empty() {
                    lines.push("Nothing to deduce yet.".into());
                }
            }
        }
        lines
    }

    fn deduction_screen(
        &self,
        title: String,
        player: u8,
        deduction: DeductionId,
        blank: usize,
        book: &Casebook,
        data: &CaseData,
    ) -> ScreenText {
        let case = &data.case;
        let draft = book.draft(case, player, deduction);
        let mut lines = vec![book.sentence(case, player, deduction)];
        for (index, pick) in draft.picks.iter().enumerate() {
            let term = pick.map_or("____", |term| data.text(case.terms.get(term).text));
            lines.push(format!("Blank {}: < {term} >", index + 1));
        }
        if book.is_confirmed(deduction) {
            lines.push("Confirmed.".into());
        } else if draft.locked {
            lines.push("Locked. Waiting for the other player.".into());
        }
        if let Some(message) = &self.seats[player as usize].message {
            lines.push(message.clone());
        }
        ScreenText {
            title,
            lines,
            cursor: Some(1 + blank),
            hint: "Up/Down: blank   Left/Right: word   A: lock or unlock   B: back".into(),
        }
    }

    /// What `player`'s screen shows; nothing while they walk.
    pub fn view(
        &self,
        player: u8,
        book: &Casebook,
        data: &CaseData,
        now: GameTime,
        phase: Phase,
    ) -> Option<ScreenText> {
        let case = &data.case;
        match self.mode(player) {
            Mode::Walking => None,
            Mode::Book {
                page,
                row,
                editing,
                blank,
            } => Some(match editing {
                Some(deduction) => self.deduction_screen(
                    "Casebook: Deductions".into(),
                    player,
                    *deduction,
                    *blank,
                    book,
                    data,
                ),
                None => ScreenText {
                    title: format!("Casebook: {}", page.title()),
                    lines: Self::book_lines(*page, book, data, player),
                    cursor: Some(*row),
                    hint: if *page == Page::Deductions {
                        "Left/Right: page   Up/Down: choose   A: fill in   B: close".into()
                    } else {
                        "Left/Right: page   Up/Down: scroll   B: close".into()
                    },
                },
            }),
            Mode::Talking { npc, row, saying } => Some(match saying {
                Some(saying) => ScreenText {
                    title: name_of(case, *npc).to_string(),
                    lines: vec![data.text(saying.lines[saying.at]).to_string()],
                    cursor: None,
                    hint: "A: go on".into(),
                },
                None => {
                    let mut lines: Vec<String> = case
                        .dialogue(*npc)
                        .map(|dialogue| {
                            offers(dialogue, now, phase, book)
                                .iter()
                                .map(|offer| ask_label(case, dialogue.topics[offer.topic].ask))
                                .collect()
                        })
                        .unwrap_or_default();
                    lines.push("Leave".into());
                    ScreenText {
                        title: name_of(case, *npc).to_string(),
                        lines,
                        cursor: Some(*row),
                        hint: "Up/Down: choose   A: ask   B: leave".into(),
                    }
                }
            }),
            Mode::Inspecting { placement, spot } => {
                let placement = case.placements.get(*placement);
                let lines = match &placement.interaction {
                    Interaction::Inspect { hotspots } => hotspots
                        .iter()
                        .enumerate()
                        .map(|(index, hotspot)| {
                            if index == *spot {
                                data.text(hotspot.label).to_string()
                            } else {
                                format!("Detail {}", index + 1)
                            }
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                Some(ScreenText {
                    title: data.text(placement.name).to_string(),
                    lines,
                    cursor: Some(*spot),
                    hint: "Up/Down: look closer   B: put back".into(),
                })
            }
            Mode::Reading { placement, page } => {
                let placement = case.placements.get(*placement);
                let pages: &[StrId] = match &placement.interaction {
                    Interaction::Document { pages, .. } => pages,
                    _ => &[],
                };
                Some(ScreenText {
                    title: data.text(placement.name).to_string(),
                    lines: pages
                        .get(*page)
                        .map(|text| data.text(*text).to_string())
                        .into_iter()
                        .collect(),
                    cursor: None,
                    hint: format!(
                        "Page {} of {}   A: next   B: put back",
                        page + 1,
                        pages.len()
                    ),
                })
            }
            Mode::Finale { blank } => {
                let finale = self.finale?;
                let statements = &case.finale.statements;
                Some(match finale.stage {
                    FinaleStage::Asking => self.deduction_screen(
                        format!(
                            "The gathering: statement {} of {}",
                            finale.statement + 1,
                            statements.len()
                        ),
                        player,
                        statements[finale.statement].deduction,
                        *blank,
                        book,
                        data,
                    ),
                    FinaleStage::Reaction { line } => {
                        let spoken = &statements[finale.statement].reaction[line];
                        ScreenText {
                            title: name_of(case, spoken.speaker).to_string(),
                            lines: vec![data.text(spoken.text).to_string()],
                            cursor: None,
                            hint: "A: go on".into(),
                        }
                    }
                    FinaleStage::Ending { card } => ScreenText {
                        title: data.text(case.title).to_string(),
                        lines: case
                            .finale
                            .ending
                            .get(card)
                            .map(|text| data.text(*text).to_string())
                            .into_iter()
                            .collect(),
                        cursor: None,
                        hint: "A: go on".into(),
                    },
                })
            }
            Mode::Card => {
                let card = self.card.as_ref()?;
                let mut lines = Vec::new();
                if card.new_facts.is_empty() {
                    lines.push(if card.night > 1 {
                        "No new facts last night.".to_string()
                    } else {
                        "The evening begins at six.".to_string()
                    });
                } else {
                    lines.push("New facts last night:".into());
                    for fact in &card.new_facts {
                        lines.push(format!("  {}", data.text(case.facts.get(*fact).text)));
                    }
                }
                lines.push(String::new());
                lines.push(format!("You are {}.", self.role_title(player)));
                Some(ScreenText {
                    title: format!("Night {}", card.night),
                    lines,
                    cursor: None,
                    hint: "A: begin   Y: swap roles".into(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, Mode, Page, Press, Session};
    use crate::book::Casebook;
    use crate::case::{GameTime, Phase, Role};
    use crate::casefile::CaseData;

    struct Table {
        data: CaseData,
        book: Casebook,
        session: Session,
        now: GameTime,
    }

    impl Table {
        fn new() -> Self {
            let mut session = Session::default();
            session.seats[0].present = true;
            session.seats[1].present = true;
            Self {
                data: CaseData::test(),
                book: Casebook::default(),
                session,
                now: GameTime::from_hm(20, 0).unwrap(),
            }
        }

        fn press(&mut self, player: u8, presses: &[Press]) -> Vec<Effect> {
            presses
                .iter()
                .flat_map(|press| {
                    self.session.press(
                        player,
                        *press,
                        &mut self.book,
                        &self.data,
                        self.now,
                        Phase::Before,
                    )
                })
                .collect()
        }

        fn lines(&self, player: u8) -> Vec<String> {
            self.session
                .view(player, &self.book, &self.data, self.now, Phase::Before)
                .map(|screen| screen.lines)
                .unwrap_or_default()
        }

        fn fact(&mut self, name: &str) {
            let fact = self.data.case.facts.id(name).unwrap();
            self.book.grant(fact);
        }

        fn confirm(&mut self, name: &str) {
            let deduction = self.data.case.deductions.id(name).unwrap();
            let case = &self.data.case;
            let blanks = case.deductions.get(deduction).blanks.clone();
            for player in 0..2 {
                for (blank, wanted) in blanks.iter().enumerate() {
                    for _ in 0..8 {
                        if self.book.draft(case, player, deduction).picks[blank]
                            == Some(wanted.answer)
                        {
                            break;
                        }
                        self.book.cycle(case, player, deduction, blank, true);
                    }
                }
                self.book.toggle_lock(case, player, deduction, [true, true]);
            }
            assert!(self.book.is_confirmed(deduction), "{name} needs more facts");
        }
    }

    #[test]
    fn talking_asks_what_is_on_offer_and_an_answer_can_grant_a_fact() {
        let mut t = Table::new();
        let hobbs = t.data.case.characters.id("npc_03").unwrap();
        t.session.talk(1, hobbs, &t.data, false);

        assert_eq!(t.lines(1), ["Anything unusual tonight?", "Leave"]);
        t.press(1, &[Press::Confirm]);
        assert!(t.lines(1)[0].starts_with("Mr Corwin came through"));
        assert_eq!(t.book.facts().len(), 1, "the answer granted its fact");
        assert_eq!(t.session.notices.len(), 1);
        t.press(1, &[Press::Confirm, Press::Down, Press::Confirm]);
        assert!(t.session.is_walking(1), "Leave ends the talk");

        t.session.talk(1, hobbs, &t.data, true);
        assert_eq!(t.lines(1), ["Beg pardon, I am needed."]);
        t.press(1, &[Press::Confirm]);
        assert!(t.session.is_walking(1), "a busy character says one line");
    }

    #[test]
    fn looking_a_prop_over_and_reading_a_note_grant_their_facts() {
        let mut t = Table::new();
        let case = t.data.case.placements.id("p_case").unwrap();
        let note = t.data.case.placements.id("p_note").unwrap();

        t.session.examine(0, case, &mut t.book, &t.data);
        assert!(t.book.facts().is_empty(), "the first detail grants nothing");
        t.press(0, &[Press::Down]);
        assert_eq!(t.book.facts().len(), 1);
        assert_eq!(
            t.lines(0),
            [
                "Detail 1",
                "The velvet is pressed flat where something small sat."
            ]
        );
        t.press(0, &[Press::Back]);

        t.session.examine(0, note, &mut t.book, &t.data);
        assert_eq!(t.book.facts().len(), 2, "a one-page note is read at once");
        t.press(0, &[Press::Back]);
        assert!(t.session.is_walking(0));
    }

    #[test]
    fn both_players_fill_a_deduction_in_the_casebook_and_lock_it() {
        let mut t = Table::new();
        t.fact("f_01");
        t.fact("f_02");
        for player in 0..2 {
            t.session.open_book(player);
            t.press(player, &[Press::Left]);
            assert!(matches!(
                t.session.mode(player),
                Mode::Book {
                    page: Page::Deductions,
                    ..
                }
            ));
            t.press(
                player,
                &[
                    Press::Confirm,
                    Press::Right,
                    Press::Down,
                    Press::Right,
                    Press::Confirm,
                ],
            );
        }

        let lines = t.lines(0);
        assert_eq!(lines[0], "[Mr Corwin] took [the snuffbox] from the study.");
        assert!(lines.contains(&"Confirmed.".to_string()), "{lines:?}");
        t.press(0, &[Press::Back]);
        assert_eq!(
            t.lines(0),
            [
                "[Mr Corwin] took [the snuffbox] from the study. (confirmed)",
                "It was hidden in [____] in the [____]."
            ]
        );
        t.press(0, &[Press::Book]);
        assert!(t.session.is_walking(0));
    }

    #[test]
    fn the_gathering_asks_each_statement_and_a_wrong_answer_ends_the_night() {
        let mut t = Table::new();
        for fact in ["f_01", "f_02", "f_03", "f_04", "f_05", "f_06"] {
            t.fact(fact);
        }
        t.confirm("d_01");
        t.confirm("d_02");
        assert!(t.book.finale_ready(&t.data.case));

        t.session.start_finale(&t.book, &t.data);
        assert_eq!(t.lines(0)[0], "The thief is [____].");
        let wrong: Vec<Effect> = (0..2)
            .flat_map(|player| t.press(player, &[Press::Right, Press::Confirm]))
            .collect();
        assert_eq!(wrong, [Effect::EndNight], "Lady Ashby is the wrong answer");
        assert!(t.session.is_walking(0) && t.session.finale.is_none());

        t.session.start_finale(&t.book, &t.data);
        for player in 0..2 {
            t.press(player, &[Press::Right, Press::Confirm]);
        }
        assert_eq!(t.lines(1), ["This is absurd. You have no proof."]);
        t.press(1, &[Press::Confirm]);
        assert_eq!(t.lines(0)[0], "The reason was [____].");
        for player in 0..2 {
            t.press(player, &[Press::Right, Press::Confirm]);
        }
        t.press(0, &[Press::Confirm]);
        assert_eq!(t.lines(0), ["The snuffbox goes back in its case."]);
        t.press(0, &[Press::Confirm, Press::Confirm]);
        assert!(t.session.finished);
        assert!(t.session.is_walking(0) && t.session.is_walking(1));
    }

    #[test]
    fn the_night_card_lists_new_facts_and_swaps_roles() {
        let mut t = Table::new();
        t.fact("f_01");
        t.session.show_card(2, t.book.fresh().to_vec());

        let lines = t.lines(1);
        assert_eq!(lines[0], "New facts last night:");
        assert_eq!(lines.last().unwrap(), "You are the Staff.");
        t.press(1, &[Press::Book]);
        assert_eq!(t.session.role(1), Role::Guest);
        t.press(0, &[Press::Confirm]);
        assert!(t.session.is_walking(0) && t.session.is_walking(1));
        assert!(t.session.card.is_none());
    }
}

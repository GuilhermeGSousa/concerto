use concerto::ecs::Resource;
use concerto::ui::grid::{GridStep, grid_step};

use crate::book::{Casebook, Verdict};
use crate::case::{
    DeductionId, FactId, GameTime, Id, Interaction, NpcId, Phase, PlacementId, Role, StrId,
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
    PagePrev,
    PageNext,
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

/// A person, room or object the casebook lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    pub label: String,
    /// The index of the portrait to show beside it.
    pub portrait: Option<usize>,
    /// How many found facts are about it.
    pub count: usize,
}

/// What a casebook page holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookBody {
    /// A list of subjects and the facts about the selected one.
    Subjects {
        subjects: Vec<Subject>,
        selected: usize,
        facts: Vec<String>,
    },
    /// Rooms across, hours down; `counts` holds a row of rooms for each hour, and `facts` those in the selected cell.
    Timeline {
        rooms: Vec<String>,
        hours: Vec<String>,
        counts: Vec<usize>,
        selected: usize,
        facts: Vec<String>,
    },
    Lines {
        lines: Vec<String>,
        cursor: Option<usize>,
    },
}

/// A player's open casebook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookScreen {
    pub page: Page,
    pub body: BookBody,
    pub hint: String,
}

const HOW_TO_PLAY: [&str; 6] = [
    "Something happens in this house tonight. Find out what.",
    "X uses what you look at. Y opens the casebook.",
    "The Guest talks to guests. The Staff talks to staff, opens service doors,",
    "and hears through a door by standing at it and holding RB.",
    "The night repeats. What you learn stays with you.",
    "Hold LB for your watch; hold both watches together to pass the time.",
];
const CONTROLS: [&str; 9] = [
    "Left stick or WASD: walk (left Shift walks slowly)",
    "Right stick or mouse: look",
    "X or E: use what the + is on",
    "Y or Tab: casebook    LB/RB or Page Up/Down: turn its pages",
    "A or Enter: choose    B or Q: back, or put down what you carry",
    "D-pad or arrows: move on a screen",
    "LB or C, held: pocket watch; both held: pass the time",
    "RB or R, held: the Staff listens at a door",
    "Start or Escape: pause",
];
const FIRST_HOUR: u16 = 18;
const HOURS: usize = 6;

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
    /// What the players have done for the first time, for the first night's lesson.
    pub learned: Learned,
}

/// What the players have done at least once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Learned {
    pub opened_book: bool,
    pub seen_deductions: bool,
    pub locked: bool,
}

/// A step of the first night's lesson.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lesson {
    pub step: usize,
    pub of: usize,
    pub text: &'static str,
}

impl Lesson {
    /// The step as one line.
    pub fn line(&self) -> String {
        format!(
            "First night, step {} of {}: {}",
            self.step, self.of, self.text
        )
    }
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
            learned: Learned::default(),
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
        self.learned.opened_book = true;
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
        self.view(player, book, data, now, data.case.phase_at(now))
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
        let rows = match self.mode(player) {
            Mode::Book { page, .. } => Self::book_rows(*page, book, data),
            _ => self.row_count(player, book, data, now),
        };
        let rooms = data.catalog.rooms.len().max(1);
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
                    (None, Press::PagePrev | Press::PageNext) => {
                        next = Mode::Book {
                            page: page.step(press == Press::PageNext),
                            row: 0,
                            editing: None,
                            blank: 0,
                        }
                    }
                    (Some(_), Press::PagePrev | Press::PageNext) => {}
                    (None, Press::Up | Press::Down | Press::Left | Press::Right)
                        if page == Page::Timeline =>
                    {
                        let step = match press {
                            Press::Up => GridStep::Up,
                            Press::Down => GridStep::Down,
                            Press::Left => GridStep::Left,
                            _ => GridStep::Right,
                        };
                        next = Mode::Book {
                            page,
                            row: grid_step(row, rooms, rows, step),
                            editing,
                            blank,
                        }
                    }
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
                        self.learned.locked = true;
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
                if matches!(
                    next,
                    Mode::Book {
                        page: Page::Deductions,
                        ..
                    }
                ) {
                    self.learned.seen_deductions = true;
                }
                self.seats[seat].mode = next;
            }
            Mode::Talking { npc, row, saying } => {
                let dialogue = data.case.dialogue(npc);
                let on_offer = dialogue
                    .map(|dialogue| offers(dialogue, now, phase, book, self.seats[seat].carried))
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
                Press::Book | Press::PagePrev | Press::PageNext => {}
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
                        Press::Book | Press::PagePrev | Press::PageNext => {}
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

    fn objects_noted(book: &Casebook, data: &CaseData) -> Vec<crate::case::PlacementId> {
        data.case
            .placements
            .ids()
            .filter(|placement| {
                book.facts()
                    .iter()
                    .any(|fact| data.case.facts.get(*fact).object == Some(*placement))
            })
            .collect()
    }

    fn book_rows(page: Page, book: &Casebook, data: &CaseData) -> usize {
        match page {
            Page::People => data.case.characters.len(),
            Page::Timeline => HOURS * data.catalog.rooms.len(),
            Page::Places => data.catalog.rooms.len(),
            Page::Objects => Self::objects_noted(book, data).len(),
            Page::Deductions => book.visible_deductions(&data.case).len(),
        }
        .max(1)
    }

    fn trimmed(
        book: &Casebook,
        data: &CaseData,
        keep: impl Fn(&crate::case::Fact) -> bool,
    ) -> Vec<String> {
        Self::facts_where(book, data, keep)
            .into_iter()
            .map(|line| line.trim_start().to_string())
            .collect()
    }

    /// `player`'s open casebook as a layout: its page, what the page lists and the selection. `None` unless the casebook is open.
    pub fn book_screen(&self, player: u8, book: &Casebook, data: &CaseData) -> Option<BookScreen> {
        let Mode::Book {
            page,
            row,
            editing,
            blank,
        } = self.mode(player)
        else {
            return None;
        };
        let case = &data.case;
        let subject = |label: &str, portrait: Option<usize>, facts: &[String]| Subject {
            label: label.to_string(),
            portrait,
            count: facts.len(),
        };
        let listed = |subjects: Vec<Subject>, facts: Vec<Vec<String>>| BookBody::Subjects {
            selected: (*row).min(subjects.len().saturating_sub(1)),
            facts: facts.get(*row).cloned().unwrap_or_default(),
            subjects,
        };
        let move_hint = "Up/Down: choose   LB/RB: page   B: close";
        let (body, hint) = match (page, editing) {
            (_, Some(deduction)) => {
                let screen =
                    self.deduction_screen(String::new(), player, *deduction, *blank, book, data);
                (
                    BookBody::Lines {
                        lines: screen.lines,
                        cursor: screen.cursor,
                    },
                    screen.hint,
                )
            }
            (Page::People, None) => {
                let facts: Vec<Vec<String>> = case
                    .characters
                    .ids()
                    .map(|npc| Self::trimmed(book, data, |fact| fact.people.contains(&npc)))
                    .collect();
                let subjects = case
                    .characters
                    .ids()
                    .zip(&facts)
                    .map(|(npc, facts)| {
                        let portrait = case.characters.get(npc).portrait.index();
                        subject(name_of(case, npc), Some(portrait), facts)
                    })
                    .collect();
                (listed(subjects, facts), move_hint.to_string())
            }
            (Page::Places, None) => {
                let facts: Vec<Vec<String>> = data
                    .catalog
                    .rooms
                    .ids()
                    .map(|room| Self::trimmed(book, data, |fact| fact.place == Some(room)))
                    .collect();
                let subjects = data
                    .catalog
                    .rooms
                    .ids()
                    .zip(&facts)
                    .map(|(room, facts)| {
                        subject(room_title(data.catalog.rooms.name(room)), None, facts)
                    })
                    .collect();
                (listed(subjects, facts), move_hint.to_string())
            }
            (Page::Objects, None) => {
                let objects = Self::objects_noted(book, data);
                let facts: Vec<Vec<String>> = objects
                    .iter()
                    .map(|object| Self::trimmed(book, data, |fact| fact.object == Some(*object)))
                    .collect();
                let subjects = objects
                    .iter()
                    .zip(&facts)
                    .map(|(object, facts)| {
                        subject(data.text(case.placements.get(*object).name), None, facts)
                    })
                    .collect();
                (listed(subjects, facts), move_hint.to_string())
            }
            (Page::Timeline, None) => {
                let rooms: Vec<_> = data.catalog.rooms.ids().collect();
                let cell = |hour: usize, room: crate::case::RoomId| {
                    Self::trimmed(book, data, |fact| {
                        fact.place == Some(room)
                            && fact
                                .time
                                .is_some_and(|time| time.hour() == FIRST_HOUR + hour as u16)
                    })
                };
                let counts = (0..HOURS)
                    .flat_map(|hour| rooms.iter().map(move |room| (hour, *room)))
                    .map(|(hour, room)| cell(hour, room).len())
                    .collect();
                let selected = (*row).min(HOURS * rooms.len().max(1) - 1);
                let facts = rooms
                    .get(selected % rooms.len().max(1))
                    .map(|room| cell(selected / rooms.len().max(1), *room))
                    .unwrap_or_default();
                (
                    BookBody::Timeline {
                        rooms: rooms
                            .iter()
                            .map(|room| room_title(data.catalog.rooms.name(*room)).to_string())
                            .collect(),
                        hours: (0..HOURS)
                            .map(|hour| format!("{:02}:00", FIRST_HOUR as usize + hour))
                            .collect(),
                        counts,
                        selected,
                        facts,
                    },
                    "D-pad: move   LB/RB: page   B: close".to_string(),
                )
            }
            (Page::Deductions, None) => (
                BookBody::Lines {
                    lines: Self::book_lines(Page::Deductions, book, data, player),
                    cursor: Some(*row),
                },
                "Up/Down: choose   A: fill in   LB/RB: page   B: close".to_string(),
            ),
        };
        let hint = match self.lesson(book) {
            Some(lesson) if lesson.step > 2 => format!("{}\n{hint}", lesson.line()),
            _ => hint,
        };
        Some(BookScreen {
            page: *page,
            body,
            hint,
        })
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
        let partner =
            (0..PLAYER_COUNT).find(|other| *other != player && self.seats[*other as usize].present);
        lines.push(String::new());
        if book.is_confirmed(deduction) {
            lines.push("Confirmed: this is what happened.".into());
        } else {
            lines.push(
                if draft.locked {
                    "You: LOCKED IN"
                } else {
                    "You: still choosing"
                }
                .into(),
            );
            match partner {
                Some(partner) => {
                    lines.push(
                        if book.draft(case, partner, deduction).locked {
                            "Other player: LOCKED IN"
                        } else {
                            "Other player: still choosing"
                        }
                        .into(),
                    );
                    lines.push(
                        "Once you have both locked in, the casebook says if you agree and if it is right."
                            .into(),
                    );
                }
                None => lines.push("Lock in your answer to test it.".into()),
            }
        }
        if let Some(message) = &self.seats[player as usize].message {
            lines.push(format!("Last try: {message}"));
        }
        ScreenText {
            title,
            lines,
            cursor: Some(1 + blank),
            hint: if draft.locked {
                "A: unlock to change your answer   B: back".into()
            } else {
                "Up/Down: blank   Left/Right: word   A: lock in   B: back".into()
            },
        }
    }

    /// The step of the first night's lesson the players are on; nothing once they have confirmed a deduction.
    pub fn lesson(&self, book: &Casebook) -> Option<Lesson> {
        let steps = [
            (
                !book.facts().is_empty(),
                "Find a fact: put the + on something and press X, listen to people, or ask them.",
            ),
            (self.learned.opened_book, "Press Y to open the casebook."),
            (
                self.learned.seen_deductions,
                "In the casebook, RB turns the page. Go to the last one, Deductions.",
            ),
            (
                self.learned.locked,
                "Pick a deduction with A, choose a word for each blank, then press A to lock in.",
            ),
            (
                !book.confirmed().is_empty(),
                "Lock in the same answer as each other. If a word is missing, a fact will teach it.",
            ),
        ];
        steps.iter().position(|(done, _)| !done).map(|step| Lesson {
            step: step + 1,
            of: steps.len(),
            text: steps[step].1,
        })
    }

    /// The next thing worth doing, said to a player who is walking around; nothing once they have the hang of it.
    pub fn guide(&self, player: u8, book: &Casebook, data: &CaseData) -> Option<String> {
        if !self.seats[player as usize].present || !self.is_walking(player) {
            return None;
        }
        if book.finale_ready(&data.case) && !self.finished {
            return Some("You can say what happened. Strike the gong in the drawing room.".into());
        }
        self.lesson(book).map(|lesson| lesson.line())
    }

    /// What both players see while the game is paused: the controls.
    pub fn pause_screen() -> ScreenText {
        ScreenText {
            title: "Paused".into(),
            lines: CONTROLS.iter().map(|line| line.to_string()).collect(),
            cursor: None,
            hint: "Start, or Escape: go on".into(),
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
                            offers(
                                dialogue,
                                now,
                                phase,
                                book,
                                self.seats[player as usize].carried,
                            )
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
                    hint: "Right stick: turn   Left stick: closer   Up/Down: look closer   B: put back"
                        .into(),
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
                if card.night == 1 {
                    lines.push(String::new());
                    lines.extend(HOW_TO_PLAY.iter().map(|line| line.to_string()));
                }
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
        t.fact("f_09");
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
        assert!(
            lines.contains(&"Confirmed: this is what happened.".to_string()),
            "{lines:?}"
        );
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
        for fact in ["f_01", "f_09", "f_03", "f_04", "f_05", "f_06"] {
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
    fn the_casebook_lists_subjects_and_moves_cell_by_cell_on_the_timeline() {
        use super::{BookBody, Subject};
        let mut t = Table::new();
        t.fact("f_03");
        t.fact("f_08");
        t.session.open_book(0);

        t.press(0, &[Press::Down]);
        let people = t.session.book_screen(0, &t.book, &t.data).unwrap();
        let BookBody::Subjects {
            subjects,
            selected,
            facts,
        } = people.body
        else {
            panic!("People lists subjects");
        };
        assert_eq!(selected, 1);
        assert_eq!(
            subjects[1],
            Subject {
                label: "Mr Corwin".into(),
                portrait: Some(1),
                count: 2
            }
        );
        assert_eq!(facts.len(), 2);

        t.press(
            0,
            &[
                Press::PageNext,
                Press::Down,
                Press::Right,
                Press::Right,
                Press::Right,
            ],
        );
        let timeline = t.session.book_screen(0, &t.book, &t.data).unwrap();
        assert_eq!(timeline.page, Page::Timeline);
        let BookBody::Timeline {
            rooms,
            counts,
            selected,
            facts,
            ..
        } = timeline.body
        else {
            panic!("Timeline is a grid");
        };
        assert_eq!(rooms, ["Drawing room", "Study", "Service corridor"]);
        assert_eq!(selected, 5, "one hour down, and held at the last room");
        assert_eq!(counts[4], 1, "the argument is at 19:00 in the study");
        assert!(facts.is_empty());
        t.press(0, &[Press::Left]);
        let BookBody::Timeline { facts, .. } =
            t.session.book_screen(0, &t.book, &t.data).unwrap().body
        else {
            panic!("Timeline is a grid");
        };
        assert_eq!(
            facts,
            ["Lady Ashby and Mr Corwin argued in the study at seven about a debt."]
        );
        t.press(0, &[Press::Up, Press::Up]);
        assert!(
            matches!(t.session.mode(0), Mode::Book { row: 1, .. }),
            "it stops at the top"
        );
    }

    #[test]
    fn the_guide_and_the_deduction_screen_say_what_to_do_next() {
        let mut t = Table::new();
        let step = |t: &Table| t.session.lesson(&t.book).map(|lesson| lesson.step);
        assert!(
            t.session
                .guide(0, &t.book, &t.data)
                .unwrap()
                .starts_with("First night, step 1 of 5: Find a fact"),
            "with nothing found, the guide says how facts are found"
        );
        t.fact("f_01");
        assert_eq!(step(&t), Some(2), "a fact found, the casebook is next");

        t.session.open_book(0);
        assert_eq!(
            t.session.guide(0, &t.book, &t.data),
            None,
            "a screen has its own hint"
        );
        assert_eq!(step(&t), Some(3));
        t.press(0, &[Press::PagePrev]);
        assert_eq!(step(&t), Some(4), "reaching Deductions is its own step");
        assert!(
            t.session
                .book_screen(0, &t.book, &t.data)
                .unwrap()
                .hint
                .starts_with("First night, step 4 of 5"),
            "inside the casebook the lesson rides on the hint line"
        );
        t.press(0, &[Press::Confirm]);
        let waiting = t.lines(0);
        assert!(
            waiting.contains(&"You: still choosing".to_string()),
            "{waiting:?}"
        );
        assert!(
            waiting.contains(&"Other player: still choosing".to_string()),
            "{waiting:?}"
        );
        t.press(0, &[Press::Confirm]);
        let locked = t.lines(0);
        assert!(locked.contains(&"You: LOCKED IN".to_string()), "{locked:?}");
        let other = {
            t.session.open_book(1);
            t.press(1, &[Press::PagePrev, Press::Confirm]);
            t.lines(1)
        };
        assert!(
            other.contains(&"Other player: LOCKED IN".to_string()),
            "{other:?}"
        );
        t.press(1, &[Press::Confirm]);
        assert!(
            t.lines(0).iter().any(|line| line.starts_with("Last try: ")),
            "both locks are judged and the verdict stays on screen"
        );

        t.session.show_card(1, Vec::new());
        assert!(
            t.lines(0).iter().any(|line| line.contains("holding RB")),
            "the first night's card says how to play"
        );
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

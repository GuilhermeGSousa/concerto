# Case schema, version 1

A case is a directory of thirteen RON files. `one-more-night-case`
(`examples/one-more-night/case`) parses them and resolves every reference
into a typed id. The crate depends on `serde` and `ron` only, so tools build it
without the engine.

The schema is frozen at the end of milestone M1. Until then it may change
without notice; after that, any change raises `version` in `case.ron`, and a
case written for another version fails to load.

The worked example is the placeholder case in
`examples/one-more-night/case/tests/fixture/case`. It has no story; it uses
every construct below once.

## Loading

```rust
let catalog = Catalog::from_level_manifest(&manifest_text)?
    .with_prop("prop_letter")
    .with_portrait("portrait_01")
    .with_activity("sit");
let case = Case::load_dir(Path::new("cases/test"), &catalog)?;
```

- `Case::load(&CaseFiles, &Catalog)` parses from memory. `CaseFiles::with_file`
  adds a file's text under its name without the extension, which is what a
  sealed bundle uses after it decrypts.
- `Case::load_dir` reads `<name>.ron` for each of the thirteen names and ignores
  anything else in the directory.
- All thirteen files are required. A file with no rows still holds its empty
  form: `[]` for a list, and empty lists inside `Terms(..)`, `Finale(..)`,
  `Doors(..)` and `Features(..)`.

## Ids

A file names things with strings. These rules cover all of them:

- **Ids the case defines** are unique within their kind: strings, characters,
  conversations, facts, terms, term banks, deductions, placements.
- **Ids the case borrows** come from the `Catalog` the game passes in: rooms,
  anchors, doors, keys and features from the level's `level_manifest.ron`,
  version 1; props, portraits and activities from the game.
- **Ids are opaque.** Debug output and errors print them, so an id must not
  describe what it names: `npc_04`, `f_037`, `conv_12`, not a name or a clue.
- **An id is 1 to 64 characters of letters, digits, `_`, `.` and `-`.**
  Anything else in an id's place, a sentence for instance, fails the load with
  an error that does not repeat it.

After loading, each kind has its own index type (`FactId`, `NpcId`, `RoomId`,
…). `case.facts.get(id)` returns the row, `case.facts.name(id)` the name the
file used, and `case.facts.id("f_037")` the id for a name. Catalog ids are read
the same way: `catalog.rooms.name(id)`.

## Errors

`CaseError` names a file, the kind of thing and its id. It carries nothing
else from a file:

| Error | Meaning |
|---|---|
| `MissingFile` | One of the thirteen files is absent. |
| `Unreadable` | The file exists and could not be read. |
| `Syntax` | The file is not valid RON or does not match the schema. Line and column only. |
| `UnsupportedVersion` | `case.ron` or the level manifest carries another version. |
| `NotAnId` | Something that is not shaped like an id sits where an id belongs. It is not shown. |
| `Duplicate` | Two rows of a kind share an id; or an NPC has two schedules or dialogues; or a door has two overrides; or a feature is set twice. |
| `Unknown` | A reference names something neither the case nor the catalog defines. |
| `Invalid` | One of the rules below is broken. |

The rules `Invalid` reports:

- A line of a conversation is spoken by one of its participants.
- A schedule names only conversations its character takes part in.
- The entries of one phase of a schedule are in strictly increasing order of
  `start`.
- A line's `seconds` are zero or more.
- A `facing` is a finite number, and an `earshot` is zero or more.
- A blank's answer is in its bank.
- A deduction's template has one run of three or more underscores for each
  blank.
- The finale stages a character once.
- Every feature of the level is on or off.

Everything else is the validator's job: that a conversation is started by
every participant at the same time, that schedules can be walked, that facts
can be found and that the case can be solved. So are repeats the loader lets
through: a deduction used by two finale statements, a term listed twice in a
bank, a door listed twice in `staff_opens`.

## Notation

- An unknown field is an error. A field marked *optional* may be left out.
- An optional value is written bare: `time: "19:30"`, not `time: Some("19:30")`.
- A time is `"HH:MM"`, from `"00:00"` to `"24:00"`. It loads as `GameTime`,
  minutes since midnight.
- `TimeWindow(from: "23:00", until: "24:00")` includes `from` and excludes
  `until`, and `until` is later than `from`. "After 23:00" ends at `"24:00"`,
  not `"00:00"`.
- A struct's name before its parentheses is optional in RON; when written it
  must be the name shown here.

## Files

### `case.ron`

```ron
Manifest(version: 1, title: "case.title", discovery: "21:00")
```

| Field | Type | Meaning |
|---|---|---|
| `version` | integer | The schema version, 1. |
| `title` | string id | The case's name on the title screen. |
| `discovery` | time, optional | When the discovery happens. From then on characters follow their `After` entries and `case.phase_at(time)` answers `After`. Left out, the night stays in `Before`. |

### `strings.ron`

One map from string id to text. Every other file refers to text by string id
and holds none itself, so this is the only file with readable prose.

```ron
{
    "case.title": "…",
    "npc_04.name": "…",
}
```

`case.strings.get(id).as_str()` gives the text. The `Debug` output of a loaded
`Case` prints string ids and hides the text.

### `characters.ron`

A list of `Character`. A character needs no schedule: someone only spoken of
is a character too.

| Field | Type | Meaning |
|---|---|---|
| `id` | id | The character's id. |
| `name` | string id | The name shown in the casebook. |
| `portrait` | portrait id | The casebook portrait. |
| `questioned_by` | `Guest` or `Staff` | Which player may talk to them. |

### `schedules.ron`

A list of `Schedule`, at most one per character.

| Field | Type | Meaning |
|---|---|---|
| `npc` | character id | Whose schedule it is. |
| `entries` | list of `ScheduleEntry` | Within a phase, in order of `start`. |

`ScheduleEntry`:

| Field | Type | Meaning |
|---|---|---|
| `start` | time | When the NPC is at the anchor. |
| `anchor` | anchor id | Where. |
| `activity` | activity id | The idle set played there. |
| `facing` | degrees, optional | Replaces the anchor's own facing. |
| `phase` | `Before` or `After` | Before or after the discovery. |
| `critical` | bool, optional, false | The NPC is busy and cannot be interrupted. |
| `conversation` | conversation id, optional | The conversation that starts with this entry. |
| `rings` | list of fact ids, optional | When present, even empty, the character rings the bell of the anchor's room as the entry starts. The flag stays up on the bell board until the Staff reads it, and reading it grants these facts. |

A conversation starts when the entries that name it start. Each participant's
schedule should name it at the same time; the loader checks only that whoever
names it takes part.

### `conversations.ron`

A list of `Conversation`: what players overhear.

| Field | Type | Meaning |
|---|---|---|
| `id` | id | The conversation's id. |
| `participants` | list of character ids | Who takes part. |
| `private` | bool, optional, false | It breaks off while the Guest is in sight. |
| `earshot` | metres, optional | Replaces the game's default earshot radius. |
| `lines` | list of `SpokenLine` | The conversation, in order. |
| `cover` | list of `SpokenLine`, optional | What a private conversation switches to while the Guest is in sight. Empty means it stops. |

`SpokenLine`:

| Field | Type | Meaning |
|---|---|---|
| `speaker` | character id | Who says it. In a conversation, a participant. |
| `text` | string id | The subtitle. |
| `seconds` | number | How long the line stays up, in seconds of the game clock. |
| `grants` | list of fact ids, optional | Facts a player gets by hearing the line. |

### `dialogue.ron`

A list of `Dialogue`, at most one per character.

| Field | Type | Meaning |
|---|---|---|
| `npc` | character id | Who answers. |
| `brush_off` | string id | The one line said during a critical schedule entry. |
| `no_answer` | string id | The line for a fact or person the NPC has no topic for. |
| `topics` | list of `Topic` | What the NPC can be asked. |

`Topic`:

| Field | Type | Meaning |
|---|---|---|
| `ask` | `Question(string id)`, `Fact(fact id)`, `Person(character id)` or `Item(placement id)` | A fixed question with its prompt, an "Ask about…" entry, or an item shown: an `Item` topic is offered only while the asking player carries that placement. |
| `responses` | list of `Response` | The first whose condition holds is given. A topic with none that holds is not offered. |

`Response`:

| Field | Type | Meaning |
|---|---|---|
| `when` | `Condition`, optional | Left out, the response always holds. |
| `lines` | list of string ids | What the NPC says. |
| `grants` | list of fact ids, optional | Facts the answer gives. |

`Condition`, every field optional, all of them must hold:

| Field | Type | Meaning |
|---|---|---|
| `window` | `TimeWindow` | The clock is inside it. |
| `phase` | `Before` or `After` | The NPC is in this phase. |
| `knows` | list of fact ids | The players have found all of them. |

### `facts.ron`

A list of `Fact`.

| Field | Type | Meaning |
|---|---|---|
| `id` | id | The fact's id. |
| `text` | string id | The casebook line. |
| `category` | `Person`, `Place`, `Time`, `Object` or `Event` | The casebook category. |
| `source` | `Seen`, `Heard`, `Read` or `Told` | How it is found. |
| `people` | list of character ids, optional | Who it is about. |
| `place` | room id, optional | Where; with `time`, its cell on the timeline. |
| `time` | time, optional | When. |
| `object` | placement id, optional | The placed prop it is about. |
| `unlocks_terms` | list of term ids, optional | Terms it adds to the word banks. |

A fact says what it is, not where it is found. What grants it is a line of a
conversation, a dialogue response, a hotspot or a document.

### `terms.ron`

```ron
Terms(
    terms: [Term(id: "t_01", text: "t_01.text")],
    banks: [TermBank(id: "bank_people", terms: ["t_01"])],
)
```

| Field | Type | Meaning |
|---|---|---|
| `Term.id` | id | The term's id. |
| `Term.text` | string id | The word shown in a blank. |
| `TermBank.id` | id | The bank's id. |
| `TermBank.terms` | list of term ids | Every term the bank can ever offer. A player sees those a found fact has unlocked. |

### `deductions.ron`

A list of `Deduction`. The finale's statements are deductions too.

| Field | Type | Meaning |
|---|---|---|
| `id` | id | The deduction's id. |
| `template` | string id | The sentence, with `___` (three or more underscores) for each blank in order. |
| `blanks` | list of `Blank(bank, answer)` | For each blank, the bank it picks from and the right term, which must be in that bank. |
| `appears_with` | list of fact ids, optional | It appears in the casebook once any of these is found. |
| `prerequisite_for_finale` | bool, optional, false | The finale needs it confirmed. |
| `unlocks` | list of deduction ids, optional | Deductions that appear when this one is confirmed. |

When a deduction shows in the casebook:

- One with `appears_with`: once any of those facts is found.
- One another deduction `unlocks`: once that one is confirmed. With
  `appears_with` as well, whichever comes first.
- One with neither: from the start, unless it is a finale statement.
- A finale statement with neither shows only in the finale.

### `finale.ron`

```ron
Finale(
    staging: [Staging(npc: "npc_01", anchor: "drawing_05")],
    statements: [Statement(deduction: "d_09", reaction: [SpokenLine(…)])],
    ending: ["finale.end_1"],
)
```

| Field | Type | Meaning |
|---|---|---|
| `staging` | list of `Staging(npc, anchor)` | Where each character stands. |
| `statements` | list of `Statement` | Answered in order. |
| `Statement.deduction` | deduction id | The sentence to complete. |
| `Statement.reaction` | list of `SpokenLine`, optional | What the room says when it is right. |
| `ending` | list of string ids | The ending text, one entry per card. |

### `placements.ron`

A list of `Placement`: a prop of the library put on an anchor.

| Field | Type | Meaning |
|---|---|---|
| `id` | id | The placement's id. |
| `prop` | prop id | Which prop. |
| `anchor` | anchor id | Where it sits. |
| `name` | string id | Its name in prompts and in the casebook. |
| `interaction` | see below | What a player can do with it. |

`interaction` is one of:

| Value | Meaning |
|---|---|
| `Scenery` | Nothing. |
| `Inspect(hotspots: [Hotspot(label: string id, grants: [fact ids])])` | Brought up to the camera; each hotspot has a label and may grant facts. |
| `Document(pages: [string ids], grants: [fact ids])` | Read in full; reading grants the facts. |
| `Carry(key: key id)` | Picked up and carried; with `key`, it is that key. |

`grants` and `key` are optional. An item that is no key is `Carry()`, with the
parentheses.

### `doors.ron`

```ron
Doors(
    staff_opens: ["d_pantry"],
    overrides: [DoorOverride(door: "d_study", state: Locked, locked: [TimeWindow(from: "23:00", until: "24:00")])],
)
```

| Field | Type | Meaning |
|---|---|---|
| `staff_opens` | list of door ids, optional | Doors the Staff key ring opens. |
| `overrides` | list of `DoorOverride`, optional | At most one per door. |

`DoorOverride`; a field left out keeps the level's value:

| Field | Type | Meaning |
|---|---|---|
| `door` | door id | Which door. |
| `enabled` | bool | False removes the connection for this case. |
| `state` | `Open`, `Closed` or `Locked` | The state each loop starts with. |
| `key` | key id | The key that opens it. |
| `zone` | `Public`, `Service` or `Private` | Who may pass: anyone, the Staff only, or by who is in the room, as [access-design.md](access-design.md) sets out. |
| `locked` | list of `TimeWindow` | Spans in which it is locked whatever its state. Left out, the case adds none. |

`key` can replace a door's key and cannot remove it.

### `features.ron`

```ron
Features(on: ["F02"], off: ["F01"])
```

The level's special features, each built into the level and switched by the
case. Every feature in the level manifest is in exactly one of the two lists,
so no default says which ones matter. `case.features.is_on(id)` answers for
one.

## Not in version 1

Each of these is left to the issue that builds the system, which adds it here
before the freeze if it turns out to be case data:

- Anything about the dumbwaiter.
- Removing a door's key, and a door that is always open.

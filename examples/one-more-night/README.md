# One More Night

A couch co-op mystery for two: one night in a house, 18:00 to 00:00, played
again and again. Only what the players learn survives the night.

```sh
cargo run -p one-more-night
cargo run -p one-more-night -- --manor
cargo run -p one-more-night-editor -- --project <scratch copy of examples/one-more-night>
```

The game plays the open test case in `cases/test` on a graybox test level,
`content/level/test_level.gasset`: a drawing room, a study and a service
corridor. `cases/test/README.md` has the case and its solution. The characters
are figurines built in `assets/characters`, whose
[README](assets/characters/README.md) describes them; they blend between
standing, walking and sitting, and turn their heads to a player who is talking
to them.

`-- --manor` loads the manor's greybox, all five levels, and the art over it,
which dresses every room, with an open case
that solves nothing (`cases/manor_walk`): ten characters, one of each
figurine, standing about the house to be met, and no props or facts. The Guest starts
upstairs and the Staff in the servants' hall. Its walls, floors and furniture
collide, its doors open, and its bells, bell board, beds, dumbwaiter stops
and the dinner gong in the hall are found by the names of the nodes in the
export, with nothing placed by hand. A door leaf that stands open is walked through and does not block the
way; aim at it to close it. Every indoor room has ceiling lamps, placed from the
room plan: warm in the guest rooms, paler in the service ones, hung lower under
the cellar's and attic's low ceilings. Every lamp casts
shadows; only the lamps a player's camera sees are lit and shadowed. Outdoors there is only moonlight.

### The sealed case

The real case is never in the repository and never on disk in the clear. It is
one file, `sealed/case.sealed`: the case's files compressed with zstd and
encrypted with ChaCha20-Poly1305 under the key in `sealed/case.key`. Git
ignores both. `-- --manor` plays it when both are there, opening it in memory
only, and saves to `saves/manor`; without them it plays the open walk. If the
bundle does not open or load, the game says so in one line that tells nothing
of what is inside, and plays the open walk.

```sh
cargo run -p one-more-night --bin seal-case -- <case directory>
```

`seal-case` checks that the case loads over the manor and writes the bundle,
making a key if there is none. It prints PASS, or FAIL with the file and the
id that did not load, and nothing else about the case. `ONE_MORE_NIGHT_CASE` and `ONE_MORE_NIGHT_KEY` name other paths for the
two files. This keeps the case from being read by accident; it does not stop
someone who sets out to open it with the key.

## Playing

The window is split in two. A pad joins by pressing A (South) or Start; the
keyboard and mouse can stand in for one player and join with Enter or Space.
The first player is the Guest and the second the Staff; the night card lets
them swap.

| Pad | Keyboard | What it does |
|---|---|---|
| Left stick | WASD | Walk; a full push hurries, and keys push fully, so hold left Shift to walk |
| Right stick | Mouse | Look |
| X | E | Use what the dot in the middle of your half points at |
| B | Q | Put down what you carry; leave a screen |
| Y | Tab | Open and close the casebook |
| A | Enter | Pick the highlighted row; go on |
| D-pad | Arrow keys | Move on a screen |
| LB / RB | Page Up / Page Down | Turn the casebook's pages |
| LB, held | C | Pocket watch; when every player holds theirs, time runs twelve times faster, unless someone is overhearing a conversation |
| RB, held | R | The Staff listens at a door they stand by |
| Start | Escape | Pause for both players; it also frees the mouse |

- **Looking a prop over** brings it up in front of you: the right stick turns
  it, the left stick moves it nearer and farther, and the d-pad steps through
  its details.
- **You can see each other.** Each player has a body, by role: the Guest in a
  navy suit, the Staff as a maid. It follows the roles when they are swapped.
- **The first night is a lesson.** Its card lists the basics, and five steps
  follow what you have done: find a fact, open the casebook, reach
  Deductions, lock in an answer, confirm one together. The step shows under
  your role while you walk and on the casebook's hint line while it is open,
  and the lesson ends with the first confirmed deduction. When the Staff
  stands at a door with talk behind it, the line says to hold RB.
- **Pausing shows the controls** to both players.
- **Facts** are found by looking a prop over, reading, overhearing and asking.
  Both players get a fact at once, and it is never lost.
- **The casebook** has People, Timeline, Places, Objects and Deductions.
  People, Places and Objects list their subjects on the left, with the number
  of facts about each, and the facts about the chosen one on the right. The
  Timeline is a grid of rooms across and hours down; the d-pad moves cell by
  cell and the facts of the chosen cell show under it. A
  deduction is a sentence with blanks; each player fills their own copy and
  locks it, and when both are locked the casebook says whether they agree and
  whether it is right. A player alone judges their own copy.
- **The Guest** talks to guests, and a private conversation changes subject
  while the Guest is in the room. **The Staff** talks to staff, opens the doors
  on the key ring, and hears through a door. Service doors refuse the Guest.
- **Private rooms** follow who is in them. The Guest is let into someone's room
  only while they are in; the Staff goes in only while it is empty, or when its
  bell has rung. Your own room is always yours, and nobody is kept from
  leaving. [docs/access-design.md](docs/access-design.md) has the whole rule.
- **Props**: one item is carried at a time. The dumbwaiter has three stops;
  using a stop calls the car, and then sends or takes an item.
- **Bells**: a bell pull raises its room's flag on the board in the corridor
  and tells the Staff; the Staff reading the board drops the flags. Characters
  ring too, at times the case sets, and a flag they raised tells the Staff who
  reads it a fact.
- **Showing an item**: a character may have something to say about what you
  carry; the topic appears when you talk to them with it in hand.
- **The night ends** at midnight, or when both players lie in their own beds.
  The house goes back to six o'clock and the next night's card lists what was
  learned. The game saves then, in `saves/` under the working directory or in
  `ONE_MORE_NIGHT_SAVES`.
- **The gong** gathers the house once the deductions the finale needs are
  confirmed. The players answer its statements in order, each a joint lock; a
  wrong answer ends the night, and the whole chain ends the game.

## Debug keys

Built in by the `debug` feature: `cargo run -p one-more-night --features debug`.

| Key | What it does |
|---|---|
| F5 | Moves the clock on half an hour |
| F6 | Holds and releases the clock |
| F7 | Grants the next fact not yet found |
| F8 | Ends the night |
| F9 | Confirms the next open deduction |
| F10 | Brings the first inspectable prop up for the first player |

With a sealed case loaded, F7, F9 and F10 do nothing but say so: they would
give the case away.

With the `debug` feature, setting `ONE_MORE_NIGHT_NO_CARD` starts the night
without its card, for looking at a level without a pad in hand, and
`ONE_MORE_NIGHT_START=19:50` starts the first night at that time.

## Code

| Module | What it holds |
|---|---|
| `casefile`, `level` | The case and its level manifest, loaded into ids, rooms, anchors and routes |
| `book` | Found facts, unlocked terms, deductions and their joint lock |
| `schedule` | Where a schedule puts a character at a time; which line of a conversation is being said |
| `rules` | Dialogue topics on offer; who may pass a door |
| `session` | Each player's screen as a state machine over button presses |
| `book_ui` | The casebook drawn as tabs, lists and the timeline grid, and every other screen and HUD line |
| `house` | Doors, items, bells, the dumbwaiter, beds and the gong; ending a night |
| `save` | The save file and its backups |
| `seal` | The sealed case bundle: sealing it and opening it in memory |
| `world` | The systems that run all of it: fixtures, characters, props, targeting, the HUD and the loop |
| `players`, `controls`, `clock` | Split-screen seats, the actions, the night clock |

The level scene holds collision boxes, lights, spawn points and fixtures, and
no meshes; what is seen comes from `assets/art`, whose
[README](assets/art/README.md) has the art direction and how it is built. The
level's doors, bell pulls, bell board, gong, beds and dumbwaiter stops are
scene entities with a `Fixture` component, placed in the editor. A fixture's
`name` is the door or room id, the bed's player, or the stop's number.

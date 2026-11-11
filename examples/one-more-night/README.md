# One More Night

A couch co-op mystery for two: one night in a house, 18:00 to 00:00, played
again and again. Only what the players learn survives the night.

```sh
cargo run -p one-more-night
cargo run -p one-more-night-editor -- --project <scratch copy of examples/one-more-night>
```

The game plays the open test case in `cases/test` on a graybox test level,
`content/level/test_level.gasset`: a drawing room, a study and a service
corridor. `cases/test/README.md` has the case and its solution. The characters
are the tech demo's UAL1 model, as placeholders.

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
| LB, held | C | Pocket watch |
| RB, held | R | The Staff listens at a door they stand by |
| Start | Escape | Pause for both players; it also frees the mouse |

- **Facts** are found by looking a prop over, reading, overhearing and asking.
  Both players get a fact at once, and it is never lost.
- **The casebook** has People, Timeline, Places, Objects and Deductions. A
  deduction is a sentence with blanks; each player fills their own copy and
  locks it, and when both are locked the casebook says whether they agree and
  whether it is right. A player alone judges their own copy.
- **The Guest** talks to guests, and a private conversation changes subject
  while the Guest is in the room. **The Staff** talks to staff, opens the doors
  on the key ring, and hears through a door. Service doors refuse the Guest.
- **Props**: one item is carried at a time. The dumbwaiter has three stops;
  using a stop calls the car, and then sends or takes an item.
- **Bells**: a bell pull raises its room's flag on the board in the corridor
  and tells the Staff; the Staff reading the board drops the flags.
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

## Code

| Module | What it holds |
|---|---|
| `casefile`, `level` | The case and its level manifest, loaded into ids, rooms, anchors and routes |
| `book` | Found facts, unlocked terms, deductions and their joint lock |
| `schedule` | Where a schedule puts a character at a time; which line of a conversation is being said |
| `rules` | Dialogue topics on offer; who may pass a door |
| `session` | Each player's screen as a state machine over button presses |
| `house` | Doors, items, bells, the dumbwaiter, beds and the gong; ending a night |
| `save` | The save file and its backups |
| `world` | The systems that run all of it: fixtures, characters, props, targeting, the HUD and the loop |
| `players`, `controls`, `clock` | Split-screen seats, the actions, the night clock |

The level's doors, bell pulls, bell board, gong, beds and dumbwaiter stops are
scene entities with a `Fixture` component, placed in the editor. A fixture's
`name` is the door or room id, the bed's player, or the stop's number.

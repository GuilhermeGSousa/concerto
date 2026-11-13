# One More Night — Level Spec v2

Nov 7, 2026 · @Gui

## Purpose and rules

This spec defines the manor before any case exists, so nothing in it can point at the solution. The sealed case is written afterwards to fit the frozen level, and it can only use or switch off what is here.

- **All of it is open to Gui.** The authoring chat works from the exported level manifest and never edits this doc.
- **Everything is built and the case switches it.** Every special feature (see Feature list) gets built. The case must set each one on or off, and the loader refuses a case that leaves any feature out, so no default reveals anything.
- **More than any case needs.** Anchors, props and features all outnumber what one case can use.
- **Freeze.** Once the greybox passes the checklist at the end, tag `level-v1`. After the freeze, these changes are free: cosmetic geometry, moving anchors up to 0.5 m, travel-time changes within ±10%, and adding anchors or props. Removing or renaming anything needs a request from the authoring side.
- **Change requests** from the authoring side arrive generic and padded with decoys, at most one batch per milestone.

**Count:** 40 room volumes on five levels: the GDD's space list, plus corridors and stairs, four small extra rooms (lamp and boot room, housemaid's closet, box room, potting shed), and the outdoors split into separate areas. The "22 spaces" figure in the GDD should become 40.

## Scale and metrics

The house is compact: any two public rooms indoors are at most about 30 seconds' walk apart, which is 10 minutes of the night. All units are metres on a 0.5 m grid.

**Coordinates.** The origin is the south-west corner of the main block, at ground-floor level. +X is east, +Y is north, +Z is up, which is Blender's default; the glTF export converts the axes. Plan dimensions are measured on wall centre lines.

| Item | Value |
| --- | --- |
| Floor levels | Cellar −3.0, ground 0.0, first +3.8, attic +7.2 |
| Ceiling heights | Cellar 2.4, ground 3.4, first 3.0, attic 2.4 in the corridor falling to 1.2 at the eaves |
| Hall | South half (y 0–8) is double height, 7.2, overlooked by the first-floor gallery |
| Walls | Exterior 0.4, interior 0.2 |
| Openings (w × h) | Door 1.0 × 2.2, double door 1.8 × 2.6, French window 1.6 × 2.4, service and baize door 0.9 × 2.1, garden gate 1.2 wide, garage door 2.8 × 2.4 |
| Corridors | 2.0 between centre lines (about 1.8 clear) on every floor |
| Main stairs | U-shaped with a half-landing, 1.4 wide, rise 0.18, going 0.28 |
| Back stairs | Dog-leg from cellar to attic, 1.0 wide, rise 0.19, going 0.24 |
| Player capsule | Radius 0.3, height 1.75, eye height 1.62 |
| NPC capsule | Radius 0.3, height 1.6–1.9 |
| Speeds | Player 1.4 m/s walking, 2.4 m/s hurrying; NPC 1.3 m/s; all ×0.75 on stairs |
| Interact ray | 2.0 |
| Earshot | 6 m, same room (GDD) |

Stairs collide as ramps. NPCs walk a little slower than players, so a player can follow one.

**Walk budget.** These times come from path lengths measured on the plans. Measure the real ones in the greybox. The freeze needs every indoor public route at 30 s or less, and every route at 45 s or less. One real second is 20 seconds of the night.

| Route | Path (m) | Walk (s) | Night (min) |
| --- | --- | --- | --- |
| Drawing room to dining room, through the hall | 19 | 14 | 5 |
| Conservatory to billiard room, along the corridors | 31 | 22 | 7 |
| Hall to guest bedroom 1, main stairs | 26 | 21 | 7 |
| Servants' hall to guest bedroom 2, back stairs | 29 | 23 | 8 |
| Conservatory to guest bedroom 1 (longest public route) | 38 | 30 | 10 |
| Far corner of the walled garden to the hall | 28 | 20 | 7 |
| Kitchen to garage | 26 | 19 | 6 |
| Servants' hall to the Staff player's attic room | 38 | 33 | 11 |

The back stairs are the servants' shortcut: from the service wing they reach the east bedrooms in about half the distance of the main stairs.

## Floor plans

The house has four parts:

- the main block, 30 × 16 m on two floors with an attic over its east half
- a single-storey service wing to the east, with a back-stair tower running from cellar to attic
- a glazed conservatory to the west
- the grounds, to the north and east

The tables give each volume's bounds in plan coordinates. Every bedroom also has a bed, wardrobe, dressing table, writing table, fireplace and bell pull.

### Site

&#91;embedded content: site plan · to scale, north up\]

The garage can be reached from the yard and from the garden path, and the dashed route links the front door to the terrace without going through the house.

| Id | Space | x | y | Size | Zone | Fixtures |
| --- | --- | --- | --- | --- | --- | --- |
| `forecourt` | Forecourt and drive | −8 to 52 | −10 to 0 | 60 × 10 | Open | Front door; the drive runs east into the yard |
| `terrace` | Terrace | −8 to 30 | 16 to 20 | 38 × 4 | Public | Steps down to the garden at x 13; doors from the conservatory, library, hall and billiard room |
| `garden` | Walled garden | −6 to 24 | 20 to 36 | 30 × 16 | Public | Gravel paths, sundial, lily pond, benches, cold frames; gate in the east wall |
| `shed` | Potting shed | −4 to 0 | 31 to 35 | 4 × 4 | Public | Bench, tools, tins on shelves (F12) |
| `path` | Garden path | 24 to 44 × 29 to 31, and 42 to 44 × 18 to 31 |  | L-shaped | Public | Links the garden gate, the garage side door and the yard |
| `garage` | Garage | 44 to 52 | 18 to 26 | 8 × 8 | Service | Car, workbench, chauffeur's corner |
| `yard` | Service yard | 42 to 52 | 0 to 18 | 10 × 18 | Open | Back door of the service wing, garage doors; open to the drive |

### Ground floor

&#91;embedded content: ground floor plan · to scale, north up\]

The service wing is reached through the baize door at the east end of the corridor, or from the dining room through the pantry.

| Id | Space | x | y | Size | Zone | Fixtures |
| --- | --- | --- | --- | --- | --- | --- |
| `hall` | Entrance hall | 11 to 19 | 0 to 16 | 8 × 16 | Public | Grandfather clock, dinner gong, telephone alcove; main stairs at x 15.5–19, y 10–16; south half double height |
| `drawing` | Drawing room | 0 to 11 | 0 to 8 | 11 × 8 | Public | Finale stage, gramophone, fireplace, bell pull |
| `dining` | Dining room | 19 to 27 | 0 to 8 | 8 × 8 | Public | Table for 10, sideboard, serving hatch, bell pull |
| `corr_w` | West corridor | 0 to 11 | 8 to 10 | 11 × 2 | Public | Archway to the hall |
| `corr_e` | East corridor | 19 to 30 | 8 to 10 | 11 × 2 | Public | Archway to the hall; baize door at the east end |
| `library` | Library | 0 to 7 | 10 to 16 | 7 × 6 | Public | Shelves on three walls, reading chairs, French window, bell pull |
| `study` | Study | 7 to 11 | 10 to 16 | 4 × 6 | Private | Desk with telephone extension, wall safe, fireplace, window onto the terrace, bell pull |
| `billiard` | Billiard room | 19 to 30 | 10 to 16 | 11 × 6 | Public | Billiard table, card table, cue rack, French window, bell pull |
| `conservatory` | Conservatory | −8 to 0 | 6 to 16 | 8 × 10 | Public | Glazed; wicker chairs, plants; doors to the drawing room, corridor and terrace; bell pull |
| `pantry` | Butler's pantry | 27 to 30 | 0 to 8 | 3 × 8 | Service | Serving hatch, dumbwaiter, key cupboard, silver cupboard, speaking tube |
| `kitchen` | Kitchen | 30 to 37 | 0 to 8 | 7 × 8 | Service | Range, table, sink, dumbwaiter |
| `servhall` | Servants' hall | 37 to 42 | 0 to 8 | 5 × 8 | Service | Bell board, long table; Staff player start |
| `passage` | Service passage | 30 to 42 | 8 to 10 | 12 × 2 | Service | Back door at the east end |
| `backstairs` | Back stairs | 30 to 33 | 10 to 16 | 3 × 6 | Service | One volume through every level; the tower rises above the wing |
| `housekeeper` | Housekeeper's room | 33 to 37 | 10 to 16 | 4 × 6 | Service | Desk, linen press, still-room cupboard |
| `lamproom` | Lamp and boot room | 37 to 42 | 10 to 16 | 5 × 6 | Service | Lamp bench, boots, coats; staff collect the lamps here |

### First floor (+3.8)

&#91;embedded content: first floor plan · to scale, north up\]

The Guest player's room is the first door east of the landing. The Staff come up by the back stairs and through the baize door at the east end of the gallery.

| Id | Space | x | y | Size | Zone | Fixtures |
| --- | --- | --- | --- | --- | --- | --- |
| `gallery` | Landing and gallery | 0 to 30 × 8 to 10, and 11 to 19 × 10 to 16 |  | 2 × 30 + landing | Public | Main stairs arrive; balustrade over the hall; dumbwaiter hatch at the east end |
| `master` | Master bedroom | 0 to 7 | 0 to 8 | 7 × 8 | Private | Speaking tube |
| `dressing` | Dressing room | 7 to 11 | 0 to 8 | 4 × 8 | Private | Wardrobes; its own door to the gallery |
| `g1` | Guest bedroom 1 | 19 to 24.5 | 0 to 8 | 5.5 × 8 | Private | Connecting door to g2 |
| `g2` | Guest bedroom 2 | 24.5 to 30 | 0 to 8 | 5.5 × 8 | Private | Dumbwaiter shaft cuts the north-east corner |
| `g3` | Guest bedroom 3 | 0 to 5.5 | 10 to 16 | 5.5 × 6 | Private | Connecting door to g4 |
| `g4` | Guest bedroom 4 | 5.5 to 11 | 10 to 16 | 5.5 × 6 | Private | Fireplace on the study's flue |
| `g5` | Guest bedroom 5 | 19 to 24.5 | 10 to 16 | 5.5 × 6 | Private | The Guest player's room: start point and bed |
| `bath` | Bathroom | 24.5 to 27 | 10 to 16 | 2.5 × 6 | Private | Bolts from inside |
| `closet` | Housemaid's closet | 27 to 30 | 10 to 16 | 3 × 6 | Service | Linen shelves, hot-water cans, sink |
| `backstairs` | Back-stair lobby | 30 to 33 | 8 to 10 | 3 × 2 | Service | Baize door to the gallery |

### Attic (+7.2) and cellar (−3.0)

&#91;embedded content: attic and cellar plans · to scale, north up\]

The attic covers only the east half of the main block, and the back stairs are the only way up or down.

| Id | Space | x | y | Size | Zone | Fixtures |
| --- | --- | --- | --- | --- | --- | --- |
| `attic_corr` | Attic corridor | 19 to 30 | 8 to 10 | 11 × 2 | Service | Under the ridge |
| `attic_maids` | Maids' room | 19 to 24.5 | 3 to 8 | 5.5 × 5 | Service | Two beds |
| `attic_men` | Menservants' room | 24.5 to 30 | 3 to 8 | 5.5 × 5 | Service | Two beds |
| `attic_staff` | Staff player's room | 19 to 24.5 | 10 to 14 | 5.5 × 4 | Service | Staff player's bed |
| `boxroom` | Box room | 24.5 to 30 | 10 to 14 | 5.5 × 4 | Service | Guests' trunks and cases (F13) |
| `cellar_pass` | Cellar passage | 30 to 33 | 8 to 10 | 3 × 2 | Service | Foot of the back stairs |
| `cellar` | Wine cellar | 27 to 37 | 0 to 8 | 10 × 8 | Service | Racks, a cask table; locked door |

The back-stair lobby also sits at 30–33 × 8–10 on the attic floor. Who sleeps in which attic bed is up to the case.

## Feature list

Fourteen special features, all built into the greybox. The case turns each one on or off, so Gui knows they exist but not which ones matter. They're all standard country-house devices, and each one works through systems the game already has, or through one of the two small additions noted under the table.

| Id | Feature | Where | In play | When off |
| --- | --- | --- | --- | --- |
| F01 | Serving hatch | Dining room ↔ butler's pantry (x 27, y 4) | Passes one item. While open, talk in the dining room carries into the pantry | Bolted shut |
| F02 | Gallery overlook | Gallery balustrade over the hall's double-height half | Talk in the south half of the hall carries up to the balustrade | Sound doesn't carry |
| F03 | Connecting door | Guest bedrooms 1 ↔ 2 | A working door between two bedrooms | Locked, no key exists |
| F04 | Connecting door | Guest bedrooms 3 ↔ 4 | Same as F03 | Locked, no key exists |
| F05 | Jib door | Library bookcase ↔ study (x 7, y 13) | Hidden door. Inspecting the bookcase reveals it; after that it works as a normal door | Just a bookcase, with no hotspot |
| F06 | Shared flue | Study fireplace ↔ guest bedroom 4 fireplace (x 10, y 16) | Talk in the study carries to the hearth upstairs | Flue blocked |
| F07 | Telephone extension | Hall telephone alcove ↔ study desk | A call made on one telephone can be heard by lifting the other | Extension disconnected |
| F08 | Speaking tube | Master bedroom ↔ butler's pantry | Speech at one mouthpiece is heard at the other | Tube capped |
| F09 | French window | Library ↔ terrace | An outside route out of the library | Locked |
| F10 | French window | Billiard room ↔ terrace | An outside route out of the billiard room | Locked |
| F11 | Study window | Study north wall ↔ terrace (x 8, y 16) | From the terrace, the study can be seen into and, with the window open, heard | Curtains drawn, window shut |
| F12 | Potting shed | North-west corner of the walled garden | Small outbuilding with a lock: tools, tins, a bench | Locked, no key exists |
| F13 | Box room | Attic, north-east | Guests' luggage, lockable | Locked, no key exists |
| F14 | Garage side door | Garage ↔ garden path (x 44, y 23) | A second way into the garage, from the garden | Locked |

The fixed co-op devices from the GDD aren't features and are always on. The dumbwaiter has stops in the kitchen, the pantry and at the east end of the gallery, with its shaft at x 29.1–29.9, y 7.1–7.9 on both floors. The bell board is in the servants' hall.

**New game work implied** (both small, neither in the GDD yet):

- **Acoustic links** for F01, F02, F06, F07, F08 and F11. A link is a pair of points: speech within a radius of the source end is heard by a player standing at the listening end, as if they were in earshot. Each link has an on flag set by the case.
- **Concealed doors** for F05. A door flagged `concealed` renders as its cover (a bookcase) and gets an inspect hotspot. Once revealed, it is an ordinary door until the loop resets.

## Door list

There are 53 openings. The case can change any door's default state, lock times, key and zone, but it cannot add a door. Positions are the centre of the opening on the wall line. The pantry's key cupboard holds a copy of every key; which keys the Staff key ring carries is case data.

| Id | Between | At (x, y) | Kind | Zone | Default | Key |
| --- | --- | --- | --- | --- | --- | --- |
| `d_front` | Hall ↔ forecourt | 15, 0 | Double | Public | Closed |  |
| `d_hall_garden` | Hall ↔ terrace | 13, 16 | Glazed double | Public | Closed |  |
| `d_hall_drawing` | Hall ↔ drawing room | 11, 4 | Double | Public | Open |  |
| `d_hall_dining` | Hall ↔ dining room | 19, 4 | Double | Public | Open |  |
| `a_hall_w` | Hall ↔ west corridor | 11, 9 | Archway | Public | Always open |  |
| `a_hall_e` | Hall ↔ east corridor | 19, 9 | Archway | Public | Always open |  |
| `d_drawing_corr` | Drawing room ↔ west corridor | 6, 8 | Door | Public | Closed |  |
| `d_drawing_cons` | Drawing room ↔ conservatory | 0, 7 | Glazed double | Public | Closed |  |
| `d_cons_corr` | Conservatory ↔ west corridor | 0, 9 | Glazed | Public | Closed |  |
| `d_cons_terrace` | Conservatory ↔ terrace | −4, 16 | Glazed double | Public | Closed |  |
| `d_library_corr` | Library ↔ west corridor | 3.5, 10 | Door | Public | Closed |  |
| `d_library_terrace` | Library ↔ terrace (F09) | 3.5, 16 | French window | Public | Closed |  |
| `d_study_corr` | Study ↔ west corridor | 9, 10 | Door | Private | Locked | `k_study` |
| `d_jib` | Library ↔ study (F05) | 7, 13 | Concealed | Private | Closed |  |
| `d_dining_corr` | Dining room ↔ east corridor | 23, 8 | Door | Public | Closed |  |
| `d_dining_pantry` | Dining room ↔ pantry | 27, 6.5 | Service | Service | Closed |  |
| `h_serving` | Dining room ↔ pantry (F01) | 27, 4 | Hatch | Service | Closed |  |
| `d_billiard_corr` | Billiard room ↔ east corridor | 24, 10 | Door | Public | Closed |  |
| `d_billiard_terrace` | Billiard room ↔ terrace (F10) | 24.5, 16 | French window | Public | Closed |  |
| `d_pantry_corr` | Pantry ↔ east corridor | 28.5, 8 | Service | Service | Closed |  |
| `d_baize_0` | East corridor ↔ service passage | 30, 9 | Baize | Service | Closed |  |
| `d_kitchen` | Kitchen ↔ service passage | 33.5, 8 | Service | Service | Open |  |
| `d_kitchen_servhall` | Kitchen ↔ servants' hall | 37, 4 | Service | Service | Open |  |
| `d_servhall` | Servants' hall ↔ service passage | 39.5, 8 | Service | Service | Open |  |
| `d_housekeeper` | Housekeeper's room ↔ service passage | 35, 10 | Door | Service | Closed | `k_housekeeper` |
| `d_lamproom` | Lamp room ↔ service passage | 39.5, 10 | Service | Service | Closed |  |
| `d_backstairs_0` | Back stairs ↔ service passage | 31.5, 10 | Service | Service | Open |  |
| `d_back` | Service passage ↔ yard | 42, 9 | Door | Service | Closed |  |
| `d_garage_yard` | Garage ↔ yard | 48, 18 | Garage | Service | Closed |  |
| `d_garage_side` | Garage ↔ garden path (F14) | 44, 23 | Door | Service | Closed |  |
| `g_garden_e` | Walled garden ↔ garden path | 24, 30 | Gate | Public | Closed |  |
| `s_terrace_garden` | Terrace ↔ walled garden | 13, 20 | Steps | Public | Always open |  |
| `d_shed` | Potting shed ↔ walled garden (F12) | 0, 33 | Door | Public | Closed | `k_shed` |
| `d_master` | Master bedroom ↔ gallery | 3.5, 8 | Door | Private | Closed | `k_master` |
| `d_master_dressing` | Master bedroom ↔ dressing room | 7, 4 | Door | Private | Open |  |
| `d_dressing` | Dressing room ↔ gallery | 9, 8 | Door | Private | Locked | `k_master` |
| `d_g1` | Guest bedroom 1 ↔ gallery | 21.75, 8 | Door | Private | Closed | `k_g1` |
| `d_g2` | Guest bedroom 2 ↔ gallery | 26, 8 | Door | Private | Closed | `k_g2` |
| `d_g3` | Guest bedroom 3 ↔ gallery | 2.75, 10 | Door | Private | Closed | `k_g3` |
| `d_g4` | Guest bedroom 4 ↔ gallery | 8.25, 10 | Door | Private | Closed | `k_g4` |
| `d_g5` | Guest bedroom 5 ↔ gallery | 21.75, 10 | Door | Private | Closed | `k_g5`, held by the Guest player |
| `d_g1_g2` | Guest bedrooms 1 ↔ 2 (F03) | 24.5, 4 | Connecting | Private | Locked |  |
| `d_g3_g4` | Guest bedrooms 3 ↔ 4 (F04) | 5.5, 13 | Connecting | Private | Locked |  |
| `d_bath` | Bathroom ↔ gallery | 25.75, 10 | Door | Private | Closed | Bolt inside |
| `d_closet` | Housemaid's closet ↔ gallery | 28.5, 10 | Service | Service | Closed |  |
| `d_baize_1` | Gallery ↔ back-stair lobby | 30, 9 | Baize | Service | Closed |  |
| `d_attic` | Back-stair lobby ↔ attic corridor | 30, 9 | Service | Service | Open |  |
| `d_maids` | Maids' room ↔ attic corridor | 21.75, 8 | Service | Service | Closed |  |
| `d_men` | Menservants' room ↔ attic corridor | 27.25, 8 | Service | Service | Closed |  |
| `d_staffroom` | Staff player's room ↔ attic corridor | 21.75, 10 | Service | Service | Closed |  |
| `d_boxroom` | Box room ↔ attic corridor (F13) | 27.25, 10 | Service | Service | Locked | `k_boxroom` |
| `o_cellar_stair` | Back stairs ↔ cellar passage | 31.5, 10 | Opening | Service | Always open |  |
| `d_cellar` | Cellar passage ↔ wine cellar | 31.5, 8 | Door | Service | Locked | `k_cellar` |

The forecourt, yard and garden path join each other with no door, so the drive, the yard, the path and the garden lead from the front door round to the terrace without going through the house.

## Anchors

Plan on about 330 anchors: every room gets more than any case could use, and each anchor carries tags saying what it can hold. Anchors are named `<room>_NN` (`library_04`), and conversation pairs are named `<room>_pNN_a` and `<room>_pNN_b`.

| Tag | Meaning | Used by |
| --- | --- | --- |
| `stand` | Stands facing the arrow | NPCs |
| `seat` | Sits; the empty sits at seat height, facing out | NPCs |
| `lean` | Leans on a mantel, wall, balustrade or billiard table | NPCs |
| `bed` | Lies or sits on a bed | NPCs, players' beds |
| `work` | A task spot: range, sink, desk, lamp bench, bell board, sideboard | Staff NPCs |
| `pair` | Half of a two-person conversation spot; the pair face each other about 1.2 m apart | NPC conversations |
| `surface` | A prop on a table, shelf or mantel | Props |
| `drawer` | A prop inside a drawer, box, cupboard or safe, found by inspecting | Props |
| `wall` | A wall-hung prop: picture, notice, calendar, coat on a hook | Props |
| `hide` | A concealed prop spot: under a cushion, behind books, in a coat pocket, under a rug | Props |
| `window` | A spot to look out from | Players, NPCs |
| `stage` | A finale position | Finale |
| `listen` | Generated by the exporter on both sides of every door that closes, 0.5 m out | Players |

An anchor can carry several tags (`seat,pair`).

**Minimum per room.** The manifest lint checks these.

| Room group | Anchors | Of which |
| --- | --- | --- |
| Small: study, pantry, dressing room, bathroom, closet, housekeeper's room, lamp room, shed, attic rooms | 6 | 3 `surface`, 1 `drawer`, 1 `hide` |
| Medium: library, bedrooms, kitchen, servants' hall | 9 | 2 `pair` sets, 3 `surface`, 1 `drawer`, 1 `hide` |
| Large: hall, drawing room, dining room, billiard room, conservatory, cellar, garage | 12 | 3 `pair` sets, 4 `surface`, 2 `drawer`, 2 `hide` |
| Corridors, passages, gallery | 1 per 3 m of length | `stand` or `lean` |
| Outdoors | Terrace 10, garden 12, yard 6, forecourt 6, path 3 | 2 `pair` sets each on the terrace and in the garden |

Every bedroom also gets one `bed`, a `seat`, a wardrobe `drawer`, and a `surface` each on the dressing table and the writing table.

**Finale stage.** The drawing room gets twelve extra `stage` anchors: `drawing_stage_01` to `_10` for the NPCs in a horseshoe facing the fireplace, plus `drawing_stage_p1` and `drawing_stage_p2` for the players at its open end.

**Named anchors** for the co-op devices and features: `spawn_guest` (g5), `spawn_staff` (servhall), `bed_guest`, `bed_staff`, `bellboard` (servhall), the three dumbwaiter stops, both ends of each acoustic link, and the telephones.

## Blender conventions and the manifest

The greybox lives in one `manor.blend`. A headless Python script exports glTF plus `level_manifest.ron`, the only file the authoring chat reads. Neither step needs the engine, so the greybox can start now.

**Collections:** `L_cellar`, `L_ground`, `L_first`, `L_attic`, `L_grounds`, `Features` (one child collection per feature, `F01`…`F14`), `Nav` (walkable floors and stair ramps).

| Object | Name | Custom properties (glTF extras) |
| --- | --- | --- |
| Room volume | `room.<id>`, a box mesh, hidden in game; one volume can be several boxes joined | `zone`, `level`, `timeline` (casebook timeline row; corridors and stairs join the nearest hall or landing) |
| Anchor | `anchor.<room>_NN`, single-arrow empty, local +Y = facing | `tags` (comma list) |
| Door | `door.<id>`, empty at the opening centre on the wall line; leaf mesh `doorleaf.<id>` with its origin on the hinge | `kind`, `zone`, `default`, `key`, `feature` |
| Bell pull | `bell.<room>` |  |
| Dumbwaiter stop | `dumbwaiter.<kitchen\|pantry\|gallery>` |  |
| Acoustic link end | `acoustic.F0N.src` and `acoustic.F0N.dst` | `radius` (source end) |
| Telephone | `phone.hall`, `phone.study` |  |
| Spawns and beds | `spawn.guest`, `spawn.staff`, `bed.guest`, `bed.staff` |  |
| Art | `art.<name>` in the `Art` collection: drawn, never solid. `art.<object name>` replaces that object's picture and leaves it solid, and is hidden when a feature hides the object |  |

**Export:** `blender -b manor.blend -P tools/export_level.py -- out/` writes one `.glb` per level plus the manifest. The script works out each anchor's room from the volume it sits in, generates the `listen` anchors at doors, builds the anchor graph (links between anchors that can see each other inside a room, joined room to room through doors), and computes walking times.

```rust
LevelManifest(
    version: 1,
    rooms: [ Room(id: "library", level: Ground, zone: Public, timeline: "library", bounds: [((0.0, 10.0), (7.0, 16.0))]) ],
    doors: [ Door(id: "d_jib", a: "library", b: "study", kind: Concealed, zone: Private, default: Closed, key: None, feature: Some("F05")) ],
    anchors: [ Anchor(id: "library_04", room: "library", pos: (2.1, 14.6, 0.0), facing: 90.0, tags: [Seat, Pair]) ],
    features: [ Feature(id: "F05", doors: ["d_jib"], links: []) ],
    links: [ AcousticLink(feature: "F06", src: "acoustic.F06.src", dst: "acoustic.F06.dst", radius: 3.0) ],
    travel: [ Travel(from: "library_04", to: "hall_02", seconds: 11.2) ], // all anchor pairs, at NPC speed
)
```

**Manifest lint** runs as part of the export and prints PASS or FAIL with ids:

- every volume, door and anchor id is unique, and every door joins two volumes
- every anchor resolves to one room, the innermost volume containing it (the shed sits inside the garden), and stands on walkable floor
- each room meets its anchor minimum
- every room can be reached by the Staff, and every public room by the Guest
- the anchor graph is connected
- every feature has its objects
- every walk-budget route is within its limit

## Props library

The library has 88 period props, a little over the GDD's 60–80 because most are hand-sized. They are fixed now, before any case exists. The case will place 20–30 of them, so the list deliberately includes many things that could be clues. Furniture belongs to the modular kit, not to this list.

- **Text comes from data.** Paper props (letters, telegrams, diaries) are blank meshes, and their words are case strings. A paper prop on its own reveals nothing.
- **States go to a whole category.** If a prop gets a second state (opened, emptied, torn, broken, stained), every prop in its category gets one, so a single variant can't flag a clue. The case can place any prop more than once.
- **Greybox stand-ins.** Until M4, a prop is a box at its real size with its id on it. The final models come in M4, as the GDD plans.

| Category | Props | Count |
| --- | --- | --- |
| Papers | Letter, envelope, telegram, postcard, visiting card, menu card, place card, newspaper, magazine, diary, notebook, address book, cheque book, ledger, guest book, railway timetable, theatre programme, legal document, photograph (framed), photograph (loose), map | 21 |
| Desk | Fountain pen, inkwell, blotter, letter opener, paperweight, typewriter, magnifying glass | 7 |
| Drink and dining | Decanter, tumbler, wine glass, champagne coupe, cocktail shaker, soda siphon, wine bottle, coffee cup and saucer, teapot, tea tray, silver salver, candlestick, hip flask | 13 |
| Personal effects | Pocket watch, wristwatch, cigarette case, cigarette holder, lighter, matchbook, ashtray, handkerchief, gloves, evening fan, powder compact, spectacles, brooch, pearl necklace, signet ring, cufflink, wallet, banknotes, walking cane, umbrella, hat | 21 |
| Medicine and household | Medicine bottle, pill box, sleeping draught, smelling salts, hot-water bottle, oil lamp, electric torch, candle, key (generic) | 9 |
| Garden, garage and tools | Weedkiller tin, gardening gloves, secateurs, trowel, petrol can, rope, spanner, oily rag | 8 |
| Leisure | Playing cards, chess set, gramophone record, book, binoculars | 5 |
| Arms (period, kept out of sight) | Service revolver in its case, fireplace poker, sword stick, hunting rifle | 4 |

The four arms are there because a 1930s country house would have them, not because any case needs them. In keeping with the GDD's tone, nothing in the game shows a wound.

## Greybox checklist and freeze

The level freezes as `level-v1` once every box below is ticked. After that, the authoring chat can start the case bible.

- [ ] Block out all five levels from the room tables, using the kit's walls and openings
- [ ] One `room.<id>` volume for each of the 40 spaces
- [ ] All 53 doors from the door list, with their properties and leaf meshes
- [ ] Features F01–F14 built in their own collections, each switchable by a flag in the test build
- [ ] Anchors placed to the minimums, plus the finale stage and the named anchors
- [ ] Bell pulls in every public room and bedroom, the bell board, the dumbwaiter shaft and its three stops, both telephones
- [ ] `export_level.py` writes the glTF files and the manifest, and the manifest lint passes
- [ ] Walk budget measured in the greybox (walking it in Blender's walk mode is enough before the engine can load it), every route within its limit
- [ ] Commit, tag `level-v1`, note the manifest's hash in the GDD

**Later, once the engine can load the level (M2 exit in the GDD):** a synthetic case with ten NPCs on random schedules runs 18-minute loops with no stuck NPC at 60 fps. It exercises the whole house without anyone having to watch the real case. Geometry fixes found then are covered by the free changes in Purpose and rules.

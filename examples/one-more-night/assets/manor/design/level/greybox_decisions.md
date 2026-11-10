# Greybox decisions

Choices made where the spec is silent, plus spec issues found while building. Every value here is a constant at the top of `tools/greybox/build.py`, so changing one is a one-line edit and a rebuild.

## Spec issues and document differences

| # | Issue | Resolution |
| --- | --- | --- |
| S1 | `d_drawing_cons` (glazed double, 1.8 wide at 0, 7) spans y 6.1–7.9. Its shared edge is y 6–8, and the conservatory's exterior south wall at y = 6 reaches y 6.2. The opening would cut 0.1 m into that wall. | Opening narrowed to **1.6** (y 6.2–7.8), with its centre unchanged. `DOOR_WIDTH_OVERRIDE`. |
| S2 | The brief names the spec `level_spec_v2.md`; the exported file was `One More Night — Level Spec v2.md`. | Renamed to `level_spec_v2.md` in this folder. |
| S3 | The spec gives the export script as `tools/export_level.py`; the brief gives `tools/greybox/export_level.py`. | Following the brief: `tools/greybox/export_level.py`. |
| S4 | The JSON has opening sizes that the Markdown doesn't list: glazed 1.0 × 2.2, glazed double 1.8 × 2.6, connecting and concealed 1.0 × 2.2, archway 1.8 × 2.6, opening 1.0 × 2.1, hatch 0.8 × 0.6, gate height 1.8. | Used as given. They add to the Markdown and don't contradict it. |
| S5 | The hall archways (1.8 wide, y 8.1–9.9) fill the 2 m corridor exactly between its 0.2 walls, so they have no jamb. | Accepted as built. |

## Step 1: floors, ceilings, walls

**Vertical stack.** Each level owns a 0.2 floor slab under its floor and a ceiling slab that fills up to the next level's floor slab, so the levels stack without gaps or overlaps:

| Level | Floor top | Ceiling underside | Ceiling slab top | Walls |
| --- | --- | --- | --- | --- |
| Cellar | −3.0 | −0.6 | −0.2 | −3.2 → −0.2 |
| Ground | 0.0 | 3.4 | 3.6 | −0.2 → 3.6 |
| First | 3.8 | 6.8 | 7.0 | 3.6 → 7.0 |
| Attic | 7.2 | 9.6 above the corridor and back stairs, sloping to 8.4 at the knee walls | +0.2 | 7.0 → follows the ceiling |

- Where there's nothing above a level, the ceiling slab is 0.2 thick. This closes the service wing, the conservatory and the west part of the first floor. **No pitched roof yet**; one can come later as cosmetic geometry.
- **Hall:** the double-height half (x 11–19, y 0–8) has no first-floor slab. Its ceiling underside is at 7.2 and the slab tops out at 7.4. The first-floor walls around it run up to 7.4.
- **Hall/gallery edge at y = 8 (first floor):** there's no wall, only a header beam from the gallery ceiling (6.8) up to 7.4. The balustrade comes in step 4.
- **Back-stair tower (30–33 × 10–16):** it's open from the cellar to the attic ceiling, with no intermediate slabs; the flights and landings come in step 4. The lobbies at 30–33 × 8–10 on the first floor and attic have normal floors and ceilings.
- **Main stair footprint (15.5–19 × 10–16):** the first-floor slab is still whole. The stairwell is cut in step 4, together with the stair geometry.

**Attic ceilings.** The ridge runs over the corridor (y 8–10). The corridor and the back-stair lobby are flat at 2.4. The rooms slope from 2.4 at their corridor wall down to 1.2 at the knee walls at y = 3 (south) and y = 14 (north). The pitches are asymmetric (5 m and 4 m runs) because the room boxes are. The rest of the main block's footprint at attic level counts as loft, and walls against it are 0.2 interior walls.

**Wall classification.** Box edges are sampled every 0.5 m and each side is classified:
- the same room on both sides gets no wall
- two rooms get an interior wall, 0.2
- a room against the outside or an outdoor area gets an exterior wall, 0.4
- a room against the loft gets a 0.2 wall
- two outdoor areas get no wall

Pieces of the same kind join into straight runs centred on the line. At corners, runs along x extend to close the corner, and runs along y stop at the face of the x-run.

**Openings.** Each door splits its run into side pieces plus a lintel at the door's head height from the JSON sizes. The serving hatch also gets a sill, with its bottom edge at 0.9. The jib door (`d_jib`) is cut as a normal opening; its bookcase cover comes with F05. `s_terrace_garden` and `g_garden_e` aren't cut yet because they belong to the step 7 garden walls.

**Windows**, as openings in the walls only (no glass or frames):
- Exterior walls of indoor rooms get one window per 3.5 m of free wall, spaced evenly and kept 0.6 m from corners and door openings.
- Ground floor: 1.2 wide, sill 0.9, head 2.7. First floor: 1.2 wide, sill 0.9, head 2.5.
- Garage and shed: 1.0 wide, sill 0.9, head 2.1.
- Attic: 0.8 wide, sill 0.8, head 1.6. Only the menservants' gable end has an exterior wall, so it is the only attic window.
- The conservatory is "glazed": 1.6-wide openings every 2 m, sill 0.6, head 2.8.
- None in the cellar or the back-stair tower; the tower is left blank for the stair geometry.
- F11 study window: exactly one window, at (8, 16).

**Outbuildings and grounds.**
- The garage (ceiling 3.0) and the potting shed (ceiling 2.4) are indoor rooms, so their shells and doors are built in step 1.
- Ground heights: the forecourt, yard, terrace and garage are at 0.0.
- The walled garden and the shed floor are at −0.6, four 0.15 steps below the terrace.
- The garden path rises in a straight line from −0.6 at the garden gate (x 24) to 0.0 at x 42, then runs level to the yard.
- Outdoor ground slabs go down to −1.0, so the terrace edge reads as solid until the steps go in.

**Object names.** All of these are in `L_<level>` or, for the garage, shed and outdoor areas, `L_grounds`:
- `floor.<level>.<room>` and `ceiling.<level>.<room>`
- `wall.<level>.NNN`, one object per wall run, with its pieces, lintels and sills joined. Walls carry `kind` (exterior / interior / header), `level`, `rooms`, `doors` and `windows`.
- Walkable top faces go to `Nav` as `nav.<level>.<room>`. That collection is hidden in the viewport.
- Every object carries `generated = True`, `gb_kind`, `level`, and `room` where it applies.

## Step 2: room volumes

- **Extent:** `room.<id>` covers every box of the room, from its floor to its ceiling underside. Attic volumes follow the sloped ceilings. Where the room continues on the level above, the box runs up to that level's floor so the parts touch:
  - the hall is 0–7.2 over its double-height half and 0–3.4 over its north half
  - the back stairs are one volume from −3.0 to 9.6, including both lobbies
- **Mesh:** each box (or each part of a box, split where the height changes) is a separate closed prism, a loose part of the room's mesh. Containment tests should test each part.
- **Outdoor volumes** are 4.0 m tall above their ground. The shed volume sits inside the garden volume, as the spec says; it is the only overlap allowed, and the build checks that.
- **Collection:** a volume goes in the collection of its room's `level` (the back stairs are in `L_ground`, the garage and shed in `L_grounds`). Volumes display as wireframe and are excluded from renders.
- **Properties:** `zone`, `level` and `timeline` as in the spec, plus `room`, `outdoor`, `gb_kind = room` and `generated`.
- **`timeline`:** each room's own id, except that corridors and stairs join the nearest hall or landing:
  - `corr_w`, `corr_e` → `hall`
  - `passage` → `servhall`
  - `backstairs` → `servhall` (it's entered from the service passage)
  - `cellar_pass` → `servhall` (the cellar has no hall; it's the foot of the back stairs)
  - `gallery` keeps its own row as the first-floor landing, and `attic_corr` keeps its own row as the attic landing.

## Step 3: doors

- **`door.<id>` empties (53):** at the centre of the opening, on the wall line, at the floor of room `a`. Local +X runs along the wall; local +Y points into room `a`. The empties carry the spec's `kind`, `zone`, `default`, `key` and `feature`; an empty string means none. They also carry `a`, `b`, `level`, `width`, `height` and `leaves`, plus `note` for `d_g5` (the Guest player holds `k_g5`) and `d_bath` (bolt inside).
- **Leaves (58):**
  - Archways, steps and plain openings get none.
  - Double, glazed double, French window and garage openings get two leaves, `doorleaf.<id>` and `doorleaf.<id>.2`, each just under half the opening wide.
  - Everything else gets one: doors, service, baize, glazed, connecting, concealed, gate and hatch.
- **Hinge and swing:**
  - Every leaf swings into room `a`, the first room in the spec's "between" pair. That pair puts the room before the corridor or outdoors in almost every case, so doors open into rooms and exterior doors open inwards.
  - A single leaf hinges on its west or south jamb. On a pair, the second leaf hinges on the opposite jamb.
  - The leaf origin is on the hinge at the bottom of the opening. Local +X runs from the hinge along the leaf.
  - The leaf sits flush with room `a`'s face of the wall.
  - Properties: `swing` (+1 or −1; opening is a rotation of `swing × open_angle` degrees about local Z), `open_angle` = 90, `door`, `leaf` (1 or 2), and `double_acting` (true for baize doors).
- **Leaf size:** 0.05 thick, with 0.01 clearance at the jambs, between leaves and under the leaf. The hatch leaf sits on the 0.9 sill.
- **Rest pose:** all leaves are modelled closed, whatever the door's `default`. Open, closed or locked is game state.
- **Garden gate:** there is no garden wall until step 7, so its leaf is placed as if in a 0.3 wall.
- **Collections:** doors and leaves go in the level collection, or `L_grounds` when both rooms are grounds rooms. Feature doors will also be linked into their `Features/F0N` collection in step 5.
- **Review helper:** `views.swing(True)` swings every leaf open to check its direction. `reset()` closes them again.

## Step 4: stairs

**Main stair** (U-shaped, in the hall footprint 15.5–19 × 10–16):
- **Steps:** 21 risers of 0.181 (3.8 / 21, the nearest to the spec's 0.18), going 0.28, flights 1.4 wide. Flight 1 has 11 risers and flight 2 has 10.
- **Lanes:** flight 1 is in the west lane (x 15.5–16.9) and climbs north from y 11.22. The east lane (x 17.5–18.9) ends at the face of the hall/billiard wall, leaving a 0.6 well between the lanes.
- **Half-landing:** at +1.99, from y 14.02 to the north wall face at 15.8, so 1.78 deep, more than the 1.4 width.
- **Flight 2** returns south in the east lane and arrives on the first floor at **y 11.5**, so the stairwell opening (x 15.5–19, y 11.5–16) sits on the 0.5 m grid. Above the foot of flight 1 (y 11.22–11.5) the slab stays, with at least 3.0 headroom.
- **Under-stair:** the flights and landing are solid down to the hall floor; the space under flight 2 is a closed block.
- **Balustrades** are 0.06 thick, standing on the tread edges, 0.9 above the nosing line:
  - flight 1's open west side and the west edge of the landing
  - both edges of the well, and the landing edge across the well
  - on the first floor, the stairwell edges along y 11.5 (x 15.44–17.5) and x 15.5, 1.0 high. The east lane at y 11.5 is the arrival and stays open.
- **Window:** the ground-floor window that the rule placed in the hall's north wall over the landing is removed.

**Back stairs** (a dog-leg repeated per storey, in the tower at 30–33 × 10–16):

| Storey | Risers | Rise | Half-landing |
| --- | --- | --- | --- |
| Cellar → ground | 16 | 0.1875 | −1.5 |
| Ground → first | 20 | 0.190 | +1.9 |
| First → attic | 18 | 0.189 | +5.5 |

- **Going:** 0.24, flights 1.0 wide, the two flights of a storey always split evenly.
- **Lanes:** west x 30.2–31.2, clear of the cellar's 0.4 wall. East x 31.8–32.8.
- **Spine:** between the lanes there is a **0.6 solid spine wall** (x 31.2–31.8), which encloses the dog-leg. It runs from each floor landing to that storey's half-landing. On the top storey it ends 1.0 above the attic floor, where it doubles as a balustrade.
- **Each storey:** a floor landing at the south (y 10–11.1, one stair width beyond the wall face, joining the lobby floor at 8–10 on the first floor and attic). The up flight climbs north in the west lane to a half-landing at the north wall face (15.8). The second flight returns south in the east lane to the next floor landing.
- **Structure:** flights have a 0.15 waist under the nosing line. The cellar storey is solid down to the cellar floor.
- **Attic:** a 1.0 balustrade at the landing edge over the west lane.
- **Headroom:** at least 2.6 everywhere above a flight.

**Hall overlook:** a 1.0 balustrade along y 8 on the first floor (x 11.1–18.9), sitting under the header beam from step 1.

**Collision (`Nav`):**
- Each flight has one ramp on its nosing line, starting at floor level one going before the first riser. That gives an exact meeting with the floor below and the landing above.
- Every landing has a flat quad.
- Ramps carry `stairs = True`, which the walk-budget step reads to apply ×0.75. Landings carry `stairs = False`.
- Floor nav is removed where the stairs stand: the hall under the stairwell (15.5–19 × 11.5–16) and the back-stair cellar floor north of y 11.

**Volumes:** the hall volume now reaches 3.8 over the stairwell, so the stair is inside a room all the way up, which closes the gap noted in step 2.

**Objects:**
- `stair.main`, `stair.back.<cellar|ground|first>` and `landing.back.<ground|first|attic>`
- `balustrade.main_stair`, `balustrade.main_stairwell`, `balustrade.hall_gallery` and `balustrade.back_attic`
- `nav.stair.main.{flight1,landing,flight2}`, `nav.stair.back.<storey>.{a,half,b}` and `nav.landing.back.<level>`

## Step 5: features

- **Collections:** `Features/F01`…`F14`. Feature objects are linked there and stay in their level collection too, so the per-level export still finds them. Each one carries `feature` and `feature_state` (`on`, `off` or `both`).
- **Toggles:** the scene custom properties `F01`…`F14` (booleans, default on). They are kept across rebuilds, so a value set by hand survives.
  - Objects tagged `on` hide when their flag is off, and `off` objects hide when it is on. Simple drivers on `hide_viewport` and `hide_render` (`1 - v` and `v`) do this, so no script auto-run is needed.
  - Editing a value in Scene properties › Custom Properties updates the view. From Python, use `views.feature("F05", False)`, which tags the scene; otherwise the drivers keep their old value.
  - Door states (locked, no key) are case data for the game, so door features have no off-state geometry.
- **Acoustic links:** `acoustic.F0N.src` (a sphere display sized to `radius`) and `acoustic.F0N.dst` (an arrow). Both ends are 1.5 above their floor and carry `room`, `level` and `two_way`; the source also carries `radius`. All are `on`-state objects.

| Feature | Source (room, xy, radius) | Destination | Notes |
| --- | --- | --- | --- |
| F01 | dining, 26.4, 4.0, r 4.0 | pantry, 28.0, 4.0 | hatch side |
| F02 | hall, 15, 4, r 4.0 | gallery, 15, 8.6 (first) | the source also carries `area` = `11,0,19,8` from the spec, since a circle can't match the rectangle |
| F06 | study, 9, 13, r 3.7 (covers the room) | g4, 10, 14.8 (first), at the upstairs hearth | |
| F07 | `phone.hall` (11.35, 13.5, 0.85), r 1.0 | `phone.study` (10.55, 13.5, 0.80) | two-way |
| F08 | master mouthpiece (5.6, 7.85, first + 1.5), r 1.0 | pantry mouthpiece (29.85, 2.0, 1.5) | two-way |
| F11 | study, 9, 13, r 3.7 | terrace, 8, 16.8, outside the window | |

- **Telephone and mouthpiece positions** (step 6 reuses them):
  - The hall telephone is on the hall's west wall, back to back with the study desk phone on the study's east wall; this is the spec's "alcove near x 12, y 13.5".
  - The master's speaking-tube mouthpiece is on its north wall, 2 m east of `d_master`. The pantry's is on its east wall, well away from the dumbwaiter corner.
- **Feature geometry:**
  - **F01:** `F01.bolt` (off) is a bolt on the pantry face of the hatch.
  - **F02:** links `balustrade.hall_gallery`.
  - **F03, F04, F09, F10, F14:** their doors and leaves.
  - **F05:** `F05.cover` is a 0.35 deep bookcase on the library face. It is parented to `doorleaf.d_jib`, with its origin on the hinge, so it swings with the leaf. The leaf is `on`; `F05.infill` (off) fills the opening flush with the study face. So with F05 off it's just a bookcase in front of a solid wall.
  - **F06:** a chimney breast (1.6 wide, 0.4 deep, to the ceiling) with hearth, mantel and firebox at x 10 on the north wall, in both the study and g4.
  - **F08:** brass mouthpieces (both), with caps (off) over them.
  - **F11:** `F11.sash` (glass) and `F11.curtains` (off) on the study window at (8, 16).
  - **F12:** links the shed's volume, floor, ceiling, nav, walls and door.
  - **F13:** links the box room's volume, floor, ceiling, nav, its knee wall and its door.

## Step 6: devices

- **Dumbwaiter shaft:**
  - The spec gives the shaft as 29.1–29.9 × 7.1–7.9. It sits in the pantry's north-east corner on the ground floor and in g2's on the first.
  - It's enclosed by 0.2 partitions centred on its west and south edges (x 29.1, y 7.1), which leaves a 0.7 × 0.7 clear shaft. The slab is open across the 1 × 1 cell (29–30 × 7–8) between the ground and first floors.
  - The shaft has no walkable floor. The pantry volume reaches 3.8 over the shaft and g2's starts there; both are harmless.
  - `dumbwaiter.shaft` (the partitions) and `dumbwaiter.car` (resting at the ground-floor stops).
- **Dumbwaiter hatches:** 0.6 wide, sill 0.9, head 1.5.
  - Kitchen: through the x 30 wall.
  - Pantry: through the shaft's west partition.
  - Gallery: through the y 8 wall on the first floor.
  - The stop empties `dumbwaiter.<kitchen|pantry|gallery>` sit at the hatch sill on the room face, with local +Y facing into the room.
- **Fireplaces:** placed now so the bell pulls can go beside them. Every spec room with a fireplace gets one: the drawing room, the study (F06), the master bedroom and g1–g5 (g4 is F06).
  - Each is a 1.6 × 0.4 chimney breast to the ceiling, with hearth, mantel and firebox.
  - Drawing room and master: the west gable, at y 3, on one stack.
  - g3: west gable at y 13.
  - g1: west wall at y 4.
  - g2: east wall at y 3.5.
  - g5: east wall at y 13.
  - Windows keep clear of fireplaces (≥ 0.6 beyond the breast), which moved or removed a few windows: the total went from 51 to 50.
- **Bell pulls `bell.<room>`:** the 13 rooms in the spec's list. Each is a cord from 0.1 below the ceiling down to a handle; the object's origin is on the handle, 1.0 above the floor. A `facing` property gives the direction into the room.
  - Beside the fireplace (1.1 from its centre) where there is one.
  - Otherwise on an inside wall:
    - dining: north wall at x 21
    - library: east wall at y 11.2
    - billiard: west wall at y 13
    - conservatory: on the library wall at y 12
    - dressing room: east wall at y 4
    - study: between the F11 window and the fireplace, at x 9
- **Bell board `bellboard`:** servants' hall west wall, centred at y 6.2, 1.2 × 0.8, z 1.6–2.4.
- **Telephones:**
  - `phone.hall` sits on a small table (`furniture.hall.phone_table`, top 0.80) against the hall's west wall at y 13.5.
  - `phone.study` sits on the study desk (`furniture.study.desk`, 0.7 × 1.4, top 0.76) against the study's east wall. The two phones are back to back through one wall.
  - Origins are at the base of the phone. The F07 link ends moved to these heights.
- **Beds:**
  - `furniture.bed.g5`: a double, 1.4 × 2.0, mattress top 0.55, head against the west wall.
  - `furniture.bed.attic_staff`: a single, 0.9 × 1.9, mattress top 0.50, head against the east wall, under the sloping ceiling (about 1.5 clear above it).
  - The other bedrooms' beds come with the step 8 furniture.
- **Markers** use `ARROWS` empties with local +Y as facing, the same convention the anchors will use:
  - `bed.guest` and `bed.staff` sit at the centre of the mattress top, with +Y from head to foot.
  - `spawn.guest` is at the g5 bedside facing east.
  - `spawn.staff` is in the servants' hall at (38.3, 6.2), facing the bell board.

## Step 7: outdoors

- **Ground** for the forecourt, yard, terrace, garden and path, plus the garage and shed shells, comes from step 1 (heights logged there).
- **Edges:** whatever stands on an outdoor edge comes from what lies on either side of it:
  - **Garden wall** (`outdoor.garden_wall`): 0.4 thick, 2.4 above the garden (top +1.8), brick. It runs where the walled garden meets nothing or the path: the west, north and east sides. The east gate `g_garden_e` is cut 1.2 × 1.8 with a lintel. The gate leaf now sits flush with the garden face of the 0.4 wall.
  - **Terrace parapet** (`outdoor.terrace_parapet`): 0.3 thick, 0.9 high, standing on the terrace side of its edge. It runs along the north edge over the garden (the 0.6 drop) and at the terrace's west and east ends. It leaves a 3.0 gap for the steps.
  - **Hedges** (`outdoor.hedge`): 0.6 thick, 1.4 high. Not in the spec; they stop players walking off the map. They run on every other outdoor edge that faces nothing:
    - round the forecourt (south, west, east, and the strip south of the conservatory)
    - the yard's east side and its bit of west side north of the wing
    - both sides of the garden path (except where it meets the yard, the garage and the gate)
  - There's no entrance from outside the grounds.
- **Terrace steps** (`steps.terrace_garden`) at x 13: 3.0 wide, 4 risers of 0.15, going 0.35, built out into the garden (y 20–21.05) and solid to the ground. A collision ramp `nav.steps.terrace_garden` (with `stairs = True`) replaces the garden floor nav under them.
- **Garden fixtures** (the spec's "gravel paths, sundial, lily pond, benches, cold frames"), as blocks:
  - **Gravel paths:** a north–south path (x 12–14) from the steps to the north wall, and an east–west path (y 29–31) from the west wall to the gate. They meet in a 4 × 4 crossing round the sundial at (13, 30). A spur runs to the shed door. The gravel is visual only, 5 mm above the ground.
  - **Lily pond:** 4 × 2.5 at (5, 25.5), with a 0.4 high, 0.3 wide stone kerb (it can be sat on: `seat_height`) and water 0.15 below the kerb. The garden nav is removed under it.
  - **Benches:** four of them, 1.6 × 0.5, seat 0.45, with a back. Two stand against the north wall (x 8 and x 18), one south of the pond facing it, and one against the west wall facing east. Each carries `seat_height` and `facing`.
  - **Cold frames:** against the north wall at x 19.5–23.5, 0.5 high (`surface`).

## Step 8: furniture and anchors

- **Where the layout lives:** `tools/greybox/layout.py` is pure data: one list of items per room. `build.py` turns it into `furniture.<room>.<kind>_NN` boxes and `anchor.<id>` empties. To change a room, edit its list and rebuild.
- **Hand-tuning:** an anchor or piece moved by hand in Blender is rebuilt from `layout.py` on the next build, unless its `generated` property is removed first. To keep a hand edit, either remove `generated` from the object (and delete the matching item from `layout.py`), or put the new position in `layout.py`.
- **Anchor ids:** `<room>_NN` numbered in list order per room, pairs `<room>_pNN_a` / `_b`, the finale `drawing_stage_01`…`_10`, `_p1`, `_p2`. Reordering a room's list renumbers its anchors, which is free before the freeze only.
- **Anchor objects:** `SINGLE_ARROW` empties, local +Y = facing, with `tags`, `room` and `level`. Blender draws the single arrow along local Z, so it points up in the viewport; the facing is still local +Y, as the spec's conventions table says.
- **Anchor heights:**
  - `stand`, `lean`, `pair`, `work`, `window`, `stage`: on the floor.
  - `seat`: at seat height (0.45), facing out of the seat.
  - `surface`: on top of the piece (bookcases at 1.2, shelves at 1.0).
  - `drawer`: at the front face of the piece, at drawer height.
  - `hide`: at the hiding place (seat height for cushions, 1.4–1.5 in wardrobes and bookcases, the floor for rugs).
  - `wall`: on the wall face at 1.6.
  - `bed`: at the centre of the mattress top, +Y from head to foot.
- **Pairs:** 49 sets. Standing pairs are 0.6 either side of a centre, facing each other (`stand,pair`). Seated pairs are two chairs facing each other 1.2 apart, with a small table between (`seat,pair`).
- **Count:** 566 anchors, against the spec's "about 330". Every furniture kind brings its standard anchors (a chest gives a surface and a drawer, a bookcase a surface and a hide, and so on), so rooms end well over their minimum. Totals by tag: 154 surface, 65 drawer, 63 hide, 88 seat, 12 bed, 12 work, 12 wall, 9 window, 42 lean, 12 stage.
- **Checks in the build** (it fails with the ids if any break):
  - furniture stays inside its room, clear of other furniture, fireplaces, stairs, the dumbwaiter shaft and a 1.0 m zone each side of every door (1.5 for the garage doors)
  - furniture fits under the ceiling
  - standing anchors lie in their room, 0.2 clear of furniture, with 1.9 headroom
  - every room meets the spec's minimums, including the bedroom extras, the corridor rule and the outdoor pair sets
- **Named anchors:** the spec's named anchors (`spawn_guest`, `bellboard`, the dumbwaiter stops, acoustic ends, telephones) are the device and feature objects from steps 5 and 6. They aren't duplicated as `anchor.*`; the exporter will list them.
- **Back stairs:** not in any anchor group; it gets one `stand` on each of its four floor landings.
- **Finale stage:** ten spots on an arc of radius 2.6 about (2.2, 3.0), from −70° to +70°, each facing the arc's centre in front of the hearth. `drawing_stage_p1` and `_p2` stand at the open end, either side of the hearth at (1.7, 1.9) and (1.7, 4.1), facing the horseshoe. The drawing-room furniture keeps clear of the arc.
- **Furniture, room by room:** follows each room's fixtures column in the spec (table for 10, billiard and card tables, range, sink, lamp bench, linen press, racks and cask table, car and workbench, trunks, and so on). Every bedroom has a bed, two bedside tables, wardrobe, dressing table, writing table and an armchair by the fire. The attic rooms have single beds along the low side.
- **Window anchors** stand 0.6 inside a real window of the room. The guest bedrooms and the master have none, because their windows fall behind beds and dressing tables.

**Attic headroom (follows from the spec):** the ceilings fall from 2.4 at the corridor to 1.2 at the eaves. So 1.9 m of headroom (the tallest NPC) exists only within 1.7 m of the corridor wall in the north rooms (staff, box room), and within 2.1 m in the south rooms. A 1.75 m player clears 2.2 m and 2.7 m. All standing anchors there are inside that strip; beds, chests and trunks sit under the slope. The Staff player's bed (headroom about 1.5 over it) is reached from the standing strip, within the 2.0 interact ray.

## Step 9: export, manifest, lint and walk budget

- **Script:** `tools/greybox/export_level.py` (the brief's path; the spec says `tools/export_level.py`). It reads everything from the scene, so hand-tuned anchors are what gets exported. `level_spec.json` is read only for the anchor minimums, walk limits and speeds.
- **Running it headless:** Blender here is a Flatpak, which starts in its own directory, so every path must be absolute:
  `flatpak run org.blender.Blender -b "$PWD/manor.blend" -P "$PWD/tools/greybox/export_level.py" -- "$PWD/export"`. It exits 1 if the lint fails. `export/` is committed.
- **Output in `export/`:**
  - `manor_<cellar|ground|first|attic|grounds>.glb`: the level's collection plus that level's `Nav` objects. Custom properties go out as glTF extras, Y up. Room volumes, `Nav` and off-state feature objects are hidden in the file but are exported.
  - `level_manifest.ron`, `travel_full.csv`, `level_lint.txt` (the lint, the walk budget and the manifest's SHA-256).
- **Manifest, where it goes beyond the spec's example:**
  - `facing` is degrees counter-clockwise from north (+Y), the empty's Z rotation.
  - `anchors` also holds the generated `listen_<door>_a/_b` anchors and the named anchors, whose ids are the object names with dots as underscores (`spawn_guest`, `bed_staff`, `bellboard`, `dumbwaiter_kitchen`, `phone_hall`, `bell_drawing`, `acoustic_F06_src`). Named anchors have no tags, except the beds (`Bed`).
  - `Feature` gains `on` and `off`: the objects to show only in that state. `links` lists the feature's two acoustic ends.
  - `AcousticLink` gains `two_way` and `area` (the F02 rectangle).
  - `Room.bounds` lists the room's boxes in plan, without heights.
- **Travel times, a deviation from the spec:** the spec asks for every anchor pair in the manifest. That is 37,128 pairs (about 1 MB), too much for the authoring chat to read. So the manifest's `travel` has one representative anchor per room (the standing anchor nearest the middle of the room's anchors), every pair of them: 703 entries. All pairs are in `travel_full.csv`. Both are at NPC speed (1.3 m/s), stairs ×0.75.
- **Listen anchors:** on both sides of every door with a leaf except the hatch, 0.5 from the wall line, facing the door: 96 of them. Archways, the cellar-stair opening, the terrace steps and the hatch get none.
- **The graph:**
  - Nodes: every anchor a person can occupy, a node each side of every door, and the ends of every stair ramp and the middle of every landing (from `Nav`).
  - It also has a lattice of free-floor waypoints: 1 m apart, 2 m in the forecourt, garden and yard, 0.3 clear of walls and furniture. Without them, paths zigzag between anchors and read 20–30% long.
  - Links join nodes in the same room that can see each other (sampled every 0.2 m against furniture, fireplaces, stairs and balustrades); waypoint links reach 2.3 lattice steps.
  - Rooms join through door nodes. The forecourt, yard and path join where they meet with no door. The terrace and garden join on the steps' ramp. Nobody passes the hatch.
  - Seats and beds join at floor level.
- **Lint:** the spec's seven checks, as nine lines. Notes on how each is read:
  - "Stands on walkable floor": standing anchors must be on a `Nav` surface; seats, beds and prop anchors must be within 1.5 m of one.
  - "Reachable by the Guest": from `spawn.guest`, using only doors whose zone is public, plus the door of the Guest's own room. Public rooms are those zoned public or open.
  - "Reachable by the Staff": from `spawn.staff`, through any door.
  - The anchor minimums are checked again here, from the exported anchors.
- **Walk budget:** player speed 1.4 m/s, stairs ×0.75. Each route runs between the standing anchors nearest the middle of its two rooms; the garden route starts from the garden's farthest standing anchor. Indoor public routes (the first, second, third and fifth) have the 30 s limit, the rest 45 s.

| Route | Estimate | Spec | Limit |
| --- | --- | --- | --- |
| Drawing room → dining room | 12.0 s | 14 s | 30 s |
| Conservatory → billiard room | 22.5 s | 22 s | 30 s |
| Hall → guest bedroom 1 | 18.7 s | 21 s | 30 s |
| Servants' hall → guest bedroom 2 | 24.3 s | 23 s | 45 s |
| Conservatory → guest bedroom 1 | 29.8 s | 30 s | 30 s |
| Garden far corner → hall | 25.0 s | 20 s | 45 s |
| Kitchen → garage | 21.5 s | 19 s | 45 s |
| Servants' hall → Staff attic room | 33.1 s | 33 s | 45 s |

The conservatory → guest bedroom 1 route passes with 0.2 s to spare, and only between mid-room anchors: from the conservatory's far end to the far side of the bedroom it would be over 30 s.

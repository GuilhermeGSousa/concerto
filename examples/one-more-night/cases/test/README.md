# The open test case

Every system is built and tested against this case, never the real one. It is
tiny and its solution is public.

- Level: `level_manifest.ron`, two rooms (`room_a`, the drawing room; `room_b`,
  the study) and a service corridor.
- Three characters, ten facts, two deductions and a two-statement finale.

## What it exercises

| System | Where |
|---|---|
| A private overheard conversation | `conv_01`, 19:00 in the study; it switches to its cover line while the Guest is in the room, so the Staff listens at the service door |
| A locked door the Staff opens | `d_service`, between the study and the corridor; the key `p_key` hangs in the corridor |
| The dumbwaiter | `p_cufflink` lies in the service corridor, where only the Staff goes, and only Lady Ashby, whom only the Guest questions, knows whose it is: the Staff sends it up, or carries it through |
| The bell board | Lady Ashby rings from the study at 19:30; the Staff who reads the board while the flag is up gets `f_10` |
| Topic dialogue | Each character has a question; two answers change with the hour or with a fact |
| The timeline | `f_03`, `f_04` and `f_07` have a room and a time |
| Inspect, document and carry | `p_case`, `p_basket`; `p_note`; `p_key`, `p_cup` |

At 21:30 the theft is discovered: Lady Ashby and Hobbs go to the study, and
Hobbs has a new answer for a player who already has `f_04`.

## Solution

- `d_01`: Mr Corwin took the snuffbox from the study.
- `d_02`: It was hidden in the linen basket in the service corridor.
- Finale: the thief is Mr Corwin; the reason was a debt.

Facts needed: `f_01` (display case) shows `d_01`; `f_09` (the Guest shows
Lady Ashby the cufflink the Staff found) gives "Mr Corwin"; `f_05` (basket) and `f_04` (ask Hobbs after 19:40) give the terms of
`d_02`; `f_03` (the argument) gives "a debt".

# STILL LIFE — design

A short first-person horror game built on Concerto, for itch.io (web first).
The crate and folder are still called `after-hours`, after the game's first
version.

> Marrow House, Bloomsbury, 1893. You are the auctioneer's clerk, sent to
> catalogue a dead painter's pictures over five nights. The painter worked from
> life-size wooden lay figures. They only move when you are not looking.

This document is the source of truth for *why* the game is the way it is.
Future work should update it alongside the code.

## Why the rework

The first version (AFTER HOURS, a supermarket) was playable but bland.
Playtest feedback, and what changed because of it:

| Feedback | Response |
|---|---|
| No personality, story or art direction | A Victorian painter's house with a written story: night cards, Marrow's diary pages, and an ending. Wallpaper, wainscot, candlelight, gilt frames, dust sheets, and a period serif typeface. |
| A supermarket is an odd place for mannequins | The figures are the painter's lay figures, which are jointed artist's mannequins. They belong in this house. |
| The horror starts too fast | Night one is a tutorial where nothing can hurt you. Only one thing happens: a figure you have seen is somewhere else. Each later night adds one possessed figure, and hunts begin only after some lots are catalogued. |
| The AI charges straight in, so you end up walking backwards before a horde | Most figures are inert. Possessed ones watch, stalk and retreat. A director allows at most one hunter (two from night 5), and a stared-down hunter withdraws. |

## Pillars

1. **One rule, fully honest.** A figure you can see never moves. "See" is
   computed conservatively, and the lighting test mirrors the shader.
2. **Doubt.** All the figures look alike and most never move. The possessed
   ones mostly turn to face you, shift their pose, or take up a new place
   behind you. The player should never be quite sure what they saw.
3. **Light is a resource.** A bullseye lantern (oil), candles that gutter out
   as the night goes on, and moonlit windows that you can see by but that do
   not light the figures.
4. **Short nights, slow build.** Five nights of 4–8 minutes on generated
   houses, then an endless coda.

## Story

- Elias Marrow, a society portrait painter, worked from lay figures bought in
  Paris.
- His wife Clara died in 1891. After that he painted only the figures, and he
  carved one of her with no face.
- He disappeared in the winter of 1892.
- Mr Pike, the auctioneer, sends the clerk (the player) to catalogue the
  estate, by night.

Each night opens with a card of the clerk's own notes (`story::intro`). A
diary page is hidden in the house each night (`story::page`). The last line of
the last page names the grey figure, and she walks on night 5. The ending
card is the sale catalogue: "Lot 1. A lay figure, life-size, in a clerk's
coat."

## Core loop

1. A night card with the story and the goal.
2. Explore the house and find the **lots**: pictures with a paper tag. Hold E
   while facing one to catalogue it (1.6 s with your back to the room).
3. Once every lot is catalogued, go back to the hall and hold E at the
   **ledger** to sign it and end the night.
4. A hunting figure that reaches you ends the night; retry the same house.

## Systems

### House generation (`level.rs`)
- A square grid of 4 m cells, split by recursive BSP into rooms of up to 3×3
  (1-wide strips of 3+ cells become corridors).
- Rooms are joined by a random spanning tree of doorways, plus extra doors
  for loops (`loop_doors`) so you can circle away from a hunter.
- Doorways are 1.9 m wide, so you can always get past a frozen figure.
- The **hall** is a small outer room holding the front door and the ledger.
  The biggest room becomes the **studio** (easels, most of the figures), the
  next the **gallery**. From night 3 the farthest parlour becomes **Clara's
  room**. The rest become the library, dining room and parlour.
- Lots go on solid walls far from the door, one per room where possible.
  Clara's room is favoured.
- Pure and deterministic from a seed. Tests cover connectivity, tiling, and
  placement rules.

### Dressing the house (`house.rs`, `textures.rs`, `palette.rs`)
- Everything is procedural: textures are painted in code, and geometry is
  boxes batched per material.
- Each side of a wall is dressed for the room it faces:
  - wallpaper chosen by room kind;
  - panelled wainscot, dado rail, skirting;
  - picture rail and cornice.
- Wall furniture varies by room kind: sheeted armchairs, sofas and pianos,
  bookcases, sideboards, clocks, beds, fireplaces. Tables and daises sit on
  the corners between cells, clear of the walking lanes.
- Outer walls get curtained sash windows with emissive moonlit glass. These
  are for mood only and add nothing to the lighting maths.
- Pictures are six procedural oils (sitters, groups, landscape, seascape,
  still life, Clara) under a yellowed, cracked varnish.
- Candles (`Candle`) breathe. Each lot catalogued starts one guttering out,
  and from night 3 draughts snuff one every ~75 s.

### Observation (`mannequin.rs`, `lighting.rs`)
A figure is **observed** when any of its sample points (knees, chest, head)
passes all of these, from this frame's eye *or* last frame's:
- It is inside the camera frustum, widened by a small safety margin.
- It is unoccluded: a physics ray from the eye reaches it.
- It is visibly lit. The irradiance there is computed with the renderer's own
  falloff, then dimmed by fog. Lights counted:
  - candles;
  - the fanlight's moonlight;
  - the lantern's spot beam;
  - the lantern's faint spill.

  A point within arm's reach (≈2.6 m) always counts as lit.

### Figures (`mannequin.rs`)
Every figure is **inert** (it never moves) or **possessed**. A possessed
figure has a **mood**:

| Mood | While unseen |
|---|---|
| Watch | Turns to face you (a faint wooden click if close); every 15–30 s it shifts its pose. |
| Stalk | Walks to a cell 2–3 steps from you, preferably behind you and away from other figures, and waits there. It never comes closer than 3.2 m. |
| Hunt | Paths straight to you over the grid; touching you ends the night. |
| Retreat | After a hunt, withdraws to a far cell and goes back to watching. |

Poses are single frames of animation clips, picked by menace: display poses
for watching and retreating, uneasy ones for stalking, reaching ones for
hunting. Frozen figures pause their animation player.

### The director (`mannequin::direct`)
Per-night `Rules` decide pacing:
- how many may stalk and hunt at once;
- when stalking starts;
- how many lots must be catalogued before hunting starts;
- the gap between hunts, and hunt speeds.

When a hunt starts, the figure knocks twice, so you hear it coming. A hunt
ends when:
- you stare it down (1.2 s of unbroken sight after its minimum duration);
- you get more than 6 steps away;
- or it runs long.

It then retreats and the next hunt waits for the gap. Night one has no hunts,
only a one-off "doubt" move after the second lot.

| Night | Grid | Lots | Figures | Possessed | Stalkers / hunters | Hunts begin | Hunt speed |
|------:|-----:|-----:|--------:|----------:|-------------------:|-------------|-----------:|
| 1 | 5×5 | 3 | 6 | 0 | 0 / 0 | never | – |
| 2 | 6×6 | 3 | 7 | 1 | 1 / 1 | after 2 lots | 3.0 m/s |
| 3 | 6×6 | 4 | 8 | 2 | 1 / 1 | after 1 lot | 3.3 m/s |
| 4 | 7×7 | 4 | 9 | 3 | 2 / 1 | after 1 lot | 3.7 m/s |
| 5 | 7×7 | 5 | 10 | 4 + Clara | 2 / 2 | after 1 lot | 4.0 m/s |
| 6+ | 7×7 | 5 | 10 | 4 + 1/night | 2 / 2 | at once | 4.3 m/s |

The player walks at 3 m/s and hurries at 5.4 m/s, with limited breath.

### Player (`player.rs`)
- A capsule body with WASD movement and mouse look (pointer lock on the web).
  Shift hurries, limited by breath. Head bob and footsteps.
- The lantern is a shadow-casting spot light plus a faint spill light.
  - It burns oil; F closes the shutter.
  - Below 22% it sputters, and sputter frames are darkness.
  - Lamp oil tins lie about the house.

### Audio (`sfx.rs`)
- All sound is synthesized at startup.
- The ambience is wind, a tick-tock clock and a low drone.
- Positional one-shots use stereo pan and distance falloff:
  - wooden creaks when a figure moves;
  - clicks when a watcher turns;
  - the knock that starts a hunt;
  - candle snuffs;
  - a music-box chime from the nearest lot (or the ledger), for navigating by
    ear.
- A heartbeat follows the nearest hunter.

## Engine work this game drove
- **Web builds run:**
  - WebGL2-compatible device limits and shadow maps;
  - meshes, lights and cameras spawned from code are rendered;
  - content loads from the page's directory, so itch.io sub-paths work;
  - the web event loop no longer spins;
  - mouse look works.
- **Input:** taps shorter than a frame are seen and never stick; mouse
  motion accumulates.
- **`concerto-audio`:** a new crate (WebAudio backend, offline synth, stereo
  pan).
- **Rendering:** `Light::range`, camera fog and ambient override, and material
  handles can be swapped on live entities.
- **Animation:** `AnimationClipNode::with_play_rate` and `AnimationPlayer`
  pausing.
- **Time:** a `Time` scale and a clamped frame delta.

## Roadmap ideas (for future funded iterations)
- Native audio backend (cpal) so desktop builds have sound.
- A settings menu with sliders, and gamepad support.
- Clara's figure in her own finish with a veil (needs mesh attachments).
- Mirrors (render-to-texture) that show the room behind you.
- Stairs and a second floor; an attic finale.
- Daily seeded houses with a leaderboard.

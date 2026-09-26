# AFTER HOURS — design

A short first-person horror game built on Concerto, for itch.io (web first).

> You are the night-shift closer at a department store. Lock up: find the
> register keys and get out through the staff exit. The mannequins only move
> when you are not looking.

This document is the source of truth for *why* the game is the way it is.
Future work should update it alongside the code.

## Pillars

1. **One rule, fully honest.** A mannequin you can see never moves. Ever. The
   player must be able to trust this, so "see" is computed conservatively
   (wider than the real view, see *Observation*).
2. **The glance back.** Every time you look again, something has changed: a
   mannequin is closer, and holds a new pose. The pose library (120 clips) is
   the game's main scare budget.
3. **Darkness is the enemy's ally.** You only truly see what your flashlight or
   a working ceiling light illuminates. Battery, flicker and failing lights
   turn the rule against you.
4. **Short, replayable nights.** Five nights of 3–8 minutes on procedurally
   generated floors, then an endless mode. Deaths are fast; retries are instant.

## Why this game on this engine

- Only one character model exists (the UAL1 mannequin), and it is the premise.
- Darkness hides the procedural, low-detail environment art.
- The engine's strengths (skinned animation, spot-light shadows, physics
  raycasts) map directly onto the core systems; there is no combat feel to tune.
- All audio is synthesized (`concerto-audio`'s `synth`), which suits drones,
  hums and creaks.

## Core loop

1. Night intro card ("NIGHT 1 — 11:52 PM").
2. Explore the store floor, collect **keys** (3 on night 1, up to 5).
3. The **staff exit** unlocks when every key is held; reach it to end the night.
4. A mannequin touching you ends the run (jumpscare), restart the night.

## Systems

### Store generation (`level.rs`)
- Grid of square cells (4 m). Night 1 is 7×7, growing to 10×10.
- Randomized DFS maze, then a fraction of dead-end walls removed ("braided") so
  there are loops to evade through. Walls are tall shelving units that block
  sight.
- Start cell in a corner; exit and keys placed by BFS distance (far from the
  start, spread from each other).
- Pure and deterministic from a seed; unit tested for connectivity and
  placement rules.

### Observation (`mannequin.rs`, `lighting.rs`)
A mannequin is **observed** when any of its sample points (knees, chest, head)
passes all of these, from this frame's eye *or* last frame's:
- inside the camera frustum widened by a small safety margin,
- unoccluded (a physics ray from the eye reaches the point),
- **visibly lit**: the irradiance there, computed with the renderer's own
  falloff (`lighting.rs` mirrors the shader: inverse square, light range
  window, spot cone, near-zero ambient) and dimmed by the fog, is above a
  visibility threshold; or the point is within arm's reach (≈2.6 m).

Because the rule reproduces the renderer's lighting, "dark on screen" and
"unseen by the rule" agree: a mannequin can never be watched moving, and a
mannequin in genuine darkness is genuinely free.

The game writes the main camera pose itself (after physics interpolation,
before transform propagation), so the pose the rule tests is exactly the
pose that is rendered. A mannequin must be unobserved for a short grace
period before it may move.

### Mannequins (`mannequin.rs`)
- **Hunters** path toward the player over the cell grid (BFS) whenever
  unobserved. They start dormant and wake on a per-night schedule, or at
  once if the player walks up to one. Beyond three cells they only stalk,
  at reduced speed. Speed rises per night.
- Mannequins have no physics body: they move by transform, so nothing
  interpolates them on after they are seen; the player is kept out of them
  by `block_player`.
- **Decoys** never move… until the night clock wakes some of them.
- Poses: each time a hunter starts moving it takes a new pose, chosen from a
  table ordered by menace (idle → reaching → lunging) based on distance.
  Poses are single frames of clips (play rate 0); frozen mannequins pause
  their animation player entirely.
- Contact (< ~0.9 m while it moves) kills the player.

### Player (`player.rs`)
- Capsule body, WASD, mouse look (pointer lock on web), Shift to sprint with
  stamina, head bob, footsteps.
- Flashlight (F): a shadow-casting spot light on the camera. Battery drains
  while on; below 25% it flickers (flicker frames are darkness). Batteries are
  pickups.

### Nights (`night.rs`)
| Night | Grid | Keys | Hunters | Decoys | Lights working | Hunter speed | First wake / spacing |
|------:|-----:|-----:|--------:|-------:|---------------:|-------------:|---------------------:|
| 1 | 7×7 | 3 | 2 | 6 | 45% | 3.5 m/s | 25 s / 30 s |
| 2 | 8×8 | 3 | 3 | 7 | 35% | 4.0 m/s | 18 s / 22 s |
| 3 | 8×8 | 4 | 4 | 8 | 30% | 4.5 m/s | 12 s / 18 s |
| 4 | 9×9 | 4 | 5 | 9 | 25% | 5.0 m/s | 8 s / 14 s |
| 5 | 10×10 | 5 | 6 | 10 | 20% | 5.5 m/s | 5 s / 10 s |
| 6+ (endless) | 10×10 | 5 | +1/night | 10 | 15% | 6 m/s | 3 s / 8 s |

Working lights are capped at 12 per floor (fragment cost on weak GPUs).
During a night the clock advances; every 60 s a decoy wakes up, and each key
collected makes one working light start dying. Surviving night 5 shows the
"week survived" ending; play then continues endlessly.

### Audio (`sfx.rs`)
- Music bus: a fluorescent-hum + low drone loop.
- Positional one-shots (stereo pan + distance attenuation): plastic creaks when
  an unseen hunter moves, key chimes, footsteps.
- Heartbeat whose rate follows the nearest hunter's distance.
- A faint positional chime from the nearest key (or the open exit) every
  few seconds, so the maze can be navigated by ear.
- Stingers: key pickup, exit unlock, flashlight dying, the jumpscare.

## Engine work this game drove
- Web builds actually run: WebGL2-compatible device limits and shadow maps,
  meshes/lights/cameras spawned from code are rendered, content loads from
  the page's directory (itch.io sub-paths), the web event loop no longer
  spins, mouse look works on the web.
- Input: taps shorter than a frame are seen and never stick; mouse motion
  accumulates.
- `concerto-audio`: new crate (WebAudio backend, offline synth, stereo pan).
- Rendering: `Light::range`, camera fog and ambient override, material
  handles can be swapped on live entities.
- Animation: `AnimationClipNode::with_play_rate`, `AnimationPlayer` pausing.
- `Time` scale and a clamped frame delta.

## Roadmap ideas (for future funded iterations)
- Native audio backend (cpal) so desktop builds have sound.
- A proper settings menu (sliders) and gamepad support. Keyboard shortcuts
  for sensitivity, volume and invert exist on the title and pause screens.
- Store departments with distinct props (clothing racks, electronics, toys).
- Mannequins that tilt their heads toward you while frozen.
- Security-camera monitors showing other aisles (render-to-texture exists).
- A "blink" meter variant for a hard mode; daily seeded runs with a leaderboard.

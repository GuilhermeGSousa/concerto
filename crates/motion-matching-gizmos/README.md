# concerto-motion-matching-gizmos

Draws, in the running game, what motion matching compares: the simulation bone,
the last search's query, the frame it matched, and the path the capsule took. It
lives apart from `concerto-motion-matching` so that crate stays free of the
renderer. The editor's database preview draws through the same pieces.

## In a game

A game adds the crate to its own `Cargo.toml`; `concerto` doesn't re-export it.
Register the plugin after `DefaultPlugins`, because `DebugGizmosPlugin` must
follow the render plugin, and insert the component on the entity that holds
`CharacterLink`:

```rust
use concerto_motion_matching_gizmos::{MotionMatchingGizmos, MotionMatchingGizmosPlugin};

app.register_plugin(DefaultPlugins)
    .register_plugin(MotionMatchingGizmosPlugin);

world.insert(MotionMatchingGizmos::default(), character);
```

The plugin registers `DebugGizmosPlugin` if the `App` has a renderer and nothing
has registered it yet, which also brings `draw_skeleton_gizmos`, so a
`SkeletonGizmo` kept in a game's scene starts drawing in play. An `App` without a
renderer builds and draws nothing.

`draw_motion_matching` runs in game time, which includes Play in the editor, and
not while editing or in a preview. `MotionMatchingGizmos` is a reflected
component, so the inspector shows its toggles while playing, but it isn't a scene
component: a game inserts it from code.

## What each toggle draws

All four default to on.

| Toggle | Draws |
|---|---|
| `simulation` | The ring and arrow of the simulation bone, and a line from it to the character, so the gap between capsule and character shows. |
| `query` | `MotionMatchingInstance::last_query`, in each channel's palette colour, dimmed: the feature bones of the pose it read and the desired trajectory. Nothing before the first search. |
| `matched` | The features of the frame the instance plays, at full colour. Between searches it moves on with playback, and the query stays as it was. |
| `history` | A line through about the last second of the simulation bone's positions, in game time. It records while the toggle is off, so turning it on shows only that last second. A paused game adds nothing to it. |

Everything is placed at the same bone: the position of `CharacterLink::capsule`
this frame, and its facing from `CharacterSimulation::interpolated_facing`. The
published trajectory's raw body pose can be up to 22 cm ahead at a sprint, so
nothing is drawn at it.

Each layer draws through `FeatureChannel::preview`, so a game's own channels show
up here as they do in the preview. A channel with no `preview` draws nothing.

## What it skips

A character whose capsule or skeleton is gone is skipped whole. `query` and
`matched` draw nothing when the database, its skeleton or a clip is missing, when
the frame isn't in the database, or when the features don't fit its channels, so
a replaced database can't panic here. A graph with several motion matching nodes
draws one that is initialised and hasn't failed, and which one isn't defined.

## For tests and tools

`matched_shapes` and `query_shapes` return the placed `FeatureShape`s of each
channel, and the system only draws them. `GizmoStorage` keeps its vertices
private, so a test compares these and counts `GizmoStorage::segment_count()`.

`BakedFrame` owns what a `FrameContext` borrows. `placed_shape`, `draw_shape`,
`draw_simulation`, `channel_color` and the colours are the pieces the editor's
preview shares.

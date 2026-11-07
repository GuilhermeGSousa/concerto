# concerto-procedural-animation-gizmos

Draws, in the running game, where each `FootLockNode` holds a foot: the lock of
a planted foot, the ease back to the animated foot when a lock releases, and the
feet it leaves free. It lives apart from `concerto-procedural-animation` so that
crate stays free of the renderer.

## In a game

A game adds the crate to its own `Cargo.toml`; `concerto` doesn't re-export it.
Register the plugin after `DefaultPlugins`, because `DebugGizmosPlugin` must
follow the render plugin, and insert the component on the entity that holds the
`AnimationPlayer`:

```rust
use concerto_procedural_animation_gizmos::{FootLockGizmos, ProceduralAnimationGizmosPlugin};

app.register_plugin(DefaultPlugins)
    .register_plugin(ProceduralAnimationGizmosPlugin);

world.insert(FootLockGizmos::default(), skeleton);
```

The plugin registers `DebugGizmosPlugin` if the `App` has a renderer and nothing
has registered it yet. An `App` without a renderer builds and draws nothing.

`draw_foot_locks` runs in game time, which includes Play in the editor, after
`animate_targets`, where the node writes its locks, and after
`propagate_global_transforms`. `FootLockGizmos` is a reflected component, so the
inspector shows its toggle while playing, but it isn't a scene component: a game
inserts it from code.

## What it draws

`locks` defaults to on. For each leg of every `FootLockNode` in the player's
graph, from `FootLockState::legs()`, in world space:

| Phase | Draws |
|---|---|
| `Locked` | A green ring of 6 cm, flat on the ground plane, and a green cross of 4 cm, at the lock. |
| `Releasing` | An amber ring of the same size where the foot is shown, and a line from there to the animated foot. |
| `Free` | A grey cross of 4 cm at the animated foot. |

Each point is the leg's contact point: its toe when the leg names one, and its
foot otherwise. A leg is drawn as the last evaluate left it, so a node whose
legs can't be resolved draws nothing.

## For tests

`GizmoStorage` keeps its vertices private, so the crate's tests check the shapes
each leg is drawn with and count `GizmoStorage::segment_count()`.

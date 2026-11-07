# concerto-navigation-gizmos

Draws the nav graph and the route each `NavAgent` is following. It lives apart
from `concerto-navigation` so that crate stays free of the renderer.

## In a game

A game adds the crate to its own `Cargo.toml`; `concerto` doesn't re-export it.
Register the plugin after `DefaultPlugins`, because `DebugGizmosPlugin` must
follow the render plugin:

```rust
use concerto_navigation_gizmos::NavigationGizmosPlugin;

app.register_plugin(DefaultPlugins)
    .register_plugin(NavigationGizmosPlugin);
```

The plugin registers `DebugGizmosPlugin` if the `App` has a renderer and nothing
has registered it yet. An `App` without a renderer builds and draws nothing.

Both systems run in `LateUpdate` after `rebuild_nav_graph`. `draw_nav_graph`
also runs at edit time, so the editor's scene view shows the graph while points
are authored. `draw_nav_routes` runs in game time only, which includes Play in
the editor.

## What it draws

The `NavigationGizmos` resource turns each layer off; `graph` and `routes` both
default to on.

| Layer | Draws |
|---|---|
| `graph` | Each point as a ring of 30 cm on the ground plane, green when enabled and grey when disabled, with a 60 cm line along its +Z facing. Each link as a blue line between its points; a one-way link adds a two-line arrowhead at the edge of its `to` point's ring. |
| `routes` | A pink line from each agent through the waypoints its `NavRoute` has left. |

## For tests

`GizmoStorage` keeps its vertices private, so the crate's tests check the shapes
each point, link and route is drawn with and count
`GizmoStorage::segment_count()`.

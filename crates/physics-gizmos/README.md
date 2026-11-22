# concerto-physics-gizmos

Draws colliders, ragdolls, contacts and what a `BodySense` perceives, and the component gizmos of the
collider components. It lives apart from `concerto-physics` so that crate stays
free of the renderer.

## In a game

A game adds the crate to its own `Cargo.toml`; `concerto` doesn't re-export it.
Register the plugin after `DefaultPlugins`, because `DebugGizmosPlugin` must
follow the render plugin:

```rust
use concerto_physics_gizmos::{PhysicsGizmos, PhysicsGizmosPlugin};

app.register_plugin(DefaultPlugins)
    .register_plugin(PhysicsGizmosPlugin);
```

The plugin registers `DebugGizmosPlugin` if the `App` has a renderer and nothing
has registered it yet. The systems run in `LateUpdate` after
`propagate_global_transforms`, in game time only, which includes Play in the
editor. Set `PhysicsGizmos` to turn layers on; `PhysicsGizmos::all()` turns every
one on.

## What it draws

Every layer of `PhysicsGizmos` defaults to off.

| Layer | Draws |
|---|---|
| `colliders` | Each `Collider`'s shape at its entity's pose and scale, shifted by its `ColliderOffset`: a sphere, a box, a capsule, or a mesh shape's bounds as a box. Grey-blue when its body is static, cyan when kinematic, green when dynamic, violet when asleep. |
| `ragdolls` | Each part's shape at its pose, coloured the same way. At each joint, on the child part's origin: its frame as red, green and blue lines in the parent's orientation, a yellow swing cone from `normal_half_cone` and `plane_half_cone` around the twist axis, a violet arc for `twist_min` to `twist_max`, and the child's current twist axis and plane axis in white. |
| `contacts` | A red cross at each point of each `Contacts` and a line of 30 cm along its normal. |
| `sensing` | For each `BodySense`: its centre of mass as a yellow sphere with a line down to `ground`, the support outline as a closed white polyline, the capture point as a cross on the ground with a line from the centre's ground projection (green when `balanced()`, red otherwise), the centre's velocity as a blue arrow, and each chest or head impact as an orange cross with a line along its normal. |

## In the editor

The component gizmos are the editor's way to turn the layers on. Kinds:
`Collider`, `Ragdoll` (on the entity holding the `RagdollId` that
`create_ragdoll` returned, which the caller inserts on the owner), `Contacts`,
`Box collider`, `Mesh collider`, `Sensor` and `Body sense`. The first three draw what the
`colliders`, `ragdolls` and `contacts` layers draw, for one entity, and `Body sense` what the `sensing` layer draws. The next
three draw from the authored components, since an edit `App` creates no bodies:
the box of a `BoxCollider`, the bounds of a `MeshCollider`'s mesh, and the shape
a `Sensor` detects in, in orange.

The editor draws a kind while the entity is selected, or for every entity with
`gizmos kind=<name> always=true` or the viewport's Gizmos menu, once the game
lists [`concerto-physics-editor`](../physics-editor/), which registers them in
every edit and play `App`. A game without the editor sets `PhysicsGizmos`.

## For tests

`GizmoStorage` keeps its vertices private, so the crate's test counts
`GizmoStorage::segment_count()` for each layer.

# System scheduling

Systems live in named schedules (`Update`, `LateUpdate`, `Render`, ...). Within a
schedule, their order comes from explicit constraints and, where there are none,
from registration order. This page covers the API for both.

## Imports

The examples below assume these traits are in scope:

```rust
use concerto_ecs::{IntoSetConfig, IntoSystemConfig};
```

- `IntoSystemConfig` gives a system, or a tuple of systems, `.after`, `.before`,
  `.in_set` and `.chain`.
- `IntoSetConfig` gives a set, or a tuple of sets, `.after`, `.before` and `.chain`.

## Adding systems

```rust
app.add_system(Update, my_system);
app.add_system(Update, (a, b, c));
```

`add_system` takes a system, a `SystemConfig`, a `Vec<SystemConfig>`, or a tuple of
up to twelve of any of these, nested as deeply as needed. The render subapp has
`add_render_system`.

## Ordering one system after another

```rust
app.add_system(Update, a);
app.add_system(Update, b.after(a));
app.add_system(Update, c.before(a));
```

`.after` and `.before` **reference** their target; they do not register it.
Someone has to add `a` to the same schedule, and it runs once no matter how many
constraints name it. The target may be registered before or after the constraint:
nothing is resolved until the schedule compiles.

`.chain()` orders a tuple of systems pairwise, and `.after`, `.before` and `.in_set`
on a tuple apply to every system in it. A nested tuple is chained as a unit, so here
both movement systems run after `read_input` and before `update_camera`:

```rust
app.add_system(Update, (read_input, (move_player, move_enemies), update_camera).chain());
```

## Declaring a set

A set is a named group of systems that can be ordered as a unit. Derive
`SystemSet` on an enum (or unit struct):

```rust
use concerto_ecs::SystemSet;

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum UiSet {
    Input,
    Widgets,
    Setup,
    Project,
    Materials,
    Layout,
    PostLayout,
}
```

Make it `pub`: the point of a set is that other crates can order against it
without seeing the systems inside it.

## Ordering sets

```rust
app.configure_sets(
    LateUpdate,
    (UiSet::Input, UiSet::Widgets, UiSet::Layout).chain(),
);
app.configure_sets(LateUpdate, UiSet::Materials.after(UiSet::Input).before(UiSet::Layout));
```

A set constrained with `A.before(B)` orders every member of `A` before every
member of `B`. Configuring the same set more than once accumulates constraints
rather than replacing them, so two crates can each constrain a shared set.
Sets are configured per schedule; the render subapp uses `configure_render_sets`.

## Joining a set

```rust
app.add_system(LateUpdate, (sync_tab_bodies, update_tooltips).in_set(UiSet::Widgets));
app.add_system(LateUpdate, compute_ui_nodes.in_set(UiSet::Layout));
```

A system may belong to several sets. Constraints on the system itself still apply
alongside its sets':

```rust
app.add_system(
    LateUpdate,
    (track_panel_stack, compute_ui_nodes.after(track_panel_stack)).in_set(UiSet::Layout),
);
```

## Ordering against another crate's set

This is what sets are for. A crate that depends on `ui` can slot a system between
two UI phases without naming a UI system and without depending on plugin
registration order:

```rust
app.add_system(LateUpdate, app_widget.after(UiSet::Input).before(UiSet::Layout));
```

The render crate exports `RenderSet { Lights, Shadows, Draw, Present }` the same
way.

## The "constraint is ignored" warning

```
`my_game::b` is ordered against `my_game::a`, which is not in this schedule; the constraint is ignored
```

The system a constraint names is not registered in that schedule. The usual
causes are:

- the plugin that registers the target was not added to the app, or
- the target is registered in a different schedule (or subapp) — constraints
  never reach across schedules.

The constraint is dropped; the target is not added for you. A **set** with no
members is silent by design: a plugin contributing no systems to a set is normal.

Naming a system that was added more than once orders against every copy and logs
a separate warning.

## Rules

- Conflicting systems (overlapping data access) with no explicit constraint
  between them still run in registration order.
- An explicit constraint always wins over registration order.
- Contradictory explicit constraints panic at compile with
  `Cycle in schedule ordering`, naming the systems on the cycle.
- Sets are flat: a set cannot contain another set.
- Closures run fine but cannot be ordering targets — there is no way to name
  them again. Use a named `fn` for anything that needs to be referenced.
- The same goes for function pointers: every `fn(ResMut<Score>)` pointer shares one
  type, so they cannot be told apart. This is why `Vec<SystemConfig>` is accepted but
  a `Vec` of bare functions is not — collecting functions into a `Vec` coerces them
  to pointers. Call `.into_config()` on each before collecting.

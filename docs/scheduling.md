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
  `.in_set`, `.chain` and `.tagged`.
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

The render crate exports `RenderSet { Lights, Shadows, Draw, Overlay, Present }` the
same way: `Lights`, `Shadows`, `Draw` and `Overlay` are chained in that order in
`Render`, `Overlay` draws over the finished scene before any pass that samples a
camera's target, and `Present` runs in `LateRender`, after all of them. The window
crate exports `FoldInput`, the set in `First` that folds
window events into `Input`; a system that decides `InputGate` for the frame runs
before it.

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

## Tags

A tag is a type deriving `SystemTag`; bring the trait into scope for the derive, as
with `SystemSet`. A system carries tags through `.tagged(MyTag)`, and on a tuple it
tags every system in it:

```rust
#[derive(SystemTag, Clone, PartialEq, Eq, Hash, Debug)]
struct Preview;

app.add_system(Update, (spin, bob).tagged(Preview));
```

Tags never affect ordering, and `compile` ignores them. `compile_filtered` takes a
`TagFilter { include, exclude }`: a system runs when `include` is empty or it carries
an included tag, and it carries no excluded tag. The others stay in the schedule as
stand-ins. Explicit orderings through them hold, they declare no access and add no
sync point, and they do nothing when run. The implicit edges a replaced
system's own data access or sync point would have added are gone, but a stand-in is
still ordered by registration against sync points and against systems that take the
whole world, because empty access still conflicts with whole-world access. A
component only stand-ins would have queried is never registered.

## Edit time

An `App` created with `App::for_editing()` is an edit `App`. `concerto-app`'s
`AtEditTime` trait gives `.at_edit_time()`: on a system, or a tuple of systems, it adds
the `EditTime` tag to each, and on a listener it marks it.

```rust
use concerto_app::edit_time::AtEditTime;

app.add_system(LateUpdate, propagate.at_edit_time());
app.add_listener(on_select.at_edit_time());
```

- An edit `App` compiles its main world with `include: [EditTime]`, so every unmarked
  main-world system is a stand-in there. Its render world compiles without a filter.
- In an edit `App`, `App::add_listener` adds only marked listeners and drops the others
  without a word. A listener added through `World::add_listener` or inserted directly
  on an entity is never filtered, even from a plugin's `build`.
- Component lifecycle callbacks and plugin `build` are outside the mark. A plugin may
  check `app.is_editing()` in `build` only to skip resources its unmarked systems alone
  use.
- `register_event` and `register_asset` mark the systems they add.
- An edit `App` has no `PhysicsPipeline`, so a marked system must not require it;
  `PhysicsState` is present and empty.

## Change detection

`Changed<T>`, `Added<T>` and `is_changed()` mean "since this system last ran",
whatever the order of writer and reader within a frame:

- A write made after a reader ran, including a command applied at a later sync
  point or a direct `World` write between frames, is seen on the reader's next run.
- A system does not see its own `Mut` or `ResMut` writes on its next run.
- A system's first run sees every entity and resource stamped within
  `MAX_CHANGE_AGE` as added and changed.
- Overwriting a component with `insert`, or a resource with `insert_resource`,
  marks it changed and keeps its added tick, as in Bevy: `Added<T>` and
  `is_added()` are true again only after a remove and a fresh insert.
- A run skipped because an input declined keeps what changed for the next run that
  executes.
- A listener sees what changed since its own previous run.
- A query made directly on a `World` sees what changed since the last
  `world.tick()`, which `Main` and `RenderMain` call once per frame.

Ticks are 32-bit and wrap. As in Bevy, every schedule run starts with
`world.check_change_ticks()`, which, once every `CHECK_TICK_THRESHOLD` counter
advances, clamps every component stamp, resource stamp, every stored
schedule's systems, and those of the schedule whose run made the check, to at
most `MAX_CHANGE_AGE` old, and triggers the `CheckChangeTicks` signal. Ticks
kept anywhere else clamp themselves in a `CheckChangeTicks` listener with
`Tick::check_tick`. Listeners' own ticks are not clamped, as Bevy leaves
observers: a listener idle for about `MAX_CHANGE_AGE` counter advances
can misreport changes on its next run.

A test that checks change detection through a system must keep one system instance
across runs. A new instance is a first run and sees everything stamped within
`MAX_CHANGE_AGE`.

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

# Removing `World::add_child`

`World::add_child` is the only hierarchy operation the ECS exposes as a
world method. It exists because inserting `ChildOf` is not enough on its
own: the component's `on_add` callback attaches the child to its new
parent, but nothing detaches it from the old one, so `add_child_internal`
has to remove `ChildOf` first and insert it second.

This design makes `insert(ChildOf)` complete by itself and moves the
ergonomic entry point onto an entity builder, following Bevy's
relationship components.

## Goals

- `world.insert(ChildOf::new(parent), child)` maintains both sides of the
  relationship, whatever the child's previous parent was.
- `World` carries no hierarchy-specific method.
- Deferred call sites (`cmd.add_child`, `add_child_with`,
  `spawn_child_queue`) keep their signatures.

## Non-goals

- A general `EntityWorldMut` covering insert/remove/despawn. Scope is
  hierarchy only.
- `with_children(|parent| ...)` closures. The deferred API already builds
  trees ergonomically; immediate-mode callers are mostly tests.
- Generalising `ChildOf`/`Children` into a `Relationship` trait.

## 1. The `on_replace` lifecycle callback

`Component` gains a fourth callback:

```rust
/// Optional callback invoked before an existing value is overwritten by an
/// insert, while the old value is still readable. Not invoked by removal or
/// despawn, nor by the first insert of the component.
fn on_replace() -> Option<ComponentLifecycleCallback> { None }
```

It joins `ComponentLifecycleCallbacks` and gets a
`UnsafeWorldCell::trigger_on_replace_component` alongside the existing
three triggers.

`insert_internal` fires it for every inserted component the entity already
carries, before any value is written. The replaced set is the intersection
of `T::get_component_ids()` with the source archetype's component ids, and
must be computed before the local `component_ids` vector is extended with
the newly added ids:

```rust
let replaced: Vec<ComponentId> = inserted_ids
    .iter()
    .copied()
    .filter(|id| self.archetypes[source_index].component_ids().contains(id))
    .collect();
```

The trigger runs under `if trigger_events`, immediately after that
computation and before the archetype move. Callbacks see the entity in its
pre-insert state, which is what makes the old value readable.

Resulting order for an insert over an existing component:
`on_replace` (old value) → write → `on_add` (new value).

`on_add` keeps its current meaning — it fires on every insert, not only the
first — so no existing callback changes behaviour.

Unlike Bevy, removal does not also fire `on_replace`. `on_remove` already
exists here and does that job; two callbacks with one body are cheaper than
collapsing them.

## 2. `ChildOf` owns the whole relationship

| callback | body |
|---|---|
| `on_add` | attach `context.entity` to the new parent's `Children` |
| `on_replace` | detach from the old parent |
| `on_remove` | detach |
| `on_despawn` | detach |

Three of the four are the same `detach` function, so `hierarchy.rs` holds
one `attach` and one `detach` and the four callbacks return them.

`Children::on_despawn` (the cascade) is unchanged.

## 3. Emptying `Children` must be re-checked at flush time

`detach` removes the child from the parent's `Children` immediately, then
queues removal of the now-empty `Children` component — removal is
structural, so it cannot happen inside a callback.

Queueing an unconditional removal is wrong, and is a latent bug in the code
as it stands today: between the queueing and the flush, another child can
be attached to that same parent, and the flush then drops a `Children`
holding a live child. Reparenting a lone child onto its own current parent
hits exactly this path (`on_replace` empties `Children` and queues the
removal, `on_add` re-attaches into the still-present component, the queued
removal fires), so it stops being hypothetical the moment `insert(ChildOf)`
becomes the reparenting mechanism.

The fix is a command that re-checks at execution, living in `hierarchy.rs`
beside the components it serves:

```rust
pub(crate) struct RemoveEmptyChildren { parent: Entity }

impl Command for RemoveEmptyChildren {
    fn execute(self, world: &mut World) {
        let is_empty = world
            .get_component_for_entity::<Children>(self.parent)
            .is_some_and(Children::is_empty);
        if is_empty {
            world.remove_with_events::<Children>(self.parent, true);
        }
    }
}
```

`detach` needs a way to queue it from a `RestrictedWorld`, which no longer
exposes `commands()`. `RestrictedWorld` gains
`pub(crate) fn queue_command<C: Command>(&mut self, command: C)`, forwarding
to a new `pub(crate) CommandQueue::queue` — both visible only inside the ecs
crate, which is where the hierarchy components live.

## 4. `EntityWorldMut`

A borrow of one entity, in `crates/ecs/src/entity/`:

```rust
pub struct EntityWorldMut<'w> {
    world: &'w mut World,
    entity: Entity,
}

impl World {
    pub fn entity_mut(&mut self, entity: Entity) -> EntityWorldMut<'_>;
}

impl EntityWorldMut<'_> {
    pub fn id(&self) -> Entity;
    pub fn add_child(&mut self, child: Entity) -> &mut Self;
    pub fn add_children(&mut self, children: &[Entity]) -> &mut Self;
    pub fn spawn_child<T: ComponentBundle>(&mut self, bundle: T) -> EntityWorldMut<'_>;
}
```

`add_child` is `self.world.insert(ChildOf::new(self.entity), child)` plus
one guard: if the child's current parent is already `self.entity`, return
without touching anything. Section 3 makes that case *correct* without the
guard; the guard makes it *free*, avoiding a pointless detach/attach pair
and, for a lone child, an archetype migration of the parent.

`entity_mut` does not validate the entity. `insert` already panics on a
stale entity, which is the existing contract.

`World::add_child` and `add_child_internal` are deleted, along with the
`Children` and `ChildOf` imports in `world.rs`.

## 5. Command side

`CommandQueue::add_child(parent, child)` keeps its signature and becomes
`self.insert(ChildOf::new(parent), child)`. The `AddChild` command type
(`command.rs`) is deleted. `EntityCommandQueue::add_child`,
`add_child_with` and `spawn_child_queue` are untouched, so the ~70 deferred
call sites do not change.

## 6. Migration

18 immediate call sites become `world.entity_mut(parent).add_child(child)`:

- `crates/ecs/src/lib.rs` (2), `crates/ecs/tests/hierarchy_despawn.rs` (5),
  `crates/ecs/tests/despawn_callbacks.rs` (5),
  `crates/ecs/tests/deferred_callbacks.rs` (1)
- `crates/editor/src/inspector/tests.rs`, `crates/editor/src/asset_editor.rs`,
  `crates/editor/tests/tabs.rs`
- `crates/essential/src/transform/systems.rs`

Every one of those already registers `ChildOf` and `Children`, so no
registration changes are needed.

## 7. Testing

New, written before the change:

- `on_replace` fires with the old value readable, and does not fire on a
  first insert (`crates/ecs/tests/component_lifecycle.rs`).
- `world.insert(ChildOf::new(new_parent), child)` reparents correctly with
  no `add_child` involved — the property that justifies the change.
- Re-adding a child to its current parent leaves `Children` intact, both
  through `entity_mut(..).add_child(..)` and through a raw `insert`. The raw
  re-insert is the regression test for section 3: with an unconditional
  removal it fails, since `on_replace` empties `Children` and `on_add` then
  re-fills the component the queued removal is about to drop.

Existing coverage carries the rest: the cascade and reparenting tests in
`hierarchy_despawn.rs`, `despawn_callbacks.rs`, and the scene spawner,
which goes through `cmd.add_child`.

## Risks

- A component with an `on_add` that assumed "runs once per entity" would
  now see `on_replace` first; none exists today, and `on_add`'s own
  semantics are unchanged.
- `on_replace` fires before the archetype move, so a callback that queues
  structural work sees the pre-insert archetype. That matches `on_remove`,
  which already runs before its removal.

## 8. Follow-up: `trigger_events` removed

Every engine call site passing `false` did so for a component with no
callbacks at all (`GlobalTransform`, the render slots), so the flag bought
one lookup against an empty hook entry. With `on_replace` in place it also
became a way to break relationship invariants from safe code: a suppressed
insert over `ChildOf` skips the detach and strands the child in its old
parent's `Children`.

Gone, therefore: `World::insert_with_events`/`remove_with_events`,
`CommandQueue::insert_with_events`/`remove_with_events`, the
`trigger_events` fields on `InsertCommand`/`RemoveCommand`, and the bool on
`RestrictedWorld::insert`/`remove_component`. `insert_internal` and
`remove_component_internal` always trigger.

Two tests changed with it: `suppressed_insert_does_not_fire_callbacks` and
its `Quiet` component are deleted, and `on_remove_can_queue_companion_removal`
now expects the companion's own `on_remove` to fire.

## 9. Follow-up: despawn implies removal

Six components carried a literal `fn on_despawn() { Self::on_remove() }`,
`ChildOf` among them, because despawning fired only `on_despawn`.

`trigger_on_despawn` now fires each component's `on_remove` before its
`on_despawn`, all still within the pass over the ids captured at despawn
start, so every component remains present throughout. The delegating
`on_despawn` impls are deleted from `ChildOf`, `VirtualCamera`, `Collider`,
`Skeleton`, `RenderLight` and the shadow component.

`on_despawn` narrows to teardown that only makes sense for a dying entity.
`Children::on_despawn` — the cascade — is its only engine user, and must
stay distinct: removing a `Children` component from a parent should not
despawn that parent's children.

`Transform::on_remove` newly fires during a despawn; it only queues a
`GlobalTransform` removal, which `RemoveCommand` drops on the dead entity.

`despawn_and_component_removal_run_only_their_respective_hooks` is renamed
to `despawn_runs_removal_cleanup_before_its_own_hook` and expects
`["remove", "despawn"]`; component removal still expects `["remove"]`.

# Required components

A component can declare components it requires. Spawning or inserting it also
gives the entity those components, unless the entity, or the same bundle,
already supplies them.

## Declaring a requirement

```rust
#[derive(Component)]
#[require(UINode)]
struct UIText { /* … */ }

#[derive(Component)]
#[require(UINode = UINode::default(), SyncWithRenderWorld)]
struct Foo;
```

`#[require(Type)]` builds the required component with `Default::default()`.
`#[require(Type = expr)]` evaluates `expr`. The attribute may repeat. A type
with no `Default` fails to compile with the ordinary trait-bound error.

Where the attribute cannot be used, override `Component::register_required`
and call `required.register::<U>(constructor)` for each requirement. The world
grid does this.

## Registering a requirement from outside

A type you do not own can be given a requirement from `App` or `World`:

```rust
app.register_required_components::<MeshComponent, SyncWithRenderWorld>();
app.register_required_components_with::<MeshComponent, Tint>(|| Tint::red());
```

`RenderPlugin` uses this for meshes and skeletons, so a mesh gets
`SyncWithRenderWorld` only when `RenderPlugin` is built.

- It panics if an entity already holds `T` or a component that requires it.
- It panics if the requirement forms a cycle.
- A requirement `T` declares itself wins over an external one.
- A later external registration wins over an earlier one.

## Behaviour

1. A required component the entity or the same bundle already has is untouched.
   Across separate inserts the order matters: inserting `UIText` and then
   `UINode` adds a default `UINode` first, which the second insert replaces
   (`Replace<UINode>`, then `Add<UINode>`; `Added<UINode>` stays false for the
   second write). Scene loading inserts one component at a time, so a scene is
   in this case.
2. Removing the requirer leaves the required components.
3. Requirements are transitive. A cycle panics and names the types. Because
   registration is lazy, the panic comes on the first spawn or insert of the
   offending type; `World::register_component` does it at startup.
4. Lifecycle callbacks run deepest requirement first, then the requirer, then
   the other bundle members, so a requirer's `on_add` and `On<Add<_>>` see its
   requirements present.
5. Implied components take the operation's change tick, so `Added<Required>`
   matches them.
6. The editor's add-component action calls `World::insert` and expands
   requirements with no change.

When two requirers name the same component, the nearest requirer's constructor
wins over a deeper one, and a value the bundle supplies wins over any
constructor.

## How it differs from Bevy

The syntax and the rules are Bevy's. The difference is where the table lives:
this ECS has no `Components` registry type, so `World` keeps it beside the
component lifetimes, and lifecycle callbacks fire per component id, not per
bundle.

# 3. Entities, components and systems

Concerto stores the game as data in a **world**:

- An **entity** is an id, `Entity`. On its own it is nothing.
- A **component** is a plain Rust value attached to an entity. An entity is
  whatever its components say it is: a `Transform` and a `Light` make a light.
- A **resource** is a single value owned by the world: the clock, the input,
  the physics state.
- A **system** is a function that reads and writes components and resources.
  The engine calls it every time its schedule runs.

## Components

```rust
use concerto::ecs::Component;

#[derive(Component)]
struct Health {
    current: f32,
    max: f32,
}

#[derive(Component)]
struct Enemy;
```

A component with no fields, like `Enemy`, is a **marker**: it tags entities so
queries can find them.

## Spawning

Systems spawn through a `CommandQueue`. A tuple of components is a bundle:

```rust
use concerto::{
    ecs::CommandQueue,
    foundation::transform::Transform,
};

fn spawn_enemy(mut cmd: CommandQueue) {
    let enemy = cmd
        .spawn((
            Enemy,
            Health { current: 50.0, max: 50.0 },
            Transform::default(),
        ))
        .entity();
}
```

`spawn` returns a handle to the new entity: `.entity()` gives its id,
`.add_child(bundle)` spawns a child under it, and `.insert(component)` adds
more. Other commands:

```rust
cmd.entity(enemy).insert(Burning);
cmd.remove::<Burning>(enemy);
cmd.despawn(enemy);
cmd.insert_resource(Score(0));
```

Commands are not applied immediately: they are queued and applied at the
system's sync point, with exclusive access to the world. An entity spawned in
a system is visible to systems that run after that point, not to the rest of
the system that spawned it.

## Parents and children

`add_child` makes a hierarchy. A child's `Transform` is relative to its
parent's; the engine computes every entity's world transform into its
`GlobalTransform` in `LateUpdate`.

```rust
cmd.spawn((CameraRig, Transform::default()))
    .add_child((VirtualCamera::default(), Transform::from_translation(Vec3::Z * 10.0)));
```

`add_child_with(bundle, |child| { ... })` spawns a child and lets you add its
own children in the closure. A parent has a `Children` component listing them,
and a child has `ChildOf`. Despawning a parent despawns its children.

## Systems and queries

A system is a function whose parameters say what it needs:

```rust
use concerto::{
    ecs::{Query, Res, With},
    foundation::time::Time,
};

fn regenerate(time: Res<Time>, enemies: Query<&mut Health, With<Enemy>>) {
    let dt = time.delta().as_secs_f32();
    for mut health in enemies.iter() {
        health.current = (health.current + 5.0 * dt).min(health.max);
    }
}
```

A `Query<D, F>` has a **data** part and an optional **filter**:

| Data | Gives |
|---|---|
| `&T` | read access to `T` |
| `&mut T` | write access to `T` |
| `Option<&T>` | `T` if the entity has it |
| `Entity` | the entity's id |
| `(A, B, ...)` | several at once |

| Filter | Keeps entities that |
|---|---|
| `With<T>`, `Without<T>` | have, or lack, `T` |
| `Added<T>` | got `T` since this system last ran |
| `Changed<T>` | got or changed `T` since this system last ran |
| `Or<(A, B)>` | match either filter |

`query.iter()` visits every match; `query.get_entity(entity)` returns one
entity's data, or `None`:

```rust
fn hit(targets: Query<(Entity, &mut Health)>, attacks: Query<&Attack>) {
    for attack in attacks.iter() {
        if let Some((_, mut health)) = targets.get_entity(attack.target) {
            health.current -= attack.damage;
        }
    }
}
```

The parameters a system may take:

| Parameter | For |
|---|---|
| `Query<D, F>` | components |
| `Res<T>`, `ResMut<T>` | a resource; the system is skipped, with a warning, while it is missing |
| `Option<Res<T>>`, `Option<ResMut<T>>` | a resource that may not exist |
| `CommandQueue` | spawning, despawning, inserting, removing, triggering signals |
| `EventReader<E>`, `EventWriter<E>` | events, see the next chapter |
| `SystemLocal<T>` | state private to this system, kept between runs; `T` implements `FromWorld` |

A system takes at most twelve parameters; group some in a tuple if you need
more. Systems never take the `World` itself: work that needs the whole world
goes through commands or a signal listener.

## Resources

```rust
use concerto::ecs::{ResMut, Resource};

#[derive(Resource, Default)]
struct Score(u32);

fn count_kills(mut score: ResMut<Score>, killed: Query<&Health, (With<Enemy>, Changed<Health>)>) {
    for health in killed.iter() {
        if health.current <= 0.0 {
            score.0 += 1;
        }
    }
}
```

Insert it in the recipe with `app.insert_resource(Score::default())`. The
engine's own resources include `Time`, `Input`, `AssetServer`, `PhysicsState`,
`UITheme` and `Window`.

## Change detection

Every component and resource remembers when it was last written. `Changed<T>`
and `Added<T>` filter queries by it, and `is_changed()` and `is_added()` (from
the `concerto::ecs::query::change_detection::DetectChanges` trait) answer it
for a `Res`, a `ResMut` or a `Ref<T>`:

```rust
fn show_score(score: Res<Score>, labels: Query<&mut UIText, With<ScoreLabel>>) {
    if !score.is_changed() {
        return;
    }
    for mut label in labels.iter() {
        label.text = format!("Score: {}", score.0);
    }
}
```

"Changed" always means *since this system last ran*, whatever order systems
run in. Writing through `&mut T` or `ResMut<T>` marks the value changed even
if you wrote the same value, so compare before writing when it matters:

```rust
if label.text != text {
    label.text = text;
}
```

Use change detection for "this state changed". For "this happened", use an
event, and for "do this", a signal: both are in the next chapter.

Next: [Events and signals](04-events-and-signals.md).

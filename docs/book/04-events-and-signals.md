# 4. Events and signals

Systems do not call each other. When one has something to tell the rest of the
game, it uses one of two tools:

- An **event** says *something happened*. Any number of systems read it, on
  their own schedule.
- A **signal** says *do this now*. It runs its listeners at the next sync
  point, and a listener can do work no ordinary system can.

## Events

```rust
use concerto::ecs::{Entity, Event};

#[derive(Event)]
struct Died {
    entity: Entity,
}
```

Register the type in the recipe. Until it is registered, a system that reads
or writes it is skipped, with a warning every frame:

```rust
app.register_event::<Died>();
```

Write with an `EventWriter`, read with an `EventReader`:

```rust
use concerto::ecs::events::{event_reader::EventReader, event_writer::EventWriter};

fn check_deaths(enemies: Query<(Entity, &Health)>, mut died: EventWriter<Died>) {
    for (entity, health) in enemies.iter() {
        if health.current <= 0.0 {
            died.write(Died { entity });
        }
    }
}

fn award_points(mut died: EventReader<Died>, mut score: ResMut<Score>) {
    for _ in died.read() {
        score.0 += 10;
    }
}
```

Each reader keeps its own cursor, so every reader sees every event once.
Events are kept for two frames: a reader that runs every frame never misses
one, wherever it runs relative to the writer.

## Signals

A signal is a value you trigger; listeners react to it.

```rust
use concerto::ecs::signal::{On, Signal};

struct Explode {
    at: Vec3,
    radius: f32,
}

impl Signal for Explode {}
```

A **global listener** is a function whose first parameter is `On<S>`, followed
by any system parameters. Add it in the recipe:

```rust
fn explode(on: On<Explode>, targets: Query<(&Transform, &mut Health)>) {
    let explosion = on.signal();
    for (transform, mut health) in targets.iter() {
        if transform.translation.distance(explosion.at) < explosion.radius {
            health.current -= 30.0;
        }
    }
}

app.add_listener(explode);
```

Trigger it from any system with commands:

```rust
cmd.trigger(Explode { at: Vec3::ZERO, radius: 5.0 });
```

The listeners run when the command queue is applied. A listener may itself use
a `CommandQueue`, which is how work that needs exclusive access to the world is
done: the system triggers a signal, and the listener's commands are applied
with the world to themselves.

### Signals aimed at one entity

A signal that also implements `EntitySignal` can be sent to one entity, and
the entity carries the listener as a component:

```rust
use concerto::ecs::signal::{EntitySignal, listener::IntoListener};

struct Damage(f32);

impl Signal for Damage {}
impl EntitySignal for Damage {}

fn take_damage(on: On<Damage>, targets: Query<&mut Health>) {
    if let Some(mut health) = targets.get_entity(on.entity()) {
        health.current -= on.signal().0;
    }
}

cmd.spawn((Health { current: 100.0, max: 100.0 }, take_damage.into_listener()));
cmd.entity(target).trigger(Damage(12.0));
```

`on.entity()` is the entity whose listener is running. An entity carries at
most one listener per signal type. The UI uses this for its own events: a
button's `on_click` is a `Listener<UIClick>` on the button (chapter 8).

A listener that needs data about the entity it sits on captures it in its
closure instead of reading it back from a component:

```rust
fn damage_by(factor: f32) -> Listener<Damage> {
    (move |on: On<Damage>, targets: Query<&mut Health>| {
        if let Some(mut health) = targets.get_entity(on.entity()) {
            health.current -= on.signal().0 * factor;
        }
    })
    .into_listener()
}
```

`Listener` is in `concerto::ecs::signal::listener`.

### Lifecycle signals

The engine triggers a signal when a component is added, replaced, removed, or
its entity despawned: `Add<T>`, `Replace<T>`, `Remove<T>` and `Despawn<T>`, in
`concerto::ecs::signal::lifecycle`. Each carries the `entity`:

```rust
use concerto::ecs::signal::lifecycle::Add;

fn greet(on: On<Add<Enemy>>) {
    println!("an enemy appeared: {:?}", on.signal().entity);
}

app.add_listener(greet);
```

Use them to keep data in step with a component's presence, rather than polling
every frame with `Added<T>`.

## Which one?

| You want to say | Use |
|---|---|
| "This value is different now" | change detection: `Changed<T>`, `is_changed()` |
| "This happened" | an event |
| "Do this", or work that needs the whole world | a signal |

Do not add a counter that is bumped to say "this changed" and compared
elsewhere: one of the three above always says it better.

Next: [Assets and scenes](05-assets-and-scenes.md).

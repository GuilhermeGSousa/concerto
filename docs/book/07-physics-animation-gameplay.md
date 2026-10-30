# 7. Physics, animation and gameplay

## Physics

`PhysicsPlugin`, in `DefaultPlugins`, simulates the world with Jolt natively
and Rapier in the browser. Your code is the same on both.

### Bodies and colliders

A `Collider` gives an entity a shape. Alone, it is static: walls, floors and
props that things bump into. With a `RigidBody` as well, the entity is
simulated: gravity moves it, collisions push it, and the engine writes the
result back into its `Transform`.

```rust
use concerto::physics::{collider::Collider, rigid_body::RigidBody, shape::MeshCollider};

cmd.spawn((
    RigidBody::default(),
    Collider::sphere(1.0),
    Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)),
));

cmd.spawn((Collider::cuboid(9.0, 1.25, 0.5), Transform::default()));

cmd.spawn((MeshCollider, MeshComponent { handle: floor_mesh }, floor_transform));
```

| Shape | Constructor |
|---|---|
| Sphere | `Collider::sphere(radius)` |
| Box | `Collider::cuboid(half_x, half_y, half_z)`, half extents |
| Capsule | `Collider::capsule(half_height, radius)` |
| The entity's own mesh, static | `MeshCollider`, next to a `MeshComponent` |

`RigidBody` has a `density`, a `motion_type` (`Dynamic` by default) and
`allowed_dofs`, which can lock rotation: a character uses
`AllowedDofs::TRANSLATION | AllowedDofs::ROTATION_Y` so it never tips over. A
`ColliderOffset` moves the shape relative to the entity;
`ColliderOffset::bottom_origin(&collider)` puts the entity's origin at the
bottom of a capsule, where a character's feet are.

### Queries and forces

The `PhysicsState` resource answers questions about the simulated world and
takes forces:

```rust
use concerto::physics::physics_state::PhysicsState;

fn shoot(physics: Res<PhysicsState>, guns: Query<&GlobalTransform, With<Gun>>) {
    for gun in guns.iter() {
        let forward = gun.matrix().transform_vector3(Vec3::NEG_Z);
        if let Some(hit) = physics.cast_ray(gun.translation(), forward * 100.0) {
            println!("hit {:?} at {}", hit.entity, hit.point);
        }
    }
}
```

`cast_ray(origin, direction_and_length)` returns the first hit, with its
point and, when it hit a body with an entity, that `entity`. A simulated
entity gets a `BodyId`, which `physics.add_impulse(body_id, impulse)` takes.

### Characters

`CharacterSimulation` springs a dynamic body toward the `DesiredMotion` that
other systems write, and `GroundProbe` reports whether it stands on something:

```rust
use concerto::motion_matching::{CharacterSimulation, DesiredMotion};
use concerto::physics::ground::GroundProbe;

let collider = Collider::capsule(2.0, 1.0);
let offset = ColliderOffset::bottom_origin(&collider);
cmd.spawn((
    Player,
    RigidBody {
        density: 1000.0,
        allowed_dofs: AllowedDofs::TRANSLATION | AllowedDofs::ROTATION_Y,
        motion_type: MotionType::Dynamic,
    },
    CharacterSimulation::default(),
    collider,
    offset,
    GroundProbe::default(),
    Transform::from_translation(spawn_point),
));

fn walk(input: Res<Input>, players: Query<(&mut DesiredMotion, &GroundProbe), With<Player>>) {
    let forward = if input.is_held(PhysicalKey::Code(KeyCode::KeyW)) { 1.0 } else { 0.0 };
    for (mut desired, ground) in players.iter() {
        if ground.is_grounded() {
            desired.velocity = Vec3::new(0.0, 0.0, -forward * 5.0);
        }
    }
}
```

Each fixed step the simulation advances its velocity and facing springs
(halflife 0.27 s by default) toward `DesiredMotion`, sets the body's
horizontal velocity to the spring's, and reads the body's position back after
collisions. It also fills `CharacterSimulation::trajectory` with
`sample_count` predicted samples, `sample_dt` apart, assuming the desired
motion holds.

The default halflife changed from 0.1 s, the old `CharacterMovement` value, to
0.27 s.

`ground.ground()` says more: `InAir`, `OnGround` or `OnSteepGround`. Input
comes from the `Input` resource in `concerto::window::input`, with
`is_held`, `is_just_pressed` and `is_just_released` for keys and the matching
`is_mouse_button_*` methods for the mouse.

The physics step runs in `LateFixedUpdate`, at a fixed rate; transforms of
simulated bodies are interpolated between steps for display.

## Animation

A skinned model imported from glTF comes with a skeleton asset (bone names, parents, rest pose, inverse bind poses and bone ids, in `concerto::mesh`) and its animation
clips. When its scene spawns, the skinned entity gets an `AnimationPlayer`.
You give it an **animation graph**, which decides which clips play and how they
blend, and you drive the graph with **parameters**.

```rust
use concerto::animation::{
    clip::AnimationClip,
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
};

fn setup_animations(
    players: Query<Entity, (With<AnimationPlayer>, Without<AnimationsReady>)>,
    assets: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    let Some(entity) = players.iter().next() else {
        return;
    };
    let idle = assets.load::<AnimationClip>(IDLE);
    let walk = assets.load::<AnimationClip>(WALK);
    let run = assets.load::<AnimationClip>(RUN);

    let mut graph = AnimationGraph::new();
    graph.result_node().with_blend_space_2d_input("movement", |space| {
        space
            .animation_clip_input(idle, Vec2::ZERO)
            .animation_clip_input(walk, Vec2::new(0.0, 0.5))
            .animation_clip_input(run, Vec2::new(0.0, 1.0));
    });

    cmd.insert(AnimationHandleComponent::new(assets.add(graph)), entity);
    cmd.insert(AnimationsReady, entity);
}

fn drive(players: Query<&mut AnimationPlayer>) {
    for mut player in players.iter() {
        player.set_vec2_param("movement", Vec2::new(0.0, 0.8));
    }
}
```

A graph is a tree of nodes under a result node:

- **Clip** plays one `AnimationClip`, looping or once
  (`AnimationClipNode::new(clip).with_play_mode(PlayOnce)`).
- **Blend** mixes its inputs.
- **2D blend space** places clips at points and blends by a `Vec2`
  parameter: the classic locomotion setup.
- **State machine** switches between sub-graphs on triggers such as
  `AnimationFSMTrigger::on_bool("is_grounded", false)` or
  `AnimationFSMTrigger::OnAnimationEnd`. Each transition takes a
  `blend_time`, and the machine's `TransitionBlender` (set with
  `.blender(..)` on the builder) decides what it means: `CrossFade`, the
  default, fades the old state out over `blend_time` seconds, and
  `Inertialization` plays only the new state and decays the pose difference
  to the old one with `blend_time` as the halflife.

Parameters are set by name on the `AnimationPlayer` with `set_bool_param`,
`set_int_param`, `set_float_param` and `set_vec2_param`. They live on a
blackboard that stores each value as a boxed `Reflect`, so `set_param(key, value)`
takes any `Reflect` type and `get_param::<T>(key)` returns `None` when the key is
missing or holds another type; the typed methods are wrappers over those two.
The tech demo's
`examples/tech-demo/src/character.rs` builds a full locomotion graph with a
jump state machine.

### Your own nodes

The node set is open: any crate can define a node. Derive `Reflect` and
`Default`, mark it `#[reflect(AnimationNode)]`, implement `AnimationNode`, and
register the type with `app.register_type::<MyNode>()`.

```rust
#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
struct Gain {
    amount: f32,
}

impl AnimationNode for Gain {
    type Instance = ();
    // create_instance, update, evaluate, reset; is_finished defaults to true
}
```

The node's fields are its data and are stored with the graph. `Instance` is the
per-player runtime state (a playhead, say): `create_instance` makes one for each
player that uses the graph, and `update`, `evaluate`, `reset` and `is_finished`
receive it. Add a node with `with_input(Gain { amount: 2.0 }, |node| { .. })`
on the result node or on another node's context, or start a graph with
`AnimationGraph::from_node`.

A graph stores each node as its registry `type_name` plus its reflected value,
which is what a `.gasset` file holds. The live nodes are built from that data
by `make_live`, which the asset server runs in `on_load`, so a graph loaded
from disk and a graph built in code both go live when they reach the asset
server: `assets.add(graph)` runs it before the handle resolves. A player skips
a graph that is not live. A node whose type is not registered, or whose stored
value does not apply, becomes a `PlaceholderNode` that passes its first input
through, and a warning is logged and kept in `AnimationGraph::load_warnings()`.
The stored data is left as it was, so saving the graph does not lose the node.

Handles inside a node (the clip of an `AnimationClipNode`, the state graphs of
an `AnimationStateMachine`) are stored as asset ids and loaded again by
`make_live`. Keep the clip and state graph handles you give to a builder alive
until `assets.add(graph)` has run.

## Gameplay helpers

`concerto::gameplay` has pieces most character games need:

- `CameraPivot` and `EntityFollow { target, offset }` make a third-person
  camera rig: put a `VirtualCamera` as a child of a pivot that follows the
  player.
- `gameplay::player::spawn_first_person_player(&mut cmd, position, extra)`
  spawns a player with a camera, and the `gameplay::movement::first_person_player_fly`
  system flies it with the keyboard and mouse, for looking around a scene.

```rust
cmd.spawn((
    CameraPivot::default(),
    Transform::from_translation(spawn_point),
    EntityFollow { target: character, offset: Vec3::Y },
))
.add_child((
    VirtualCamera::new(0),
    Transform::from_translation_rotation(Vec3::NEG_Z * 10.0, Quat::from_rotation_y(PI)),
));
```

Next: [User interface](08-ui.md).

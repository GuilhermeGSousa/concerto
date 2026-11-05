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
use concerto::physics::body::BodyId;
use concerto::physics::physics_state::PhysicsState;
use concerto::physics::query::QueryFilter;

fn shoot(physics: Res<PhysicsState>, guns: Query<(&GlobalTransform, &BodyId), With<Gun>>) {
    for (gun, body) in guns.iter() {
        let forward = gun.matrix().transform_vector3(Vec3::NEG_Z);
        let filter = QueryFilter { ignore: &[*body] };
        if let Some(hit) = physics.cast_ray(gun.translation(), forward * 100.0, filter) {
            println!("hit {:?} at {}", hit.entity, hit.point);
        }
    }
}
```

`cast_ray(origin, direction_and_length, filter)` returns the first hit, with
its fraction, point and normal and, when it hit a body with an entity, that
`entity`. A ray that starts inside a body hits it at fraction 0. The
`QueryFilter` lists bodies the ray passes through, such as the shooter's own;
`QueryFilter::default()` ignores nothing.

`cast_sphere(origin, radius, direction_and_length, filter)` sweeps a sphere
instead of a ray and returns a `ShapeHit` for the first body it touches: the
fraction of the sweep at which it touches, the contact point on the body, and
the normal of the hit surface, pointing away from it. A sphere that starts
overlapping a body hits it at fraction 0, whichever way it moves, and triangle
meshes are hit from both sides. A radius below `0.001` is raised to it.

A simulated entity gets a `BodyId`, which
`physics.add_impulse(body_id, impulse)` takes.

### Characters

`CharacterSimulation` springs a dynamic body toward the `DesiredMotion` that
other systems write, and `GroundProbe` reports whether it stands on something:

```rust
use concerto::motion_matching::{CharacterSimulation, DesiredMotion};
use concerto::physics::ground::GroundProbe;

let collider = Collider::capsule(0.5, 0.3);
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
(halflife 0.27 s by default) toward `DesiredMotion` and sets the body's
horizontal velocity to the spring's. After the physics step it predicts
`sample_count` samples of the path ahead, `sample_dt` apart, assuming the
desired motion holds, for motion matching to search with. The springs and the
prediction are internal; to read how a character moves, ask its body with
`physics.linear_velocity(body_id)`.

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
  (`AnimationClipNode::new(clip).with_play_mode(PlayOnce)`), at the rate set
  with `with_play_rate` times the float parameter named with
  `with_play_rate_param`. Root motion scales with the rate.
- **Blend** mixes its inputs, each in proportion to its node weight, set
  with `AnimationPlayer::set_node_weight(node, weight)`. Every weight starts
  at 1.0, so inputs mix equally; an input at weight 0 is left out, and with
  none left the node gives the rest pose.
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
A node can read its playback rate from a float parameter: the rate is the
node's own rate times the parameter, and a parameter that is missing or holds
another type leaves the node's rate alone. A product that is negative or not
finite holds the node at rate 0.
`crates/animation/tests/graph_serialization.rs` builds a graph with a blend
space and a state machine. The tech demo's character plays no authored graph:
`examples/tech-demo/src/character.rs` gives it one `MotionMatchingNode`, which
plays a motion database, and the
[motion matching README](../../crates/motion-matching/README.md#playing-a-database)
covers that node.

### Your own nodes

The node set is open: any crate can define a node. Derive `Reflect` and
`Default`, mark it `#[reflect(AnimationNode)]`, implement `AnimationNode`, and
register it in your plugin with `app.register_animation_node::<MyNode>()` (from
`AnimationAppExt`), which registers the type and its inputs.

```rust
#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
struct Gain {
    amount: f32,
}

impl AnimationNode for Gain {
    type State = ();
    type CreateInput = ();
    type UpdateInput = Res<'static, Tuning>;
    type EvaluateInput = ();
    // create_state, update, evaluate, reset; is_finished defaults to true
}
```

The node's fields are its data and are stored with the graph. `State` is the
per-player runtime state (a playhead, say): `create_state` makes one for each
player that uses the graph, and `update`, `evaluate`, `reset` and `is_finished`
receive it.

The three input types say what the node reads in each walk of the graph:
`create_state` gets `CreateInput`, `update` gets `UpdateInput` and
`evaluate` gets `EvaluateInput`, already fetched, as their typed data (`&Res<Tuning>`
above). Each is a read-only system input: `Res`, `Option<Res>`, a read-only
`Query`, or a tuple of them; `()` reads nothing. The systems that walk graphs,
`initialize_animation_players`, `update_animation_players` and
`animate_targets`, each declare the union of their walk's inputs over every
registered node, and nothing else, so `animate_targets` never declares what only
an update reads. A node whose input is missing, or whose type was registered
without `register_animation_node`, is skipped in that walk with a warning. The
clip node reads `AssetStore<AnimationClip>` and the state machine
`AssetStore<AnimationGraph>` this way. `AnimationGraphContext` carries only the
player's own data: its blackboard, its root bone in `evaluate`, and its
skeleton.

Add a node with `with_input(Gain { amount: 2.0 }, |node| { .. })`
on the result node or on another node's context, or start a graph with
`AnimationGraph::from_node`.

A graph holds each node as an `AnimationNodeData`, written to a `.gasset` file
as its registry `type_name` plus its reflected value. A node added in code or
read from disk is `Stored`; `make_live` builds it `Live`, and the asset server
runs `make_live` in `on_load`, so a graph loaded from disk and a graph built in
code both go live when they reach the asset server: `assets.add(graph)` runs it
before the handle resolves. A player skips a graph that is not live. A node
whose type is not registered, or whose stored value does not apply, becomes a
`Placeholder` that plays as a `PlaceholderNode`, passing its first input
through, and a warning is logged and kept in `AnimationGraph::load_warnings()`.
The placeholder keeps the stored value, so saving the graph does not lose the
node, and a later `make_live` with the type registered builds it.

Handles inside a node (the clip of an `AnimationClipNode`, the state graphs of
an `AnimationStateMachine`) are stored as asset ids and loaded again by
`make_live`. Keep the clip and state graph handles you give to a builder alive
until `assets.add(graph)` has run.

### Root motion

A clip whose root bone moves gives root motion: a `RootMotion` that says how the
root moved this frame, with a `translation` and a `rotation` that read as
"forward, as the character faced last frame". Every source in a graph produces
one, and blends and transitions carry it with the pose, so it works for any
graph. Each frame the result lands on the root bone, in
`AnimationRootBone::root_motion`. A paused player, an unloaded skeleton or graph,
or a clip with no channels for the root gives `RootMotion::IDENTITY`.

The root bone's `Transform` is written like any other bone's, net of what root
motion took: its yaw, measured from the rest pose, and its ground translation.
Its height, pitch, roll and scale stay in the bone. That split assumes the root's
rest pose stands upright, and on a skeleton whose rest doesn't, such as a BVH's,
lean can read as yaw. Yaw and ground translation are always removed, with or
without a `CharacterDisplacement`, so a clip that turns or walks stays on the
spot in a game that applies nothing. A joint a clip has no channels for shows the
skeleton's rest pose.

Nothing applies it on its own. To have it move a character, make the visible
character a separate entity from the capsule, with the skeleton under it, and
give it a `CharacterDisplacement` and a `CharacterLink` naming the capsule and
the skeleton. The link's fields are `SceneEntityRef`s: pick both in the
editor's inspector and the link is saved with the scene, or build it in code:

```rust
use concerto::ecs::component::scene::SceneEntityRef;
use concerto::motion_matching::{CharacterDisplacement, CharacterLink, Clamp};

cmd.spawn((
    CharacterDisplacement::Clamp(Clamp::default()),
    CharacterLink {
        capsule: SceneEntityRef::Entity(capsule),
        skeleton: SceneEntityRef::Entity(skeleton),
    },
    Transform::default(),
));
```

Each frame the character moves by its root motion, and is then reconciled with
the capsule by one of three methods, which are mutually exclusive:

- `Synchronization` places both at a weighted average each frame, set by `data_factor`: 0
  is code-driven, 1 is data-driven. The capsule is pulled through its velocity,
  so physics still stops it at a wall.
- `Adjustment`, the default, damps the character towards the capsule, by no more
  than a ratio of this frame's root motion, so a standing character is not
  dragged.
- `Clamp` keeps the character within a distance and an angle of the capsule.

Height is always the capsule's. Do not parent the character to its capsule: it
would move twice. The [crate's README](../../crates/motion-matching/README.md)
has the fields, what the skeleton has to be, and what each method promises at a
wall.

## Gameplay helpers

`concerto::gameplay` has pieces most character games need:

- `CameraPivot` and `EntityFollow { target, offset }` make a third-person
  camera rig: put a `VirtualCamera` as a child of a pivot that follows the
  player.
- `CameraBoom { offset, probe_radius, min_distance, return_halflife }` on that
  camera child keeps it out of walls. Each frame it sweeps a sphere of
  `probe_radius` from the pivot toward `offset`, ignoring the body of the
  pivot's `EntityFollow` target, and pulls the camera in to the first hit at
  once, never closer than `min_distance`. Once the way is clear it eases back
  out, closing half the gap every `return_halflife` seconds. The pivot must be
  a root entity.
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
    CameraBoom { offset: Vec3::NEG_Z * 10.0, ..Default::default() },
));
```

Next: [User interface](08-ui.md).

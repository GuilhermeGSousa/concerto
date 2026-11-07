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
use concerto::name::name;

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
    graph.result_node().with_blend_space_2d_input(name!("movement"), |space| {
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
        player.set_vec2_param(name!("movement"), Vec2::new(0.0, 0.8));
    }
}
```

A graph is a tree of nodes under a result node:

- **Clip** plays one `AnimationClip`, looping or once
  (`AnimationClipNode::new(clip).with_play_mode(PlayOnce)`), at the rate set
  with `with_play_rate` times the float parameter whose `Name` is given to
  `with_play_rate_param`. Root motion scales with the rate.
- **Blend** mixes its inputs, each in proportion to its node weight, set
  with `AnimationPlayer::set_node_weight(node, weight)`. Every weight starts
  at 1.0, so inputs mix equally; an input at weight 0 is left out, and with
  none left the node gives the rest pose.
- **Additive** (`AdditiveNode`) plays an additive clip's delta (see
  [Additive clips](#additive-clips)) over its one input, the base; with no
  input it goes over the rest pose, and inputs after the first are ignored
  with a warning. The node owns its clip and a clock over the delta, in delta
  time: seconds from the start of the clip's `range`, so `start_time` is where
  the delta starts and where `Loop` returns to, and `PlayOnce` holds at its
  end. The rate is `play_rate` times the float under `play_rate_param`, as on
  **Clip**. The weight is `weight` times the float under `weight_param`, with
  NaN as 0, clamped to [0, 1]; a missing param leaves `weight` alone, and the
  input's node weight is ignored. Nothing eases it, so smooth the param in the
  game. The clock runs at weight 0 too, so an overlay fading in doesn't start
  over, and at weight 0 the output is the base exactly. On the player's root
  bone the delta adds no twist about up and no horizontal move, so a lean
  stays but the heading and the ground position are the base's. Root motion
  is always the base's. The base's events keep their weights, and the clip's
  are added at the node's weight. A plain clip passes the base through with
  one warning; an additive clip whose delta isn't built, such as one added in
  code, passes it through with none. `AnimationPlayer::play(node)` restarts
  the delta, and the node never holds back `OnAnimationEnd`: it always counts
  as finished. The node has no mask: it layers the delta on every bone the
  clip animates. `AdditiveNode::new(clip)` starts at weight 1, looping from 0
  at rate 1, and every other field has a `with_<field>`:
  ```rust
  graph.result_node().with_input(
      AdditiveNode::new(cold).with_weight_param(name!("cold")),
      |mut base| {
          base.with_input(AnimationClipNode::new(walk), |_| {});
      },
  );
  ```
- **Layer** (`LayerNode`) lays its second input, the overlay, over its first,
  the base, on the bones a `BoneMask` asset names. Build it with
  `with_layer_input(node, base, overlay)`, which adds the base through the
  first closure and then the overlay through the second, so the order can't
  come out wrong; it panics unless each closure adds exactly one input. Layers
  stack by chaining: one layer can be the base of the next. With no overlay
  the node passes the base through, with no input at all it gives the rest
  pose, and inputs after the second are ignored with a warning. A `BoneMask` is made from the content browser's New menu,
  or in code. Its `branches` are `BoneMaskBranch`es, each a `bone` by name, a
  `weight`, a `depth` and `descendants`. A branch covers its bone and, with
  `descendants` on (the default), the bones under it. Each bone takes the
  nearest branch at or above it that covers it, so a branch on a child
  overrides its ancestor's, and a branch at weight 0 cuts a sub-branch out.
  With `descendants` off, a branch changes its own bone alone, and the bones
  under it keep what covered its parent. `depth` ramps the weight in over that
  many levels below the bone: `depth: 2` gives ⅓, ⅔ and then the full
  weight. A bone no branch covers keeps the base. A mask names bones, so one
  mask serves every skeleton with those names; a name the skeleton lacks is
  skipped with a warning. `preview_skeleton` is read only by the editor,
  whose bone picker lists that skeleton's bones; a game never loads it.
  `space` sets where rotations blend. `LayerSpace::Mesh`, the default, blends
  each masked bone's model-space rotation, so a masked arm keeps the
  overlay's orientation whatever the base's spine does: use it for aiming and
  for a held prop. `LayerSpace::Local` blends each joint's local rotation, as
  **Blend** does, so the masked bones ride on the base's body: use it when
  the overlay was made over the same body motion as the base, such as a wave
  over an idle. Translation and scale blend locally in both. The layer weight
  is `weight` times the float under `weight_param`, with NaN as 0, clamped to
  [0, 1]; a missing param leaves `weight` alone. The inputs' node weights are
  ignored, and nothing eases the weight, so smooth the param in the game.
  Root motion is always the base's. When the player has a root bone (one
  marked with `AnimationRootBone`), its joint is the base's too, even under a
  mask that names it, which warns; without one, a mask may cover bone 0 like
  any other. The base's events keep their
  weights, and the overlay's are added at their weight times the layer
  weight, whatever the mask.
  ```rust
  let upper_body = assets.add(
      BoneMask::new()
          .with_branch(BoneMaskBranch::new("Spine1").with_depth(2))
          .with_branch(BoneMaskBranch::new("Neck").with_weight(0.0)),
  );
  graph.result_node().with_layer_input(
      LayerNode::new(upper_body).with_weight_param(name!("aim_weight")),
      |mut base| {
          base.with_input(AnimationClipNode::new(run), |_| {});
      },
      |mut overlay| {
          overlay.with_input(AnimationClipNode::new(aim), |_| {});
      },
  );
  ```
- **2D blend space** places clips at points and blends by a `Vec2`
  parameter: the classic locomotion setup.
- **State machine** switches between sub-graphs on triggers such as
  `AnimationFSMTrigger::on_bool(name!("is_grounded"), false)` or
  `AnimationFSMTrigger::OnAnimationEnd`. Each transition takes a
  `blend_time`, and the machine's `TransitionBlender` (set with
  `.blender(..)` on the builder) decides what it means: `CrossFade`, the
  default, fades the old state out over `blend_time` seconds, and
  `Inertialization` plays only the new state and decays the pose difference
  to the old one with `blend_time` as the halflife.
- **Two-bone IK** (`TwoBoneIkNode`) bends a limb so its tip reaches a goal.
  The `tip` bone names the chain: its parent is the elbow or knee and its
  grandparent the shoulder or hip, all under bone 0. The `target` is an
  `IkGoal`: `Bone` fixes it to a bone of the input pose, with a translation
  and rotation in that bone's space, and `Param` reads an `IkTarget` the game
  set on the blackboard, in world space. An optional `pole`, another
  `IkGoal`, picks the side the elbow or knee bends toward. `softness` eases
  the last stretch before full extension, `tip_rotation` keeps the input's
  hand or foot rotation (`KeepInput`) or takes the goal's (`FromTarget`), and
  `weight`, times the float under `weight_param`, blends the solve over the
  input. Wire it after the nodes whose pose it corrects. A target param the
  game has not set yet leaves the pose as it is. The same solve is
  `concerto_animation::ik::solve_two_bone` for code outside a graph: it takes
  the three joint positions in any one space and returns rotations in that
  space.
- **Look-at** (`LookAtNode`, in `concerto::procedural_animation`, registered
  by `ProceduralAnimationPlugin`) turns a character's spine, neck, head and eyes
  toward a point while the body plays its input. The `chain` lists
  `LookAtBone`s from the spine to the head, each under the one before, all
  under bone 0. Each bone turns by its `share` of the angle left to it, and
  within its `yaw_limit` and `pitch_limit` in radians, so a bone held at a
  limit leaves the rest to the bones after it. The `eyes`, `LookAtEye`s under
  the last chain bone, then each aim on their own from their own position,
  within their limits. `forward` and `up` are model-space axes, +Z and +Y by
  default, read in each bone's rest pose. The target is a world-space `Vec3`
  the game set under `target_param`. `weight`, times the float under
  `weight_param`, blends the look in; a target that is missing, or more than
  `max_angle` (at most 170°) off the first bone's animated gaze, eases it out
  over `weight_halflife`. Each bone eases toward its turn with its own
  `halflife`, so fast eyes reach the target while a slower head is still
  turning. The look starts from the input's pose when the node starts or
  resets. Wire it after the nodes whose pose it turns, and before two-bone
  IK. The yaw and pitch maths are `concerto_animation::look_at` for code
  outside a graph.
- **Foot placement** (`FootPlacementNode`, in
  `concerto::procedural_animation`, registered by `ProceduralAnimationPlugin`)
  puts each foot on the ground under it. Each `FootPlacementLeg` names a foot
  bone, whose parent is the knee and grandparent the hip, under the `pelvis`
  bone, and an optional toe bone. The node casts physics rays straight down at
  the ankle and the toe, from `max_step_up` above the floor, the model
  origin's world height, to `max_step_down` below it; a missing
  `PhysicsState` counts as every ray missing. It lifts or lowers each foot by
  the ground under it, keeping the clip's lift above its floor, pitches it to
  the line from ankle to toe hit and rolls it to the ankle hit's normal,
  within `max_foot_angle`, and lowers the pelvis by the lowest foot, at most
  `max_pelvis_drop`. Each leg is then bent to its foot with the two-bone
  solve, with `softness` as in two-bone IK; the toe keeps its local rotation.
  The pelvis, the foot heights and the foot tilts ease toward their targets
  with `pelvis_halflife`, `foot_halflife` and `rotation_halflife`, and snap
  on the first evaluate. `weight`, times the float under `weight_param`,
  eases in and out with `weight_halflife`; a game sets that param to 0 while
  the character is in the air. Under `ignore_param` the game sets an
  `IgnoredBody` naming the character's own capsule, which the rays then skip.
  Wire it after the layers, aim and look-at nodes and before hand IK.
  `FootPlacementNode::new(pelvis)` starts from the defaults, `with_leg` adds a
  leg, and each other field has a `with_<field>`:
  ```rust
  FootPlacementNode::new("Hips")
      .with_leg(FootPlacementLeg::new("LeftFoot").with_toe("LeftToe"))
      .with_leg(FootPlacementLeg::new("RightFoot").with_toe("RightToe"))
      .with_max_step_up(0.4)
      .with_weight_param(name!("foot_placement_weight"))
      .with_ignore_param(name!("foot_placement_ignore"))
  ```

Parameters are set on the `AnimationPlayer` with `set_bool_param`,
`set_int_param`, `set_float_param` and `set_vec2_param`, each keyed by a `Name`
(`concerto::name`): an interned string, written `name!("movement")` for a
literal or `Name::new(text)` otherwise, that compares as one integer. They live
on a blackboard that stores each value as a boxed `Reflect`, so `set_param(key, value)`
takes any `Reflect` type and `get_param::<T>(key)` returns `None` when the key is
missing or holds another type; the typed methods are wrappers over those two.
A node can read its playback rate from a float parameter: the rate is the
node's own rate times the parameter, and no parameter (`None`, the default), or
one that is missing or holds another type, leaves the node's rate alone. A product that is negative or not
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
player's own data: its blackboard, its root bone in `evaluate`, its skeleton,
and, in `evaluate`, `model_to_world`: the `GlobalTransform` of bone 0's parent,
or identity when it has none.

`evaluate` writes an `AnimationFrame`: a `Pose`, the joints in bone order, with
the root motion and the clip events its sources gave this frame. Its inputs
arrive as `EvaluatedFrame`s, each a frame and the weight its node carries.
The frame carries the clip events its sources fired this frame
(`AnimationFrame::events`), and `copy_from` and `blend` carry them along,
weighted as the joints are. A node that builds its output without `copy_from`
or `blend` must forward its inputs' events itself, with
`AnimationFrame::push_event`, or they never reach the player.

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
one, and blends and transitions carry it with the frame, so it works for any
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

### Clip events

A clip can mark times where something happens: a footstep, the fire frame of a
gun. When playback crosses one, the player's entity receives an
`AnimationSignal<S>` (`concerto::animation::event`), where `S` is the event's
type. The importers write `AnimationEvent { name }`, the generic event, from a
clip's import settings ([asset importing](../asset-importing.md)), and code can
add any registered event with `AnimationClip::add_event(time, event)`.

A game defines its own event type by deriving `Reflect`, `Clone` and `Default`,
marking it `#[reflect(ClipEvent)]`, implementing `ClipEvent`, and registering
it with `app.register_clip_event::<Footstep>()` (from `AnimationAppExt`).
`AnimationPlugin` registers `AnimationEvent`. The derive names the type data by
its bare identifier, so import `ReflectClipEvent` wherever you write
`#[reflect(ClipEvent)]`:

```rust
use concerto::animation::event::{ClipEvent, ReflectClipEvent};

#[derive(Clone, Default, Reflect)]
#[reflect(ClipEvent)]
struct Footstep {
    left: bool,
}

impl ClipEvent for Footstep {}
```

A clip that loads an event whose type is not registered logs a warning naming
the clip and the type, and drops it.

`AnimationSignal<S>` holds the event as `signal`, the clip time it sits at as
`time`, the `weight` of its source in the final frame, and `mirrored`, set when
a motion matching entry plays the clip mirrored. It is an entity signal,
triggered on the player's entity, the one with the `AnimationPlayer` and
`SkeletonComponent`, so only a `Listener` on that entity hears it; a global
listener from `app.add_listener` never does. Put one there when you spawn the
character, or from a listener on `Add<AnimationPlayer>`:

```rust
use concerto::animation::event::{AnimationEvent, AnimationSignal};
use concerto::ecs::signal::{lifecycle::Add, listener::IntoListener};

fn hear_events(on: On<AnimationSignal<AnimationEvent>>, mut steps: EventWriter<Step>) {
    let fired = on.signal();
    if fired.signal.name == name!("footstep_left") {
        steps.write(Step { loudness: fired.weight });
    }
}

fn listen_for_events(on: On<Add<AnimationPlayer>>, mut cmd: CommandQueue) {
    cmd.insert(hear_events.into_listener(), on.signal().entity);
}

app.add_listener(listen_for_events.at_preview_time());
```

An entity carries one listener per signal type, and every imported event is an
`AnimationSignal<AnimationEvent>`, so all of a player's named events go through
one closure, which dispatches on `name`. Two features that both want them,
audio and effects say, share that closure, or the game gives each its own event
type. The signals run when `animate_targets`' commands are applied, after the
bones' local `Transform`s are posed for the frame; their `GlobalTransform`s may
still be last frame's. Events fire in editor previews that run an
`AnimationPlayer`, such as scene previews, but a preview keeps only listeners
marked `.at_preview_time()`, as above. The clip editor's preview samples the
clip directly and fires nothing.

Every playing source fires, with its effective weight: a blend gives each input
its share, a blend space each corner its barycentric weight, a cross-fade
the fading states theirs, and an **Additive** node its clip's the node's
weight. A state entered with `CrossFade` fires its start
event on the first frame of the fade, at that frame's weight, about
`dt / blend_time`: 0.07 for a 0.25 s blend at 60 Hz. With `Inertialization` the
new state plays alone, so its start event fires at weight 1.
`AnimationPlayer::set_event_threshold(threshold)` drops every event whose
weight is at or below the threshold; the default, 0, drops only sources a blend
left out.

A clip fires the events after its previous time up to and including its new
one, and the first update after it starts, plays again or is reset also fires
its start time. A loop that wraps fires up to the clip's end and then its start
time; a `PlayOnce` clip that ends fires up to its end. A paused clip, one that
has not loaded, and one at rate 0 fire nothing, and a clip that has not yet
moved still fires its start time once it does. Nothing else fires a span it
skipped: a state machine entering a state restarts it, so a clip state fires
its start time, but no other jump fires what lies between. A motion matching
state fires nothing on its first frame: entering it resets the state, and it
fires only once it has started playing an entry.

### Additive clips

An additive clip is layered over another pose instead of replacing it: an
overlay such as cold, wounded or fear is recorded as a full take next to a
matched neutral take, its base, and what is layered is the difference between
the two. `AnimationClip::additive()` says whether a clip is additive, as an
`Additive` (`concerto::animation::clip`):

- `Additive::None`, the default: a plain clip.
- `Additive::LocalSpace(AdditiveBase)`: additive in each bone's local space
  against `AdditiveBase::clip`, the base clip, kept by asset id.

The importers set it from a source's import settings
([asset importing](../asset-importing.md)), and code can with
`set_additive`, building the base with `AdditiveBase::new(clip)` and
`with_range`, `with_base_range` and `with_root`. `set_additive` drops any delta
the clip already holds, so a clip made plain, or given a new base, samples no
delta until it is built again.

The clip keeps its full-pose keys, so `AnimationClipNode` and the clip editor
play it as any other clip. The difference, the delta, is derived data: it is
built when the clip loads, kept in the derived data cache, and written into the
clip by a cook, so a cooked build doesn't read the base. A clip whose
base is missing, unset, itself additive, or whose ranges don't fit fails to
load, with an error naming the clip and the reason. A clip added in code with
`AssetServer::add` derives nothing, and has no delta.

For each frame, the delta of a bone is the clip's rotation times the inverse of
the base's, and the clip's translation minus the base's. Applied with
`apply_additive(&mut pose, &delta, weight)` (`concerto::animation::pose`), it
turns the pose's rotation by the delta, slerped from identity by `weight`, and
moves its translation by the weighted delta. At weight 1 over the base it gives
the clip back. `AnimationClip::sample_additive(time, &bone)` gives the delta at
a time, clamped to the delta, and `additive_duration()` its length.

The base is aligned by normalized time: `range` and `base_range`, in seconds,
trim each clip, the whole clip when unset, and the base's span is stretched to
the clip's, so a base twice as long is read at twice the clip's time. Keys are
taken at the clip's frame rate, its detected rate when it has none stored, or
30 frames a second, and land on both ends of `range`.

A delta exists only for a bone and component that both clips animate; any other
reads as identity. The bones in `roots`, the roots of the skeleton the source
emits, keep their rotation delta but no translation, so an overlay adds a lean
of the whole body but no root motion. Scale is never layered, and a clip's
events stay on the clip, fired by the **Additive** node at its weight.

The graph node that plays an additive clip over a pose is **Additive**, in the
node list under [Animation](#animation). Mesh-space deltas and a single-frame or
rest-pose base are not there yet.

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

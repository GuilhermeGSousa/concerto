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
| A box collider authored in a scene | `BoxCollider { half_extents }`, half extents, `0.5` on each axis by default |

`RigidBody` has a `density`, a `motion_type` (`Dynamic` by default) and
`allowed_dofs`, which can lock rotation: a character uses
`AllowedDofs::TRANSLATION | AllowedDofs::ROTATION_Y` so it never tips over. A
`ColliderOffset` moves the shape relative to the entity;
`ColliderOffset::bottom_origin(&collider)` puts the entity's origin at the
bottom of a capsule, where a character's feet are.

The body is built from the `Collider` once. Inserting a new `Collider` on the
entity rebuilds its body from the new shape, which is how editing a
`BoxCollider` in the inspector or over MCP takes effect. A `BoxCollider` on an
entity that also has a `MeshCollider` wins over the mesh.

### Sensors

A `Sensor` next to a `Collider` makes it a trigger volume. A sensor detects the
bodies that enter and leave it but never collides with them. It tells its own
entity through two signals, `SensorEnter` and `SensorExit`, each carrying the
`other` entity: the one holding the other body's `Collider`. React to them with
listeners on the sensor:

```rust
use concerto::ecs::signal::{listener::{IntoListener, Listener}, On};
use concerto::physics::sensor::{Sensor, SensorEnter};

fn enter_room(room: RoomId) -> Listener<SensorEnter> {
    (move |on: On<SensorEnter>, players: Query<&Player>, mut warmth: ResMut<Warmth>| {
        if players.get_entity(on.signal().other).is_some() {
            warmth.room = room;
        }
    })
    .into_listener()
}

cmd.spawn((
    Transform::from_translation(room_centre),
    Collider::cuboid(4.0, 1.5, 3.0),
    Sensor,
    enter_room(RoomId::Lodge),
));
```

An entity carries one listener per signal type, so two features that react to
one sensor share its `Listener<SensorEnter>`. A sensor placed in a scene, as a
`BoxCollider` with a `Sensor`, carries no listener: the game adds one, for
example from an `On<Add<Sensor>>` listener.

- **What it detects:** dynamic and kinematic bodies, awake or asleep. It does
  not detect static colliders or other sensors. A body that falls asleep inside
  a sensor stays inside: it gets no `SensorExit`.
- **Static or kinematic:** a `Sensor` alone is static and stays where it spawned.
  With `RigidBody { motion_type: MotionType::Kinematic, .. }` it moves by
  `set_linear_velocity`, like any kinematic body. On a dynamic `RigidBody` it is
  made kinematic, with a warning.
- **When a body goes away:** when a body inside a sensor is despawned or loses its
  `Collider`, the sensor gets a `SensorExit` naming it. By then the entity may
  no longer exist. A despawned sensor hears nothing, since its listeners go
  with it. Rebuilding either body of a pair, by inserting a new `Collider` or
  by editing a sensor's `BoxCollider` while playing, sends `SensorExit` then
  `SensorEnter`.
- **Shapes:** a sensor is a sphere, box or capsule. A triangle mesh has no
  inside, so a `Sensor` on a `MeshCollider` without a `BoxCollider` is ignored,
  with a warning; a `BoxCollider` beside it wins, and the box is the sensor.
- **Queries:** `cast_ray`, `cast_sphere` and the ground probe never hit a
  sensor.

Signals are sent once per body pair, from `LateFixedUpdate`, right after the
physics step.

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

fn walk(
    input: Res<Input>,
    actions: Res<ActionMap>,
    players: Query<(&mut DesiredMotion, &GroundProbe), With<Player>>,
) {
    let push = actions.axis2d(Move, &input);
    for (mut desired, ground) in players.iter() {
        if ground.is_grounded() {
            desired.velocity = Vec3::new(push.x, 0.0, -push.y) * 5.0;
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
`is_held`, `is_just_pressed` and `is_just_released` for keys, the matching
`is_mouse_button_*` methods for the mouse, and `gamepads()`, `gamepad(id)` and
`any_gamepad()` for connected pads, whose sticks read with the dead zone
applied; `GamepadConnection` events say when a pad comes and goes. Gameplay
code reads actions rather than keys: the `ActionMap` resource binds an action
to several keys, mouse buttons and pad buttons, and a 2D action to key sets and
sticks. `is_held`, `is_just_pressed` and `is_just_released` read a button
action and `axis2d` a 2D one, each against `Input`. `GameplayPlugin` binds
`Move` (WASD, the arrows and the left stick) and `Look` (the right stick) in
`concerto::gameplay::actions`; the `walk` system above reads `Move`, so a
half-pushed stick walks at half speed.

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
  code, passes it through with none. `AnimationPlayer::play(node, graphs)`
  restarts the delta, and the node never holds back `OnAnimationEnd`: it always counts
  as finished. The node has no mask: it layers the delta on every bone the
  delta covers. `AdditiveNode::new(clip)` starts at weight 1, looping from 0
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
- **Aim offset** (`AimOffsetNode`) lays captured aim poses over its first
  input, chosen by where a target is. Each `AimPoint` is an additive clip and
  the yaw and pitch, in radians, it aims at. The target is a world-space
  `Vec3` the game set under `target_param`; the node measures its yaw and
  pitch from `frame_bone`, under bone 0, along `forward` with `up`, model-space
  axes read in that bone's rest pose, +Z and +Y by default. A yaw toward the
  side `up × forward` is positive, and a pitch toward `up`. The angles ease
  toward the target over `angle_halflife` (0 snaps), and a target crossing
  behind the character turns the short way. The first evaluate after the node
  starts, resets, or has eased its weight out to 0 takes the target's angles
  at once. The points are weighed
  by those angles: one point takes everything, points on one line, such as a
  pitch-only set of aim up, level and down, interpolate along it and clamp at
  its ends, and any other set is triangulated as a 2D blend space is. A point
  closer than 1e-4 rad to an earlier one is left out. The chosen deltas
  combine bone by bone and are applied with `apply_additive` at the node's
  weight, root bone included, so a captured lean of the whole body comes
  through; root motion and events pass through. Every clip plays on one
  clock, each looping over its own delta, so a held pose and a breathing aim
  loop both work. `weight`, times the float under `weight_param`, eases in
  and out over `weight_halflife`; a missing target eases it out, and at
  weight 0 the input passes through untouched. A clip not loaded yet counts
  as no change, as does a plain clip, which warns once. Wire it after the
  base and the layers, and before `AimNode` and look-at.
  `AimOffsetNode::new(target_param, frame_bone)` starts from the defaults,
  `with_point` adds a point, and each other field has a `with_<field>`:
  ```rust
  AimOffsetNode::new(name!("aim_target"), "Spine2")
      .with_point(AimPoint::new(aim_down, 0.0, -0.8))
      .with_point(AimPoint::new(aim_level, 0.0, 0.0))
      .with_point(AimPoint::new(aim_up, 0.0, 0.8))
      .with_weight_param(name!("aim_weight"))
  ```
- **Aim** (`AimNode`, in `concerto::procedural_animation`, registered by
  `ProceduralAnimationPlugin`) closes what an aim offset misses: it turns a
  chain of bones so a weapon's barrel points at a world-space `Vec3` the game
  set under `target_param`. The `chain` lists `AimBone`s from the root up,
  each under the one before, all under bone 0; the root bone may be one. The
  `Muzzle` is a point (`offset`) and a direction (`direction`, +Z by default)
  in its bone's space, and its bone is the last chain bone or under it. Each
  bone turns by its `share` of the angle left to it, within its `yaw_limit`
  and `pitch_limit` in radians, about the model-space `up`. The solve runs
  three passes over the chain, each reading the barrel afresh, so the parallax
  of a muzzle in front of the spine is taken out: a target four or more
  muzzle-distances away is hit within millimetres. A target behind the
  muzzle, or nearer the first chain bone than the muzzle is, holds the last
  turn. Each bone eases from its turn to a new solve over
  `reacquire_halflife`, times the float under `reacquire_param` (0, the
  default, applies the solve at once). `weight`, times the float under
  `weight_param`, eases in and out over `weight_halflife`; a missing target
  eases it out holding the last turn, and at weight 0 the input passes
  through untouched. Root motion and events pass through. Wire a character
  that aims as base, layers, `AimOffsetNode`, `AimNode`, look-at, sway and
  tremor, then two-bone IK for the support hand. Where a shot's additive
  clip goes picks the recoil: before `AimNode` the kick is corrected away at
  a `reacquire_halflife` of 0, or recovers over that halflife above 0; after
  it, through an **Additive** node, the kick plays as captured over a
  barrel that stays on the target. `AimNode::new(target_param, muzzle)`
  starts from the defaults, `with_bone` adds a bone, and each other field has
  a `with_<field>`:
  ```rust
  AimNode::new(
      name!("aim_target"),
      Muzzle::new("RightHand").with_offset(Vec3::new(0.0, 0.05, 0.25)),
  )
  .with_bone(AimBone::new("Spine1").with_share(1.0))
  .with_bone(AimBone::new("Spine2").with_share(2.0).with_limits(0.8, 0.5))
  .with_reacquire_halflife(0.12)
  ```
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
- **Procedural motion** (`ProceduralMotionNode`, in
  `concerto::procedural_animation`, registered by `ProceduralAnimationPlugin`)
  adds small rotations that wave or wander over time to its input's pose: aim
  sway, hand tremor, breathing in the chest. Each of its `channels`, a
  `ProceduralMotionChannel`, moves the bones of its `BoneMask`, each by its
  mask weight, so a `depth` ramp turns the spine by a third, the chest by two
  thirds and the bones above by the full amount. A masked bone turns about the
  skeleton's model axes at its own pivot, from where its input put it, so a
  bone covered at its parent's weight simply follows the parent; +X is the
  character's side, +Y up and +Z forward, the same on every skeleton. The
  `waveform` is `Waveform::Sine`, one sine shared by the three axes, for
  breathing, or `Waveform::Noise { octaves }`, fractal gradient noise of its
  own on each axis, for sway and tremor. `frequency` is in hertz and
  `amplitude` is the peak rotation in radians about each model axis. The
  rotation is scaled by the float under `magnitude_param`, 1 when there is
  none or the param is missing or holds another type, 0 when it is negative
  or not finite; the scale eases toward that value with
  `magnitude_halflife`, and snaps to it when the node starts or resets, so a
  character spawned wounded sways at full strength from its first frame.
  Channels compose in list order, a later one's on the left. Root motion,
  events, translations, scales and every bone outside the masks are the
  input's, and a mask may name the root bone, for hip sway. The presets
  `ProceduralMotionChannel::breathing`, `sway` and `tremor` start from
  numbers that suit those uses. Each channel's phase and noise come from the
  node's `seed` mixed with the seed the game set on the blackboard under
  `seed_param`, a `u64` or an int param, 0 when there is none: players with
  the same seeds move in step, so set each character's from something stable
  across runs, such as its spawn index. Resetting the node keeps its
  phases, so re-entering a state does not replay the same stretch of motion.
  Wire it after aim and look-at and before hand IK, so the off hand stays on
  a swaying gun. `ProceduralMotionNode::new()` has no channels, `with_channel`
  adds one, and each channel field has a `with_<field>`:
  ```rust
  ProceduralMotionNode::new()
      .with_channel(ProceduralMotionChannel::breathing(chest_mask))
      .with_channel(
          ProceduralMotionChannel::sway(arms_mask)
              .with_magnitude_param(name!("wounded"))
              .with_magnitude_halflife(0.5),
      )
      .with_channel(
          ProceduralMotionChannel::tremor(hands_mask)
              .with_magnitude_param(name!("fear")),
      )
      .with_seed(7)
      .with_seed_param(name!("character_seed"))
  ```
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
- **Foot locking** (`FootLockNode`, in `concerto::procedural_animation`,
  registered by `ProceduralAnimationPlugin`) holds each planted foot where it
  was planted, so it doesn't slide when the clip's foot motion and the
  character's movement disagree. Each `FootLockLeg` names a foot bone, whose
  parent is the knee and grandparent the hip, and an optional toe bone, the
  contact point a plant is read from and held at. A contact point is planted
  while its horizontal speed is under `plant_speed` and its height above the
  model origin under `plant_height`; the speed is measured in the
  animation's own frame, the contact point's model-space motion plus the
  input's root motion, turning about the root bone, so a foot under a
  walking or turning root plants and a foot carried by a running root does
  not. A planted foot is locked in world
  space, and only its horizontal position is held: its height stays the
  input's. The lock is released when the plant ends, or when the animated
  foot moves more than `unlock_radius` from it, and the foot then eases back
  to the animated one over `release_halflife`; after the radius releases a
  lock, the foot waits for its next plant. Each leg is bent to its held foot
  with the two-bone solve, with `softness` as in two-bone IK, and the foot
  keeps its input rotation. `weight`, times the float under `weight_param`,
  eases in and out with `weight_halflife`, and a weight of 0 plants nothing.
  Wire it before `FootPlacementNode`, which then adds the ground's height.
  `concerto-procedural-animation-gizmos` draws each lock.
  `FootLockNode::new()` starts from the defaults, `with_leg` adds a leg, and
  each other field has a `with_<field>`:
  ```rust
  FootLockNode::new()
      .with_leg(FootLockLeg::new("LeftFoot").with_toe("LeftToe"))
      .with_leg(FootLockLeg::new("RightFoot").with_toe("RightToe"))
      .with_unlock_radius(0.2)
      .with_weight_param(name!("foot_lock_weight"))
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
receive it. `AnimationPlayer::play(node, graphs)` calls the node's `reset`, so
a node restarts on `play` by resetting its state there.

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
the fading states theirs, and an **Additive** node its clip's events at the
node's weight. A state entered with `CrossFade` fires its start
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
node list under [Animation](#animation), and **Aim offset** plays additive aim
clips chosen by a target's yaw and pitch. Mesh-space deltas and a single-frame
or rest-pose base are not there yet.

### Retarget rigs

A `RetargetRig` (`concerto::animation::retarget`) describes one skeleton in
terms another skeleton's rig can match, so that motion recorded on one can be
played on the other. It holds:

- `skeleton`, the skeleton it describes;
- `root`, the retarget root: the pelvis, `Hips` on LAFAN1 and `pelvis` on UAL1;
- `motion_root`, optional, a bone above the root that carries the character's
  ground motion, such as UAL1's `root`. Without one the retarget root carries
  it, as on LAFAN1;
- `chains`, named runs of bones;
- `reference`, the pose the rig is aligned in, and `corrections` on top of it.

A `RetargetChain` is a `name`, a `start` bone and an `end` bone below it, and
its bones run from `start` down to `end`; a single-bone chain has
`start == end`. Chains are matched across rigs by name alone. Names are free
text, but rigs written apart match only when they agree, so use these:

| Chain | From | To |
|---|---|---|
| `spine` | first spine bone | last spine bone |
| `neck` | first neck bone | last neck bone |
| `head` | head | head |
| `clavicle_l`, `clavicle_r` | clavicle / shoulder | clavicle / shoulder |
| `arm_l`, `arm_r` | upper arm | hand |
| `leg_l`, `leg_r` | thigh | foot or ball |
| `thumb_l` … `pinky_l`, `thumb_r` … `pinky_r` | first finger bone | last finger bone |

Nothing checks a rig's names against the table, and chains may share bones.

The reference pose is `ReferencePose::Rest`, the skeleton's rest pose, or
`ReferencePose::ClipFrame { clip, time }`, the rest pose with every channel of
the clip sampled at `time`; a bone the clip has no channels for keeps its rest
pose, and a one-key channel writes nothing, as in the player. Each
`BoneCorrection` then turns its `bone` by `rotation` in the bone's own space,
after the pose's rotation: the local rotation becomes `rotation * correction`.
Two corrections on one bone both apply, in list order. There is no absolute
per-bone override.

A rig is made from the content browser's New menu and edited in the inspector,
whose bone fields pick from the named skeleton's bones, or in code:

```rust
use concerto::animation::retarget::{BoneCorrection, RetargetChain, RetargetRig};

let rig = RetargetRig::new()
    .with_skeleton(skeleton)
    .with_root("pelvis")
    .with_motion_root("root")
    .with_chain(RetargetChain::new("spine", "spine_01", "spine_03"))
    .with_chain(RetargetChain::new("head", "Head", "Head"))
    .with_correction(BoneCorrection::new("upperarm_l", Quat::from_rotation_z(0.5)));
```

`rig.resolve(&skeleton, clip)` turns it into a `ResolvedRig`: the bone indices
of the root, the motion root and each chain (`chain(name)` finds one), the
reference pose in local and model space, and `root_height`, the root's height
above the model origin with model +Y up. It takes the skeleton and, for a
`ClipFrame`, the clip, which the caller loads through the rig's own handles. It
fails with a `RigError` naming the bone or chain: an invalid skeleton, an empty
or unknown bone, a motion root that is not above the root, an unnamed or
duplicate chain, a chain end not below its start, a missing clip, a time that
is not finite, a clip with no channels for the skeleton, or a root height that
is not finite or not above 0.1 mm.

The solve that plays one rig's motion on another is `PoseRetarget`, below. The
clips derived from it and the graph node that runs it are coming (#589, #590).

### Retargeting a pose

A `PoseRetarget` (`concerto::animation::retarget`) plays a pose of one
skeleton, the source, on another, the target. It is built once from the two
sides' resolved rigs, each beside the skeleton it was resolved against, and
applied as often as needed without allocating:

```rust
use concerto::animation::retarget::PoseRetarget;

let source_rig = lafan_rig.resolve(&lafan_skeleton, None)?;
let target_rig = ual_rig.resolve(&ual_skeleton, None)?;
let mut retarget = PoseRetarget::new(&source_rig, &lafan_skeleton, &target_rig, &ual_skeleton)?;

let mut pose = ual_skeleton.rest_pose.to_vec();
retarget.retarget_into(&source_pose, &mut pose)?;
```

`retarget_into(source, out)` takes a full pose of the source skeleton, in bone
order, and writes a full pose of the target into `out`. Full means with the
root motion still in it, as a clip holds it; a graph pose, net of root motion,
is not one.

Each target bone in a chain the source also has turns in model space as the
source chain turns: its model rotation's change from the target's reference
pose is the source bone's change from the source's reference pose. The rigs'
reference poses are what is compared, so each bone's local axes do not matter,
and LAFAN1's `Hips` and UAL1's `root` need no correction for theirs. What
does matter is that the two reference poses are the same pose in model space:
the same up, the same facing and the same stance, such as both in a T-pose. When
they differ, align them with a `ClipFrame` reference or `BoneCorrection`s. Chains
with different bone counts are mapped by normalised length along the chain, the
source's change slerped between the two source bones around each target bone;
a single-bone target chain takes the source chain's start.

The target keeps the rest from its reference pose. A bone in no shared chain
keeps its reference local rotation, a chain only the target has keeps its
reference pose, and a chain only the source has is ignored. Every bone but the
root and the motion root keeps its reference local translation and scale.

The target root turns as the source root does and moves by the source root's
offset from its reference, scaled on all three axes by `scale()`, the target
root's reference height over the source's. A target motion root takes the
source motion root's turn and scaled offset when the source rig has one. When
it has none, the target motion root takes the ground part of the root's motion:
its turn about +Y and its scaled offset along X and Z. The root, below it,
still reaches its full position and turn. A target without a motion root
carries all of the motion on its root. `scale()` is also the factor for
mapping a frame's root motion by hand.

`new` fails with a `RetargetError`: `RigNotForSkeleton { side }` when a rig's
pose lengths, bone indices, motion root or root height do not fit the skeleton
beside it, or the skeleton is invalid, and `NoSharedChain` when the rigs share
no chain name. `retarget_into` fails with `PoseLength { side, .. }` when the
source pose or `out` has the wrong number of bones, and then writes nothing.

Bone scale is assumed uniform: rotations are composed without it, so under a
non-uniform scale a chain bone's turn and the root's position are solved in
slightly different frames. A source pose with non-finite values gives
non-finite values in `out`.

## Gameplay helpers

`concerto::gameplay` has pieces most character games need:

- `CameraPivot` and `EntityFollow { target, offset }` make a third-person
  camera rig: put a `VirtualCamera` as a child of a pivot that follows the
  player.
- `CameraBoom { offset, shoulder, probe_radius, min_distance, return_halflife }`
  on that camera child keeps it out of walls. It has a `with_<field>` builder
  for each field. The boom hangs from a shoulder point: `shoulder` is an offset
  from the pivot in its level frame, its yaw alone, so pitching the pivot does
  not move it. Each frame the boom casts two legs, sweeping a sphere of
  `probe_radius` and ignoring the body of the pivot's `EntityFollow` target.
  The first leg runs from the pivot to the shoulder point, which stops a small
  skin short of the first hit, so a shoulder offset never reaches through a
  wall beside the character. The second runs from the shoulder point along
  `offset`, and pulls the camera in to the first hit at once, never closer
  than `min_distance`. Once the way is clear it eases back out, closing half
  the gap every `return_halflife` seconds. Only the camera's distance eases;
  the shoulder point follows at once. A zero `shoulder`, the default, hangs
  the boom from the pivot. The pivot must be a root entity.
- `ThirdPersonCamera` on the pivot eases its boomed camera between three
  framings, `follow`, `crouch` and `aim`, each a `CameraFraming { height,
  shoulder, distance, fovy, sensitivity_scale, pitch_limit, halflife }`.
  Gameplay code picks the framing by writing the rig's `stance`
  (`CameraStance::Follow`, `Crouch` or `Aim`) and which shoulder the camera
  looks over by writing its `side` (`ShoulderSide::Right` or `Left`); the rig
  reads no input. Each frame `update_third_person_cameras` eases the rig's
  current framing toward the stance's, closing half of every gap each
  `halflife` of the framing it is heading for, and writes the result onto the
  pivot's children: a `CameraBoom`'s `offset` length becomes `distance` and its
  `shoulder` becomes `height` up plus `shoulder` toward the side, along the
  view's right or left; a `VirtualCamera`'s lens takes `fovy`. A side swap
  eases the camera through the centre line rather than jumping. The defaults
  are a 3 m follow, a crouch 0.5 m lower and 2.5 m back, and an aim 0.5 m over
  the right shoulder, 1.2 m back, with a 35° field of view, half the look
  sensitivity and a 60° pitch limit. Each `CameraFraming` and the rig have a
  `with_<field>` builder per field. On a pivot with a rig, `move_camera_pivot`
  takes the pitch limit from the current framing in place of
  `CameraSettings::pitch_limit`, and multiplies `CameraSettings::sensitivity`,
  the player's preference, by its `sensitivity_scale`. Leaving aim narrows the
  limit as the framing eases, so the pitch eases back into range. The pivot
  also turns with the `Look` action, the right stick, at
  `CameraSettings::stick_turn_rate` radians per second at full deflection
  (3.0 by default), scaled the same way.
- `gameplay::actions` holds the `Move` action (WASD, the arrow keys and the
  left stick) and `Look` (the right stick). `GameplayPlugin` binds them through
  `bind_gameplay_actions`, which a test calls on its own `ActionMap`.
- `gameplay::player::spawn_first_person_player(&mut cmd, position, extra)`
  spawns a player with a camera, and the `gameplay::movement::first_person_player_fly`
  system flies it with the keyboard and mouse, for looking around a scene.

```rust
cmd.spawn((
    CameraPivot::default(),
    Transform::from_translation(spawn_point),
    EntityFollow { target: character, offset: Vec3::Y },
    ThirdPersonCamera::default(),
))
.add_child((
    VirtualCamera::new(0),
    Transform::from_translation_rotation(Vec3::NEG_Z * 3.0, Quat::from_rotation_y(PI)),
    CameraBoom::default(),
));

fn aim(rigs: Query<&mut ThirdPersonCamera>, input: Res<Input>) {
    let stance = if input.is_mouse_button_held(MouseButton::Right) {
        CameraStance::Aim
    } else {
        CameraStance::Follow
    };
    for mut rig in rigs.iter() {
        if rig.stance != stance {
            rig.stance = stance;
        }
    }
}
```

## Navigation

`NavigationPlugin`, in `DefaultPlugins`, walks characters between points you
place by hand. `concerto::navigation` holds it.

### Points and links

A `NavPoint` marks a place an NPC can walk to and through. Its `anchor` is a
`Name` that game data uses to find it, such as `"guard_post"`; leave it empty
for a point that is only a waypoint. Its `enabled` flag, on by default, says
whether a route may enter it: a disabled point is never a start, a stop or an
end. The point's +Z axis is its facing, the way an agent turns when it arrives
there.

`NavLinks` on a point lists the points it connects to. Each `NavLink` names
its `to` point and is walkable both ways; `one_way: true` lets agents walk it
only from the point that owns it. Links to an entity that has no `NavPoint`,
and links from a point to itself, are ignored. Both components are scene
components, so points are placed and linked in the editor's inspector, whose
entity picker sets each link's `to`.

There is one graph: the `NavGraph` resource, built from every `NavPoint` in
the world. It is rebuilt in `LateUpdate`, at edit time too, whenever a point
or its links are added, changed, moved or removed. Two points sharing an
anchor log a warning, and the one with the lower entity index keeps it; the
other is listed with no anchor.

`NavGraph` answers questions about the graph:

| Method | Answer |
|---|---|
| `anchor(name)` | the point with that anchor |
| `nearest(position)` | the enabled point nearest a position |
| `path(from, to)` | the shortest `NavPath` between two points, through enabled points |
| `path_from(position, to)` | the path from the enabled point nearest a position, its length counting the leg to that point |
| `point(entity)`, `points()`, `edges()` | the points, with their positions, facings and anchors, and one edge per link |

A `NavPath` holds the points it passes, their positions and its length in
metres. `path.travel_time(speed)` is the time to walk it at that speed, or
`None` for a speed that is not finite and above zero.

Nearness and arrival are measured on the horizontal plane, X and Z: a point a
step up or down a stair is as near as one on the floor. Path lengths count the
height between points.

### Agents

A `NavAgent` walks the graph. `NavAgent::new()` walks at 1.4 m/s, counts a
waypoint reached within 0.25 m and slows down within 1.5 m of the last one;
`with_speed`, `with_arrival_radius` and `with_slow_radius` change those.

Insert a `NavDestination` to send it somewhere: `NavDestination::to_point(entity)`
or `NavDestination::to_anchor(name)`, at the agent's speed, or at another with
`.with_speed(speed)` for that request alone. The agent plans from the enabled
point nearest it, and plans again when the destination changes or the graph is
rebuilt under its route. Remove the destination, or the `NavAgent`, to stop
it mid-route: its velocity drops to zero, it keeps its facing, and its route is
dropped.

Navigation decides how an agent wants to move and nothing more. Each frame it
writes `NavSteering { velocity, facing }`, a horizontal velocity toward the
next waypoint and a yaw facing along it. When the agent reaches its last point,
the velocity drops to zero, the facing becomes the point's, the destination is
removed, and a `NavArrived { agent, point }` event is sent. When no route leads
to the target, or the anchor names no point, the destination is removed and a
`NavUnreachable { agent, target }` event is sent instead.

`GameplayPlugin`'s `follow_nav_steering` copies a changed `NavSteering` into the
agent's `DesiredMotion`, so an agent that is a character, with a
`CharacterSimulation`, walks with motion matching like the player does. An
agent without a `DesiredMotion` gets its steering and nothing moves it; read
`NavSteering` yourself to move it some other way.

```rust
use concerto::name::Name;
use concerto::navigation::{NavAgent, NavArrived, NavDestination, NavLink, NavLinks, NavPoint};

let post = cmd
    .spawn((
        NavPoint { anchor: Name::new("guard_post"), enabled: true },
        Transform::from_translation(Vec3::new(4.0, 0.0, 0.0)),
    ))
    .entity();
cmd.spawn((
    NavPoint::default(),
    NavLinks { links: vec![NavLink { to: SceneEntityRef::Entity(post), one_way: false }] },
    Transform::default(),
));

cmd.spawn((
    Guard,
    NavAgent::new(),
    NavDestination::to_anchor(Name::new("guard_post")).with_speed(2.5),
    CharacterSimulation::default(),
    Transform::default(),
));

fn on_post(mut arrived: EventReader<NavArrived>, guards: Query<&mut Guard>) {
    for arrival in arrived.read() {
        if let Some(mut guard) = guards.get_entity(arrival.agent) {
            guard.on_duty = true;
        }
    }
}
```

The character's body, collider and animation come from the same set-up as the
player's, under [Characters](#characters). The tech demo's grid level has an
NPC that patrols a square of four points this way.

### Drawing the graph

The `concerto-navigation-gizmos` crate, which a game adds itself, draws each
point as a ring with a line along its facing, green when enabled and grey when
not, each link as a line with an arrowhead on a one-way link, and the route
each agent has left. Register `NavigationGizmosPlugin` after `DefaultPlugins`;
the `NavigationGizmos` resource turns the graph and routes layers off. The
graph is drawn in the editor's scene view as well, while points are placed.
Its [README](../../crates/navigation-gizmos/README.md) says more.

### Limits

Navigation is a graph of hand-placed points, and only that:

- Agents do not avoid each other or the player. Their capsules collide in
  physics, and that is all.
- A route is planned over the graph alone. A physics body standing on it is not
  walked around, and a blocked agent does not plan again.
- The nearest point an agent starts from is found by distance, without a check
  that it can be seen or reached in a straight line.
- Links are walked, nothing else: no doors, ladders or vaults.
- There is no navmesh yet; it is tracked in #678.

Next: [User interface](08-ui.md).

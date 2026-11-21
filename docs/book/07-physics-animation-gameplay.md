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
bottom of a capsule, where a character's feet are. A `GravityScale` beside the
`Collider` multiplies gravity for the body; `GravityScale(0.0)` makes a body
that moves only by the velocities it is given, as the first-person walker's
is.

A static collider's `Transform` is yours: the simulation writes poses back
only to entities with a `RigidBody`. Moving the entity does not move its body,
though; insert its `Collider` or `BoxCollider` again to rebuild the body where
the entity now is, as a door does when it swings.

The body is built from the `Collider` once. Inserting a new `Collider` on the
entity rebuilds its body from the new shape, which is how editing a
`BoxCollider` in the inspector or over MCP takes effect. A `BoxCollider` on an
entity that also has a `MeshCollider` wins over the mesh.

A `MeshComponent` with no material is drawn plain grey, so a `MeshCollider`
entity like the floor above is visible. Give it a material to choose how it
looks.

### Collision layers

A `CollisionLayers` beside a `Collider` says which layers the body is in
(`memberships`) and which it collides with (`filter`). Two bodies collide when
each is in a layer the other collides with. Without the component a body is in
`Layers::DEFAULT` and collides with every layer.

```rust
use concerto::physics::layers::{CollisionLayers, Layers};

const FOLIAGE: Layers = Layers::bit(Layers::FIRST_FREE);

cmd.spawn((Collider::cuboid(1.0, 2.0, 1.0), CollisionLayers::new(FOLIAGE, Layers::ALL), transform));
cmd.spawn((RigidBody::default(), Collider::sphere(0.1), CollisionLayers::new(Layers::DEFAULT, Layers::ALL.without(FOLIAGE)), transform));
```

There are fourteen layers. The engine names three: `Layers::DEFAULT`,
`Layers::CHARACTER` for character capsules and `Layers::RAGDOLL` for ragdoll
parts; `Layers::bit(n)` from `Layers::FIRST_FREE` up are a game's own.
`CollisionLayers::character()` and `CollisionLayers::ragdoll()` are the two
that never collide with each other. Two static bodies never collide whatever
their layers, and a sensor detects only the moving bodies its layers match;
without a `CollisionLayers` a sensor ignores ragdoll parts, so a character
entering it is reported once.

Changing the component changes the live body. `CollisionLayers::NONE` takes a
body out of every layer: it collides with nothing.

A scene query sees the bodies in any of `QueryFilter::layers`. The default is
every layer but `Layers::RAGDOLL`; `QueryFilter::default().with_layers(FOLIAGE)`
sees only foliage, and `Layers::ALL.without(FOLIAGE)` lets a camera boom pass
through it.

### Collision steps

The fixed step is 33 ms. Physics divides it into the fewest equal collision
steps of at most 1/60 s, two at 33 ms, so fast bodies, stacks and joints stay
stable. `PhysicsSettings::max_collision_step` sets the limit:

```rust
app.insert_resource(PhysicsSettings { max_collision_step: 1.0 / 120.0 });
```

`FixedUpdate` still runs once per fixed step; only the simulation is divided.

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


### Contacts

A `Contacts` component makes a body report what it touches. Without it a body
reports nothing, so ordinary bodies cost nothing. After each fixed step it
lists every solid body touching this one: the `other` entity, the world
`point`, the `normal` pointing from the other body towards this one, and the
`impulse` in N·s that the contact applied over that step.

```rust
use concerto::physics::contact::{ContactStart, Contacts};

cmd.spawn((RigidBody::default(), Collider::sphere(0.2), Contacts::default(), transform));

fn feet_down(feet: Query<&Contacts, With<Foot>>) {
    for contacts in feet.iter() {
        let grounded = contacts.iter().any(|contact| contact.normal.y > 0.7);
    }
}
```

`ContactStart` and `ContactEnd` are signals on the reporting entity, each
carrying `other`, sent when a body begins and stops touching it, or goes away
while touching. A body asleep on another keeps its contact listed, with the
impulse of the last step it was simulated in. Sensors never appear here; they
have their own signals.


### Ragdolls

A ragdoll is a tree of bodies joined by swing-twist joints, simulated by Jolt's
own ragdoll. It is described as plain data and built once:

```rust
use concerto::physics::ragdoll::{BodyPose, JointSettings, PartSettings, PartShape, RagdollSettings};

let link = |index: usize| {
    PartSettings::new(
        BodyPose::from_translation(Vec3::new(0.0, -0.5 * index as f32, 0.0)),
        PartShape::Capsule { half_height: 0.15, radius: 0.08 },
    )
    .with_shape_offset(BodyPose::from_translation(Vec3::new(0.0, -0.25, 0.0)))
    .with_mass(2.0)
};
let joint = JointSettings::new(Vec3::NEG_Y, Vec3::X).with_swing(0.6, 0.6).with_twist(-0.3, 0.3);
let built = RagdollSettings::new()
    .with_part(link(0))
    .with_part(link(1).with_parent(0, joint))
    .build()?;
```

Each part has a pose in one shared reference pose, a shape with its offset
from the part, a mass, a friction coefficient, and the joint to its parent,
which sits at the part's origin. Two bodies in contact slide by the geometric
mean of their coefficients; a part has 0.5 unless `with_friction` says
otherwise, and bodies that are not ragdoll parts have 0.2. The first part is the root; every other part names an earlier one as
its parent. `build` balances the masses so the joints stay stable and stops
each part colliding with its parent. No part ever moves faster than the settings'
`max_speed`, 50 m/s by default, however hard it is hit. The solver runs at least
`velocity_steps` iterations, 20 by default, on anything joined to a ragdoll:
the 10 it gives other bodies cannot carry a heavy trunk through a chain of
light parts, and powered joints then give under the load.

`PhysicsState::create_ragdoll(&built, owner, &pose)` adds one to the world with
each part at its world pose and returns a `RagdollId`; `destroy_ragdoll`
removes it. `ragdoll_pose` reads every part's world pose and `set_ragdoll_pose`
moves them all at once. `ragdoll_bodies` gives each part's `BodyId`, for
velocities and impulses. A ray or sphere cast that hits a part reports the
`owner` entity and the part's index in `RayHit::part`. `RagdollId` is also a
component: put it on the owner, and removing it or despawning the owner
destroys the ragdoll.

A ragdoll hangs from its joints until something drives it:

- `drive_ragdoll_motors(id, &pose)` turns every joint's motor on, aimed at the
  rotation each part has relative to its parent in `pose`. The body reaches for
  the pose and holds it against gravity as far as its motors can.
- `set_ragdoll_joint_drive(id, part, drive)` sets one joint: `JointDrive::Off`,
  or `JointDrive::Target(JointTarget { rotation, motor })` with a
  `MotorSettings { frequency, damping, torque_limit }`. A low torque limit
  makes a weak muscle that sags.
- `ragdoll_joint_motor_impulse(id, part)` reads what the motor applied in the
  last collision step, which tells a controller that a muscle is saturated.
- `set_ragdoll_part_motion(id, part, MotionType::Kinematic)` takes one part out
  of the simulation's hands, so a kinematic pelvis can carry a powered torso.
- `drive_ragdoll_kinematic(id, &pose)` sets each part's velocity so it reaches
  `pose` at the end of the next fixed step. Kinematic parts follow it exactly
  and push dynamic bodies aside.

Ragdolls exist on Jolt only. On Rapier, and so in the browser, `build`
succeeds and `create_ragdoll` returns `None`.

### Debug drawing

The `concerto-physics-gizmos` crate, which a game adds itself, draws what the
physics holds. Register `PhysicsGizmosPlugin` after `DefaultPlugins`; the
`PhysicsGizmos` resource turns four layers on, all off by default:

- `colliders` outlines every `Collider`: a sphere, a box, a capsule, or the
  bounds of a mesh shape, carried by the entity's transform and its
  `ColliderOffset`. Grey-blue is static, cyan kinematic, green dynamic and
  violet asleep.
- `ragdolls` draws each part's shape in the same colours, and at each joint
  its frame, its swing cone from `normal_half_cone` and `plane_half_cone`, its
  twist range as an arc, and the child's current twist axis in white, so you
  see how close the joint is to a limit.
- `contacts` draws a red cross at each contact of a body that holds
  `Contacts`, with a line along its normal.
- `sensing` draws what a `BodySense` perceives: the centre of mass with a line
  down to the ground, the support outline, the capture point (green when the
  body is balanced, red when not), the centre's velocity and any predicted
  impact.

In the editor the layers are component gizmo kinds, once the game lists
`concerto-physics-editor`: `Collider`, `Ragdoll` (on the entity that holds the
`RagdollId`), `Contacts`, and, drawn from the authored components where no body
exists yet, `Box collider`, `Mesh collider` and `Sensor`, plus `Body sense` for a `BodySense`. The viewport's Gizmos
menu and the `gizmos` tool turn each on by name, for the selection or for every
entity. In a game without the editor, use the `PhysicsGizmos` resource. Both
need `DebugGizmosPlugin`, which the plugin adds when the `App` has a renderer.
The crate's [README](../../crates/physics-gizmos/README.md) says more.

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

Those reads merge every device. A couch game reads each local player's own:
the `Players` resource maps a `PlayerIndex` to that player's pad and, during
development, the keyboard and mouse. `WindowPlugin` inserts it with joining
off; after the plugins are added, call `players.set_settings(..)` with
`JoinSettings` that turn on `join_on_press`, and an unassigned pad that presses
South or Start joins at the lowest free index, up to `max_players`.
`keyboard_mouse_joins` lets Enter or Space join the keyboard too. A game's own
assignment screen calls `players.assign(index, device)` and `unassign`
instead. When a player's pad disconnects they keep their index and
`players.waiting()` lists them, so the game can pause; the same pad
reconnecting gives it back, and with `join_on_press` another pad pressing a
join button can take the place, as long as it is below `max_players`.
`PlayerEvent` says when a player joins, leaves, loses or regains a pad.
`actions.player(index, &input, &players)` reads one player's actions: only
that player's devices count, `mouse_delta()` is zero for anyone without the
mouse, and `push_context_for(index, context)` gives that player a context the
other doesn't share. Context stacks are kept by index, so clear one with
`clear_contexts_for` when `PlayerEvent::Left` says its player left.
`ActionFired` carries the player whose device fired it.

```rust
fn walk_each(
    input: Res<Input>,
    actions: Res<ActionMap>,
    joined: Res<Players>,
    players: Query<(&PlayerIndex, &mut DesiredMotion)>,
) {
    for (index, mut desired) in players.iter() {
        let push = actions.player(*index, &input, &joined).axis2d(Move);
        desired.velocity = Vec3::new(push.x, 0.0, -push.y) * 5.0;
    }
}
```

The physics step runs in `LateFixedUpdate`, at a fixed rate; transforms of
simulated bodies are interpolated between steps for display.

### The ragdoll asset

A `Ragdoll` asset describes the physical body of one skeleton, authored once
and shared by every character on it. It names its `skeleton` and lists its
parts. Each `RagdollPart` follows one bone and has a `RagdollShape` (capsule,
box or sphere) with its `offset` and `rotation` in the bone's space, a `mass`,
and the `RagdollJoint` to its parent part: a twist axis and a plane axis in the
bone's space, the two swing half angles, the twist range, a friction torque
and the motor's spring.

```rust
use concerto::ragdoll::{Ragdoll, RagdollJoint, RagdollPart, RagdollShape};

let ragdoll = Ragdoll::new(skeleton)
    .with_part(RagdollPart::new("pelvis", RagdollShape::Box { half_extents: Vec3::new(0.15, 0.1, 0.1) }).with_mass(10.0))
    .with_part(
        RagdollPart::new("thigh_l", RagdollShape::Capsule { half_height: 0.15, radius: 0.07 })
            .with_offset(Vec3::new(0.0, -0.2, 0.0))
            .with_mass(7.5)
            .with_joint(RagdollJoint::new(Vec3::NEG_Y, Vec3::X).with_swing(1.0, 0.8).with_twist(-0.4, 0.4)),
    )
    .with_part(
        RagdollPart::new("calf_l", RagdollShape::Capsule { half_height: 0.15, radius: 0.05 })
            .with_offset(Vec3::new(0.0, -0.2, 0.0))
            .with_mass(3.5)
            .with_joint(RagdollJoint::hinge(Vec3::X, 0.0, 2.4)),
    );
```

A ragdoll has far fewer parts than its skeleton has bones. The parts may be
listed in any order: each is joined to the part of its nearest ancestor bone
that has one, and a bone without a part moves with that ancestor's part. The
parts must form one tree.

`ragdoll.resolve(&skeleton)` fits the asset to the skeleton and builds the
physics recipe, or says why it does not fit: a bone the skeleton lacks, two
parts on one bone, or a part outside the tree. The `ResolvedRagdoll` gives the
`BuiltRagdoll` for `PhysicsState::create_ragdoll`, the bone each part follows,
and for each bone the part it moves with.

The asset shows in the Asset Browser, where New creates one, and opens in the
inspector's form.

#### Generating a humanoid

Typing fifteen parts by hand is an afternoon of guessing. `generate_humanoid`
gives a first draft to adjust: a pelvis, up to three spine parts, a head, upper
and lower arms, hands, thighs, shins and feet. Each foot is a flat box whose
underside is the sole, so a standing body rests on two faces and not on two
rolling capsules. A part whose segment is `Foot` is built with a friction of 4,
which holds a planted foot where it is; every other part has 0.5.

```rust
use concerto::ragdoll::{HumanoidSettings, generate_humanoid};

let rig = retarget_rig.resolve(&skeleton, None)?;
let ragdoll = generate_humanoid(skeleton_handle, &skeleton, &rig, &[&body_mesh], &HumanoidSettings::default())?;
```

It needs the skeleton's `RetargetRig`, which already says which bones are the
spine, the neck, the arms and the legs. `HumanoidChains` names the rig's chains
for each; the defaults are `spine`, `neck`, `arm_l`, `arm_r`, `leg_l` and
`leg_r`. An arm uses the last three bones of its chain, so a chain may start at
the clavicle; a leg uses the first three, so it may end at the toe.

Capsules run along each bone to the next. A capsule is as thick as the mean
distance from the bone of the mesh vertices skinned to it, or in proportion to
the bone's length when no mesh is given. `total_mass`, 75 kg by default, is
split by body-segment fractions. Knees and elbows are hinges that bend one way
only, shoulders and hips get a swing cone and a twist range, the spine and neck
small ones, and every joint a little friction so a fallen body comes to rest.

In the editor, a retarget rig's tab has a **Create ragdoll** button, and an
agent has the `create_ragdoll` tool; both come from the extension crate
`concerto-ragdoll-editor` and send one signal, `CreateRagdoll`. It reads the
rig, its skeleton and its reference clip, runs `generate_humanoid`, checks the
result with `Ragdoll::resolve`, and saves `ragdoll.gasset` beside the rig, or
at an `address` you give. The tool takes `rig`, and optionally `address`,
`total_mass` and `chains`, and answers one line per part with its bone, shape
and mass. It reads no skinned mesh, so part radii follow the bones'
proportions. Given a skeleton, it says to create a rig first.

A `Ragdoll` opens in its own tab, from the same crate. The left panel is the
part tree, each part under the part of its nearest ancestor bone, labelled with
its bone; the selected part's shape, offset, rotation, mass and joint (axes,
swing, twist, friction, motor) are the **Part** form, written with the
inspector's usual number fields, radians for angles. **Add part** lists the
bones that have none, and **Remove part** drops the selected one. The preview
stands the skeleton's character, with its skinned mesh when the skeleton's
folder has a `scene`, on a floor, with `concerto-physics-gizmos` drawing each
part and each joint's limits over it, and the buttons **Drop**, **Poke**,
**Play** and **Reset** act on it: Drop lifts the body to `drop_height` and lets
it go limp, Poke hits the part on `poke_bone` with `poke_impulse` (making the
body limp first), Play plays the `clip` of the **Preview** value with the
ragdoll following it kinematically, and Reset stands it in its rest pose again.
Powered playback, where the body pulls itself toward the clip, does not exist
yet. Drop and Poke also run the headless trial `run_trial` on the tab's current
value and print its report under the buttons.

The tools do the same without a screenshot. `ragdoll_parts` lists the parts:
bone, parent part, shape and sizes, mass, and the joint's limits. `set_ragdoll_part`
sets one bone's shape, `radius`, `half_height`, `half_extents`, `mass`,
`offset`, swing and twist limits and `friction`, adds a part on a bone that has
none, or `remove`s one, on the ragdoll open in a tab; the tab becomes unsaved,
and `save` writes it. `test_ragdoll` runs a drop (`height`, 0.5 m by default)
or a poke (`bone`, `impulse`) for `seconds` (5 by default) in a physics world of
its own and answers whether the body came to rest and when, its peak speed, the
widest its joints opened and which pairs of parts interpenetrate in the rest
pose, by bone with the depth in metres. It reads the tab's value, saved or not,
so a loop of `set_ragdoll_part` and `test_ragdoll` finds an overlap and fixes it
before anything is written. `preview_ragdoll` is the buttons' other half: it
sends the tab's preview a drop, poke, play or reset.

With the `test-support` feature, `concerto::ragdoll::testing` has a reference
humanoid built in code (`reference_humanoid`) and a `RagdollWorld` to stand it
in, hit a named part and step, without a GPU.

### A character's ragdoll

`CharacterRagdoll` gives a character a physical body. It goes on the entity
with the `SkeletonComponent`, names the `Ragdoll` asset of that skeleton, and
optionally the character's `capsule`:

```rust
use concerto::ragdoll::CharacterRagdoll;

cmd.insert(CharacterRagdoll::new(ragdoll).with_capsule(capsule), skeleton_entity);
```

Once the asset and the skeleton have loaded, the ragdoll is built at the bones'
current pose and the entity gets a `RagdollId` and a `RagdollBody`. Removing
`CharacterRagdoll`, or despawning the entity, destroys it; inserting it again
rebuilds it. An asset that does not fit the skeleton is reported once and the
entity is marked `RagdollRefused`.

Its `mode` starts as `RagdollMode::FollowsAnimation`: every part is kinematic, and
each fixed step, before physics steps, it is driven to its bone's pose. The
parts trail the drawn pose by up to one frame. They push dynamic props, so an
arm swing knocks a bottle over, and they are already in the right place at the
right speed the moment something disturbs the character. The animation is not
changed.

`RagdollMode::Limp` lets the body go: every part turns dynamic with its motors
off and keeps the linear and angular velocity it had while it followed the
animation, so a running character that goes limp keeps travelling.

`RagdollMode::Powered` is the state between the two: the body tries to do what
the animation says and physics decides how well it manages. Each part is held
as the character's strength `profile` says, one of the `StrengthProfile`s in
the ragdoll asset:

- `PartStrength::Kinematic`: the part follows its bone exactly;
- `PartStrength::Powered { stiffness, damping, torque_limit }`: the part is
  dynamic and its joint's motor pulls it towards its bone's rotation under its
  parent's, with a spring of `stiffness` N·m/rad and `damping` N·m·s/rad, and
  no more than `torque_limit` N·m. The target leads the bone by the
  animation's own angular speed, so a fast clip is tracked without lag;
- `PartStrength::Limp`: dynamic with the motor off.

```rust
ragdoll.power("flinch");
```

`power` sets the mode, the profile and a weight of 1. A generated humanoid has
three profiles: `flinch`, with the pelvis and legs kinematic and everything
above powered, which takes a hit without needing balance; `braced`, with every
part powered; and `limp`. A change of profile takes hold over the component's
`blend` seconds, except to and from kinematic, which is a step.

Strength is a stiffness and not a frequency: a frequency scales the spring by
the two parts a joint joins, so an upper arm's motor would be too weak to hold
the forearm and hand that hang from it. A torque limit of half a newton metre
makes a muscle that cannot hold its limb up at all.

When the root part is not kinematic and has been further than `lost_distance`
from its animated place for `lost_seconds`, the character is sent `PoseLost`,
once, until it is back: it can no longer hold the pose, and something should
catch it or let it fall.

#### A hit

`BodyHit { character, part, point, impulse, kind }` says a character's body was hit:
on this part of its ragdoll, at this world point, with this impulse in N·s. It
says nothing about damage or death; the game decides those. Trigger it from
the ray that found the part:

```rust
let hit = physics.cast_ray(muzzle, aim, QueryFilter::default().with_layers(Layers::RAGDOLL))?;
cmd.trigger(BodyHit::new(hit.entity?, hit.part?, hit.point, aim.normalize() * 30.0));
```

It is triggered for every listener, so the engine and the game both hear it. A
9 mm bullet carries about 3 N·s, which barely moves a 75 kg body; games scale
it up for the look, and the signal carries whatever the game sends.

A character with a `HitReaction` flinches when it is hit while its ragdoll
follows its animation:

1. the body is powered with the reaction's `profile`, `flinch` by default, so
   the pelvis and legs keep following the animation and need no balance;
2. the part takes the impulse at the point;
3. the hit part's strength drops to `dip` of itself and its neighbours' to
   `neighbour_dip`, and comes back over `recover` seconds;
4. the simulated body is drawn in full for `hold` seconds after the last hit,
   then returns to the animation over `settle` seconds, after which the parts
   are kinematic again.

A second hit during a flinch restarts the recovery and adds its impulse. A hit
on a part the profile keeps kinematic, a leg, moves nothing. Without a
`HitReaction`, or when the ragdoll is limp or powered by something else, the
part only takes the impulse. `FlinchRecord` on the character holds what its
last flinch did: the part, how far it got from its animated place, and how long
the flinch lasted.

What is drawn is a separate matter, set by weights. `weight` runs from 0, the
animated pose, to 1, the ragdoll's; `part_weights` gives single parts a weight
of their own by bone name. `go_limp()` sets the mode and a weight of 1:

```rust
fn die(characters: Query<&mut CharacterRagdoll, With<Dead>>) {
    for mut ragdoll in characters.iter() {
        ragdoll.go_limp();
    }
}
```

Each frame, after the animation has written the bones and before transforms
propagate, the bones of parts with a weight above 0 are moved towards the
ragdoll's pose, interpolated between the last two fixed steps so the body does
not judder at the fixed rate. A bone with no part keeps its animated pose under
its parent, so fingers and a face still animate on a limp body. Bones are
placed in world terms, so the mesh stays on the body wherever the character's
own entity is. Changing a weight over time is how animation and physics are
blended. While a character whose parts have weight is itself moving, the drawn
body trails it by one frame's motion.

While the character's root part, its pelvis, has any weight, the character is
a body and not a walker, and its `capsule` follows the body instead of driving
it:

- the capsule gets a `Carried` component, and whatever drives it leaves it
  alone: the motion-matching springs, and so nav steering and root motion, and
  the first-person walker. Held input moves nothing;
- each fixed step it is put on the ground under the ragdoll's pelvis, so when
  the character returns to animation it is already there and nothing teleports;
- it is in no collision layer while it is carried, so it collides with nothing
  and no query sees it; it gets its own layers back when it is let go;
- when every weight at the pelvis is back at 0 it is let go where it stands,
  and the springs restart from the body's velocity and facing, which `Carried`
  holds, so the first animated frame does not snap.

The capsule is not checked for a clear standing spot when it is let go; a body
that came to rest inside a wall's reach is pushed out by the solver.

#### What the body senses

A `BodySense` beside a `CharacterRagdoll` is rewritten after each fixed step
with what a behaviour needs to know to keep the body on its feet:

- `centre`: the centre of mass, the body's mass and the centre's velocity;
- `supporting` and `support`: the feet that are on the ground and the convex
  outline under them. A simulated foot is on the ground by its contacts, a
  kinematic one when the ground is within 5 cm under it;
- `capture_point`: where the body would have to step to come to rest, the
  ground under the centre of mass plus its horizontal velocity times
  `sqrt(h / g)`. `balanced()` says it is inside the support, so the body can
  recover without a step; `off_balance` is how far outside it is and which way;
- `lean`, the angle of the pelvis-to-chest axis from upright, and `trunk_spin`;
- `airborne`, the seconds since any part last touched something else;
- `chest_impact` and `head_impact`, while no foot supports the body: where and
  when each will meet something on its current path;
- `at_rest`: every part slower than 0.2 m/s for half a second.

Each part of the ragdoll asset says what `segment` of the body it is and which
`side`; the generator fills them in. `ResolvedRagdoll::landmarks()` names the
pelvis, the spine parts, the chest, the head and each side's arm and leg parts
from them. While a body has a `BodySense`, its parts report their contacts.

#### Behaviours

A body knocked off balance needs targets no clip contains: a foot placed where
it stops the fall, arms thrown out, hands reaching for the ground. A behaviour
computes them. Every behaviour writes the same thing, to the character's
`BodyDrive`: for each part it claims, a `PartCommand` with a target rotation
relative to the parent part, a `PartStrength`, and optionally a world-space
force and torque.

```rust
fn raise_arms(characters: Query<(&RaiseArms, &RagdollBody, &mut BodyDrive)>) {
    for (_, body, mut drive) in characters.iter() {
        let Some(arm) = body.resolved().landmarks().left.upper_arm else { continue };
        let command = PartCommand::held(PartStrength::powered(500.0, 120.0))
            .with_target(Quat::from_rotation_z(1.0));
        drive.claim("raise_arms", arm, 1, 1.0, command);
    }
}

app.add_system(FixedUpdate, raise_arms.in_set(BehaviourSet));
```

A behaviour is a plugin: a component that holds its parameters and state, and
a system in `BehaviourSet`, in `FixedUpdate`, that claims parts every step it
runs. Adding the component starts it and removing it stops it, by a signal of
the behaviour's own or directly. For each part the claim with the highest
priority wins, blended by its weight over what is below it; a behaviour fades
in or out through its weight. The animated pose with the character's mode and
strength profile is below every claim. A part pinned between kinematic
neighbours cannot move, so a behaviour that moves an arm claims the whole arm.
When a behaviour ends by itself it triggers `BehaviourEnded` with
`Outcome::Succeeded` or `Outcome::GaveUp`, so the next can start.

`PhysicalState` on the character says what it is doing:

- `Animated`: its parts follow the animation;
- `Reacting`: on its feet and powered, flinching or driven by behaviours;
- `Falling`: no longer supported. A behaviour that gives up its balance sets it;
- `Down`: a falling or limp body lying at rest;
- `Dead`: limp for good. A game or the dying behaviour sets it, and nothing the
  engine does moves a character out of it.

The engine moves it between `Animated` and `Reacting` and to `Down`. While
anything claims a part the simulated body is drawn in full, and the drawn body
returns to the animation when the state returns to `Animated`. Every change
triggers `BodyStateChanged { character, from, to }` for every listener; react
to that instead of polling.

`describe_body` gives the state, the running behaviours with their priorities
and parts, and the sensing values as text, which is what the editor's
`body_report` tool replies with.

#### Balance

`Balance` is the behaviour that keeps a pushed body on its own feet. Add it to
a character that has a `BodyDrive`, a `BodySense` and a `PhysicalState`:

```rust
cmd.entity(character).insert(Balance::default());
```

It powers the body with its `profile`, `braced` by default, and from then on,
each fixed step:

- it holds the hips, knees and ankles with `leg_stiffness`, and claims the
  pelvis and legs at `BALANCE_PRIORITY`, so a flinch still moves the trunk
  and arms above them;
- it assists the pelvis, since joint torques alone do not hold up a body that
  has no ankle strategy: a torque towards the animated orientation, a force
  towards the animated height, a force against the sway of the centre of mass,
  and, only while the capture point is outside the feet, a force that pushes
  it back. Each is capped (`max_upright_torque`, `max_lift`, `max_centring`),
  so a hard push still overwhelms them. `with_assist(0.0, 0.0, 0.0)` leaves
  the joints alone, and the body falls easily;
- when the capture point is more than `step_margin` outside the feet, it
  steps: the foot further from the point lifts and lands `step_beyond` past
  it, at most `max_step` of the leg's length away, in `step_seconds`, bent
  there by two-bone IK. A point to the side is caught by the foot on that
  side, so the legs never cross. Both feet rest `step_rest` before the next.

It ends by itself, removing the component and triggering `BehaviourEnded`:
with `Outcome::Succeeded` once the body has been balanced and slower than
`settle_speed` for `settle_seconds`, and the character returns to following
its animation; with `Outcome::GaveUp` when the trunk leans past `max_lean`, the
feet leave the ground, a step has no ground to land on, or `max_steps` steps
did not bring the point back, and
`PhysicalState` becomes `Falling`. A fall behaviour starts from that signal.

With the defaults the reference humanoid, 75 kg, absorbs a 12 N·s push on the
chest without a step, catches a 70 N·s push in one or two steps, and gives up
under 300 N·s.

#### Stagger

Balance recovers as well as it can, which reads as a robot. `Stagger` makes
the same push a stumble. Add it when a character is shoved or hit, with the
way it was sent and a severity from 0 to 1:

```rust
cmd.entity(character).insert(Stagger::new(push_direction, 0.5));
```

It requires a `Balance`, so the character gets one if it has none, and it runs
on top of it instead of placing the feet itself:

- **Legs.** Each step it calls `Balance::hinder` with a `Hindrance`: steps
  cover less of the distance balance wants (`short_steps`), take longer
  (`slow_steps`), the legs and balance's assists are weaker (`weak_legs`), and
  the knees lose strength for a quarter of a second after each landing, and
  the body may take `more_steps` steps on top of balance's `max_steps`. All of
  it scales with the severity and wears off over `seconds`. Any behaviour can
  hinder balance the same way.
- **Trunk.** The spine bends towards the direction, by `bend` radians at
  severity 1, and straightens as the stagger wears off.
- **Arms.** Both arms circle about the shoulders in the sense the trunk is
  tipping, which slows it, as wide as the chest's angular velocity is fast
  (`full_spin`). As the body steadies they come down. The cycle is authored,
  not solved from the angular momentum.

It claims the spine and the arms at `STAGGER_PRIORITY`, above balance and
below nothing the engine has yet. It ends when balance ends and triggers
`BehaviourEnded` for `stagger` with the same outcome. On the reference
humanoid a 70 N·s push with severity 0.3 takes more steps than balance alone
and ends standing; with severity 1 it ends `Falling`. Between those the
outcome is in doubt, as it should be: do not build game logic on a stagger of
a given severity surviving.

#### Shot

A flinch bends the upper body over legs that never move. A character with a
`ShotReaction` answers a gunshot with its whole body, and differently by where
the bullet struck. The game marks the hit:

```rust
cmd.trigger(BodyHit::new(character, part, point, impulse).shot());
```

`HitKind::Shot` on a character that has a `ShotReaction` and is `Animated` or
`Reacting` starts a `Shot` beside it, which brings a `Balance`; the flinch
does not run for that hit. Any other character, and a blow, flinches as
before. While the shot runs:

- **Snap.** The bullet's impulse lands on the struck part. The muscles of that
  part and the parts joined to it are `spike` times stronger for
  `snap_seconds`, then drop to `slack` and come back to `wounded` over
  `recover`.
- **Wound reach.** After `reach_delay` the nearer hand goes to the newest
  wound by two-bone IK, the elbow turned onto its hinge, and stays. A hand
  does not reach for the head, a hand, or with an arm that was itself shot.
- **Look.** The head turns to the wound for `look_seconds`, then to where the
  shot came from, never more than `max_look` from the animation.
- **By region**, which is the `segment` of the struck part:
  - pelvis or spine: the spine folds over the wound by `fold` and straightens
    over `fold_seconds`; balance takes the momentum;
  - arm or hand: that arm hangs at `hanging_arm` of its strength;
  - thigh, shin or foot: that leg keeps `lame_leg` of its strength through
    `Hindrance::left_leg` or `right_leg`. Balance sags the knee of a leg under
    a half, so its foot leaves the ground, and never stands on it. With both
    legs shot the knees fold, the pelvis sinks under `Balance::min_height` of
    its standing height and balance gives up;
  - head: nothing but the impulse. A game that kills on a head shot sends the
    dying signal.
- **Repeat hits.** Each hit adds a wound with its own snap and region effect;
  the hand goes to the newest.

It claims parts at `SHOT_PRIORITY`, above stagger. It ends when balance ends
and triggers `BehaviourEnded` for `shot` with the same outcome: standing and
`Animated`, or `Falling`. `Wounds` stays on the character afterwards:
`strength_of(part)` is the share of its strength each part kept, for the game
to turn into a limp or a guarded arm. The engine does not apply it outside the
reaction.

A body standing on one leg may take a long time to settle or never settle;
the reaction then lasts until the game ends it by removing `Shot` and
`Balance`, or the character dies.

Ragdoll parts are in `Layers::RAGDOLL` and collide with everything but
`Layers::CHARACTER` (`CollisionLayers::ragdoll()`, or `RagdollSettings::layers`
to change it). Give the character's capsule `CollisionLayers::character()`:
then no ragdoll part ever touches a capsule, its own or another character's, a
default scene query sees the capsule and never a limb, and a shot asks for the
limbs with `QueryFilter::default().with_layers(Layers::RAGDOLL)`. A capsule left
on the default layer is pushed around by its own body; the engine warns when a
`CharacterRagdoll` names one.

A hit on a part reports the character and the bone:

```rust
let hit = physics.cast_ray(origin, direction, QueryFilter::default().with_layers(Layers::RAGDOLL))?;
let body = bodies.get_entity(hit.entity?)?;
let bone = body.bone(hit.part?);
```

Nothing happens in the editor's edit and preview `App`s, where physics is not
stepped.

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
    graph.result_node().add_blend_space_2d_input_with(name!("movement"), |space| {
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
  graph
      .result_node()
      .input(AdditiveNode::new(cold).with_weight_param(name!("cold")))
      .add_input(AnimationClipNode::new(walk));
  ```
- **Layer** (`LayerNode`) lays its second input, the overlay, over its first,
  the base, on the bones a `BoneMask` asset names. Build it with
  `add_layer_input(node, base, overlay)`, which takes the base node and then
  the overlay node, so the order can't come out wrong. When either has inputs
  of its own, `add_layer_input_with(node, base, overlay)` adds the base through
  its first closure and the overlay through its second; it panics unless each
  closure adds exactly one input. Layers
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
  graph.result_node().add_layer_input(
      LayerNode::new(upper_body).with_weight_param(name!("aim_weight")),
      AnimationClipNode::new(run),
      AnimationClipNode::new(aim),
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
  to the old one with `blend_time` as the halflife. A built machine is read
  with `states()`, `initial_state()` and `transitions(from)`, and edited by
  state name with `add_state`, `remove_state`, `rename_state`,
  `set_state_graph`, `set_initial`, `add_transition`, `remove_transition` and
  `set_transition`; a transition is named by its source state and its index
  among that state's transitions, and each edit returns a
  `StateMachineEditError` for an unknown state, a duplicate name or an index
  out of range. Removing a state removes every transition to or from it.
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
  `seed_param`, a `u64` param, 0 when there is none: players with
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
A node names the parameter it reads with a `Param<T>`
(`concerto::animation::param`): a `Name` that carries the type of the value
the node reads, so `weight_param` is an `Option<Param<f32>>` and a look-at's
`target_param` a `Param<Vec3>`. Every `with_*_param` builder, the state
machine's `on_bool` and `on_non_zero_vec`, and `add_blend_space_2d_input_with`
take a `Name` or a `Param<T>`; a struct literal writes
`name!("aim_weight").into()`. `AnimationBlackboard::read(&param)` returns the
value as that type, and `None` when it is missing or holds another. A
`Param<T>` is stored as its `Name`, and a plugin registers each `Param<T>` its
nodes hold with the type registry, as it registers their asset handles. `T` is
`Default`: the registration carries `ReflectParam`, which gives an editor the
parameter's name, the value's type and its name, and a `Default` value of that
type to edit, and the graph tab lists a parameter only through it.
`AnimationPlayer::set_boxed_param(key, value)` sets a parameter to such a
`Box<dyn Reflect>`.
A node can read its playback rate from a float parameter: the rate is the
node's own rate times the parameter, and no parameter (`None`, the default), or
one that is missing or holds another type, leaves the node's rate alone. A product that is negative or not
finite holds the node at rate 0.
`crates/animation/tests/animation/graph_serialization.rs` builds a graph with a blend
space and a state machine. The tech demo's character plays no authored graph:
`src/character.rs` in the tech demo's repository gives it one
`MotionMatchingNode`, which
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
    // input_slots, create_state, update, evaluate, reset; is_finished defaults to true
}
```

The node's fields are its data and are stored with the graph. `State` is the
per-player runtime state (a playhead, say): `create_state` makes one for each
player that uses the graph, and `update`, `evaluate`, `reset` and `is_finished`
receive it. `AnimationPlayer::play(node, graphs)` calls the node's `reset`, so
a node restarts on `play` by resetting its state there. A node that plays graphs
inside itself, as the state machine and the retarget node do, reports them
through `inner_graphs` and their states through `inner_states`, each with the
state that plays it when the node is a state machine, and, under `cfg(test)` or
the `test-support` feature, mutably through `inner_states_mut`. `report` says
what the node shows an editor of its state as it plays, a `NodeReport`; it is
empty unless the node overrides it, as a clip does with its time and the state
machine with its current state and transition.

`input_slots` says which inputs the node takes, in the order `evaluate`
receives them in `evaluated_inputs`: `InputSlots::None` for a node that reads
none, as a clip does; `InputSlots::Fixed` with a name for each, as a layer's
`base` and `overlay`; or `InputSlots::Variable` for any number, as a blend
takes. A node that keeps data for each input, as the blend space keeps a point,
also implements `input_added(index)` and `input_removed(index)`, which an
editor calls when it gives the node an input or takes one away.

The three input types say what the node reads in each walk of the graph:
`create_state` gets `CreateInput`, `update` gets `UpdateInput` and
`evaluate` gets `EvaluateInput`, already fetched, as their typed data (`&Res<Tuning>`
above). Each is a read-only system input: `Res`, `Option<Res>`, a read-only
`Query`, or a tuple of them; `()` reads nothing. The systems that walk graphs,
`initialize_animation_players`, `update_animation_players` and
`animate_targets`, each declare the union of their walk's inputs over every
registered node, and nothing else, so `animate_targets` never declares what only
an update reads. A node whose input is missing, or whose type was registered
without `register_animation_node`, is skipped in that walk with a warning. A
node skipped in the create walk has no state: it is never updated, and in
`evaluate` it gives its parent the rest pose at full weight, with one warning
until the player's graph is initialized again. The
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

Add a node with `add_input(Gain { amount: 2.0 })` on the result node or on
another node's context, or start a graph with `AnimationGraph::from_node`.
The context's methods mirror `EntityCommandQueue`'s: `add_input(node)` adds a
node with no inputs and returns the same context, so siblings chain;
`add_input_with(node, |node| { .. })` hands the new node's context to a
closure, for a node with several inputs; and `input(node)` returns the new
node's context, so a line of single inputs reads flat:

```rust
graph
    .result_node()
    .input(FootLockNode::new())
    .input(Gain { amount: 2.0 })
    .add_input(AnimationClipNode::new(walk));
```

A tool that edits a graph reads it and puts it together without the builder.
`node_indices()` lists every node, `node(index)` gives its `AnimationNodeData`,
`result_index()` names the result node, and `inputs(index)` gives a node's
inputs in slot order, the order they were added and the order `evaluate`
receives them in. `insert_node(data)` adds a node with no parent, from
`AnimationNodeData::of(&node)` or `AnimationNodeData::from_live(boxed)`, and
`connect(parent, child)` appends `child` as `parent`'s next input. It answers
a `ConnectError` and changes nothing when `child` already has a parent, is the
result node, would close a cycle, or either index is not a node of the graph.
`node_mut(index)` and `AnimationNodeData::live_mut()` reach a live node to edit
it through `as_reflect_mut`.

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

### Reading node state

Gizmos, overlays and tools read a node's state from the player:

```rust
fn log_gains(players: Query<&AnimationPlayer>, graphs: Res<AssetStore<AnimationGraph>>) {
    for player in players.iter() {
        player.for_each_state::<Gain>(&graphs, |gain, _state, _blackboard| {
            println!("gain {}", gain.amount);
        });
    }
}
```

`for_each_state::<N>` calls the closure with each live node of type `N` in the
player's graph, nodes inside state machines and retarget nodes included, its
state and the player's blackboard. It takes the player by shared reference, so a
system that only draws takes `Query<&AnimationPlayer>` and does not mark the
player changed. `for_each_state_mut` hands out the state mutably for tests that
set a node up; it exists only under `cfg(test)` or the crate's `test-support`
feature, which a dependent crate enables in its dev-dependencies. A player whose
graph is not loaded or not yet initialized visits nothing. Order the system after
`animate_targets` to read what the last evaluation left in the state.

`for_each_node` visits every playing node whatever its type, in node order,
with the nodes of the graphs a node plays inside itself right after it. Each
`NodeVisit` holds the node's `weight`, its `report` (`clip_time` for a clip
node; for a state machine, `state_machine` with the `current` state and the
`transition` in flight as `from`, `to` and a `progress` from 0 to 1, which
under the inertialization blender is the share of the offset decayed so far)
and its `path` from the player's graph: one `NodePathStep` per graph on the
way, each the `node` index in that graph and, when the next step is inside a
state machine's state, that `state`. The last step is the node itself. An
editor shows a playing graph from it; `AnimationGraphState::for_each_node` is
the same walk from a graph state.

```rust
player.for_each_node(&graphs, |visit| {
    if let Some(machine) = &visit.report.state_machine {
        log::info!("{:?} is in {:?}", visit.path, machine.current);
    }
});
```

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
- `reference`, the pose the rig is aligned in, and `corrections` on top of it;
- `goals`, named IK goals on bones such as the feet and hands.

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

A `RetargetGoal` is a `name` and a `bone`, the tip of a two-bone limb: the
goal bends the bone's parent, the mid, and the mid's parent, the root, so a
goal on the ankle bends the knee and the thigh. Goals, like chains, are
matched across rigs by name alone, and these names are recommended:

| Goal | Bone |
|---|---|
| `foot_l`, `foot_r` | foot / ankle |
| `hand_l`, `hand_r` | hand / wrist |

A goal never bends the pelvis: its mid and root may not be the rig's root or
motion root, or above either.

The reference pose is `ReferencePose::Rest`, the skeleton's rest pose, or
`ReferencePose::ClipFrame { clip, time }`, the rest pose with every channel of
the clip sampled at `time`; a bone the clip has no channels for keeps its rest
pose, and a one-key channel writes nothing, as in the player. Each
`BoneCorrection` then turns its `bone` by `rotation` in the bone's own space,
after the pose's rotation: the local rotation becomes `rotation * correction`.
Two corrections on one bone both apply, in list order. There is no absolute
per-bone override.

A rig is made from the content browser's New menu and edited in the editor, or
in code. With `concerto-animation-editor` installed it opens in the rig tab: a
bone tree to assign chains and roots, a preview of a second rig's character
beside it, and the inspector, whose bone fields pick from the named skeleton's
bones (see that crate's README). Without it, the rig opens in the generic
inspector:

```rust
use concerto::animation::retarget::{BoneCorrection, RetargetChain, RetargetGoal, RetargetRig};

let rig = RetargetRig::new()
    .with_skeleton(skeleton)
    .with_root("pelvis")
    .with_motion_root("root")
    .with_chain(RetargetChain::new("spine", "spine_01", "spine_03"))
    .with_chain(RetargetChain::new("head", "Head", "Head"))
    .with_correction(BoneCorrection::new("upperarm_l", Quat::from_rotation_z(0.5)))
    .with_goal(RetargetGoal::new("foot_l", "foot_l"));
```

`rig.resolve(&skeleton, clip)` turns it into a `ResolvedRig`: the bone indices
of the root, the motion root and each chain (`chain(name)` finds one), the
tip, mid and root indices of each goal, in list order (`goal(name)` finds
one), the reference pose in local and model space, and `root_height`, the root's height
above the model origin with model +Y up. It takes the skeleton and, for a
`ClipFrame`, the clip. A rig loaded through the `AssetServer` loads both through
its own handles when it loads, so they arrive without the caller asking. It
fails with a `RigError` naming the bone or chain: an invalid skeleton, an empty
or unknown bone, a motion root that is not above the root, an unnamed or
duplicate chain, a chain end not below its start, an unnamed or duplicate goal
(`UnnamedGoal`, `DuplicateGoal`), a goal bone with no parent and grandparent
(`GoalTooHigh`), a goal that would bend the root or the motion root
(`GoalBendsRoot`), a missing clip, a time that is not finite, a clip with no channels for the skeleton, or a root height that
is not finite or not above 0.1 mm.

The solve that plays one rig's motion on another is `PoseRetarget`, below, and
`RetargetNode` runs it in a graph every frame. Clips derived from it are coming
(#589).

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
root motion still in it, as a clip holds it. A graph pose, net of root motion,
is not one, with one exception: the root is measured from the source rig's
reference, so a pose netted against the source reference's root joint gives a
target pose netted against the target reference, with no ground offset. That
is the pose `RetargetNode`, below, hands the solve.

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

The target keeps the rest from its reference pose. Before goals are solved, a
bone in no shared chain keeps its reference local rotation, a chain only the target has keeps its
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

After this FK pass, each goal both rigs name moves the target limb with the
two-bone IK of `concerto::animation::ik`. The pairs run in the target rig's
goal order, and a goal on one rig only is not solved. A pair's full goal is the
target tip's reference position plus the source tip's movement from its
reference, relative to the source root and scaled by `scale()`, measured from
the target root. The part that comes from the reference poses, the target
tip's reference offset from its root less the scaled source one, turns with
the root's turn about +Y, so the spacing of the feet follows the character's
facing and the result is the same whether the pose still holds its root motion
or not. A source ankle at its reference height therefore puts the
target ankle at the target's own reference height, a planted source foot gives
a fixed target goal, and a stride scales as root motion does. The goal is the
FK tip position moved toward the full goal by the pair's `weight`, and
`softness` eases the reach near full extension. The solve turns the limb's root
and mid; the tip keeps the model rotation the FK pass gave it, and the bones
below it, such as toes and fingers, follow. The knee or elbow stays on the side
the FK pass bent it to, a goal out of reach straightens the limb toward it, and
no bone changes length. Each pair sees the pose the pairs before it left.

Every pair starts at weight 1 and softness 0. `set_goal(&settings)` replaces
one pair's, from a `RetargetGoalSettings { goal, weight, softness }` built with
`RetargetGoalSettings::new(goal)`, `with_weight` and `with_softness`. The
weight is clamped to [0, 1], and a weight of 0, or NaN, leaves the FK result
exactly. A name the two rigs do not both
have fails with `RetargetError::UnknownGoal { goal }` and changes nothing.

`new` fails with a `RetargetError`: `RigNotForSkeleton { side }` when a rig's
pose lengths, bone indices, goal bones, motion root or root height do not fit
the skeleton beside it, or the skeleton is invalid, and `NoSharedChain` when the rigs share
no chain name. `retarget_into` fails with `PoseLength { side, .. }` when the
source pose or `out` has the wrong number of bones, and then writes nothing.

Bone scale is assumed uniform: rotations are composed without it, so under a
non-uniform scale a chain bone's turn and the root's position are solved in
slightly different frames. A source pose with non-finite values gives
non-finite values in `out`.

### The retarget node

`RetargetNode` (`concerto::animation::retarget`) plays motion made for one
skeleton on another, every frame. It owns an inner graph, evaluated on the
source rig's skeleton, and retargets each frame of it onto the player's
skeleton with a `PoseRetarget`. `RetargetNode::new(graph, source, target)`
takes the inner graph and the two rigs; the target rig's skeleton must be the
player's. The node is a leaf, with no inputs, and anything can sit above it:

```rust
use concerto::animation::retarget::RetargetNode;

let inner = assets.add(AnimationGraph::from_node(AnimationClipNode::new(walk)));
let lafan_rig = assets.add(lafan_rig);
let ual_rig = assets.add(ual_rig);

let mut graph = AnimationGraph::new();
graph
    .result_node()
    .add_input(RetargetNode::new(inner, lafan_rig, ual_rig));
cmd.insert(AnimationHandleComponent::new(assets.add(graph)), entity);
```

The inner graph shares the player's blackboard, so its parameters are the
player's, and the player's `AnimationRootBone` reaches it as the source
skeleton's root, netted against the source rig's reference root joint (see
[Retargeting a pose](#retargeting-a-pose)). The output pose is then net of
root motion against the target reference, and the frame's root motion is the
inner graph's, with its translation multiplied by `scale()` and its rotation
unchanged. Clip events fired in the inner graph reach the player as its own,
and the node is finished when the inner graph is, so a state machine's
`OnAnimationEnd` follows it. `AnimationPlayer::for_each_state` also visits
the nodes inside the inner graph, which is how motion matching's gizmos find a
matcher under a retarget node.

Every goal both rigs name is solved at weight 1 and softness 0 unless the
node's `goals` say otherwise. Each `RetargetGoalSettings` in that list,
added with `with_goal`, sets one goal's weight and softness, and a later entry
for the same goal wins. The list is set in code, as the editor does not edit a
graph's nodes:

```rust
use concerto::animation::retarget::{RetargetGoalSettings, RetargetNode};

let node = RetargetNode::new(inner, lafan_rig, ual_rig)
    .with_goal(RetargetGoalSettings::new("foot_l").with_softness(0.1))
    .with_goal(RetargetGoalSettings::new("foot_r").with_weight(0.0));
```

An entry naming a goal the two rigs do not both have logs one warning and is
ignored; the node still plays. The node reads its settings and both rigs'
goals when it starts, so an edit to a rig's goals reaches a node that is
already playing only once the node restarts.

Until the player's skeleton, both rigs, their skeletons, a `ClipFrame`
reference clip and the inner graph have loaded, the node gives the rest pose,
and its `is_finished` is false. The inner graph also sees the player's
`model_to_world`, but at the source's scale, so nodes that read the world, such
as foot placement, IK and look-at, go above the retarget node, on the target
skeleton, not inside its inner graph.

The node refuses:

- a rig that does not resolve, or a target rig whose skeleton is not the
  player's, and gives the player's rest pose;
- a retarget node in its inner graph, directly or in a loaded state of a
  state machine there, a solve that `PoseRetarget::new` refuses, or an inner node,
  in the inner graph or in a loaded state of a state machine there, whose
  `CreateInput` is not also an update input, which would get no state inside
  the node's update, and gives the target rig's reference pose.

Each refusal logs one warning naming it, and the node then counts as
finished. It stays refused until the player's graph changes: fixing or
reloading the inner graph or a rig does not bring it back. A player skeleton
whose bones change while the node plays fails it the same way, at the rest
pose. A pose the solve rejects while playing gives the rest pose with no root
motion, still passes the events, and warns once.

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
  left stick), `Look` (the right stick) and `Stroll` (left Shift). `GameplayPlugin` binds them through
  `bind_gameplay_actions`, which a test calls on its own `ActionMap`.
- `gameplay::first_person` is a first-person character that walks, with no
  jump. `spawn_first_person_walker(&mut cmd, player, transform, walker)` spawns
  a `FirstPersonWalker` with its body, feet at `transform`, and returns the
  entity; `player` is the `PlayerIndex` whose `Move` and `Look` drive it, so two
  walkers with two indices are two players. The left stick walks up to
  `walk_speed` (1.6 m/s) at `hurry_threshold` (0.7 of full deflection) and
  rises to `hurry_speed` (3.4 m/s) at full; keys always push fully, so holding
  `Stroll`, left Shift, keeps a push at the walk. The right
  stick turns the view at `look_rate` radians per second, and the mouse turns
  it by `mouse_sensitivity` for the player who holds the mouse; `invert_y`
  flips the pitch. Sensitivity and inversion are fields of the walker, so they
  are per player; a settings screen writes them. The body is a dynamic capsule
  that never rotates, held `step_height` (0.35 m) above the ground by a sphere
  cast under it each fixed step: it walks up steps that high, follows ground no
  steeper than `max_slope_angle`, falls when there is none, and is stopped by
  walls, closed doors and anything else with a collider. Code that isn't a
  player drives a walker with `steer(push)` and `look(yaw, pitch)`.
  Setting `controlled` to false makes it ignore its player, standing still
  with its view kept, for a menu or a conversation.
  `FirstPersonView { walker }` on a root entity puts it at the walker's
  `eye_height`, `eye_forward` ahead of its centre (0 by default; a view moved forward
  can reach through a wall the walker stands against, so to hide a body drawn
  on the walker from its own view, use `RenderLayers` instead), looking where it
  looks; give that entity a `Camera` with a
  `viewport` for split screen, or a `VirtualCamera`.
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

`NavigationPlugin`, in `DefaultPlugins`, walks characters to points you place
by hand, over a navmesh baked from the level's colliders, or over a graph of
those points linked by hand when no navmesh is loaded. `concerto::navigation`
holds it. It registers the `NavMesh` asset, so it needs an `AssetServer`;
`DefaultPlugins` provides one.

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

A `NavPath` holds its `destination`, the point it ends at, the `positions` it
passes and its `length` in metres. `path.travel_time(speed)` is the time to
walk it at that speed, or `None` for a speed that is not finite and above zero.

Nearness and arrival are measured on the horizontal plane, X and Z: a point a
step up or down a stair is as near as one on the floor. Path lengths count the
height between points.

### The navmesh

A `NavMesh` asset is the walkable floor of a level, as polygons. It lists the
`scenes` it is baked from and its `settings`, a `NavMeshSettings`:

| Setting | Default | Meaning |
|---|---|---|
| `agent_radius` | 0.3 m | how far the mesh keeps from walls and edges |
| `agent_height` | 1.8 m | the headroom a place needs to be walkable |
| `max_slope` | 45° | the steepest walkable slope |
| `step_height` | 0.35 m | the tallest ledge an agent steps up |
| `cell_size` | 0.1 m | the horizontal resolution of the bake |
| `cell_height` | 0.05 m | the vertical resolution of the bake |

The mesh is derived data: its file holds the scene list and the settings, and
the polygons are baked from the box and mesh colliders of those scenes, sensor
boxes left out, when the asset loads, cached, and cooked into the file by
`--cook` (see [Derived data](../asset-importing.md#derived-data)). A game that
builds its geometry in code fills a `NavMeshGeometry`, calls `bake`, and adds
`NavMesh::baked(settings, data)` with `AssetServer::add`.

A `NavMeshComponent { handle }` on an entity of the level, a scene component,
names the mesh agents route on. Once that mesh is loaded, a `NavMeshStatus`
on the same entity reports its polygon count and bounds, which the editor's
inspector and MCP `inspect` show. Agents use the mesh of the entity with the
lowest index whose mesh is loaded and has polygons; with more than one
`NavMeshComponent`, one warning says so. An end of a route up to half a metre
off the mesh, such as a point standing against a wall, is moved onto it.

While a mesh is loaded, agents route on it and the graph's links are ignored:
the graph still names the destinations, and a disabled point is still never
reached. With no mesh loaded, agents walk the graph. While a
`NavMeshComponent`'s mesh is still on its way and no mesh is loaded, an agent
that has no route yet, or is given a new destination, stops and waits with its
destination kept, and plans once the mesh lands. A route already being walked
to an unchanged destination is kept as it is until then, even when the graph is
rebuilt or the mesh it was planned on goes away, and is planned again on the
mesh once it lands. A mesh whose load failed, or that was never loaded, leaves
agents on the graph, as does a loaded mesh with no polygons.

### Agents

A `NavAgent` walks the graph. `NavAgent::new()` walks at 1.4 m/s, counts a
waypoint reached within 0.25 m and slows down within 1.5 m of the last one;
`with_speed`, `with_arrival_radius` and `with_slow_radius` change those.

Insert a `NavDestination` to send it somewhere: `NavDestination::to_point(entity)`
or `NavDestination::to_anchor(name)`, at the agent's speed, or at another with
`.with_speed(speed)` for that request alone. On a navmesh, the agent plans from
where it stands, through the mesh's corners, to the point. On the graph, it
plans from the enabled point nearest it. It plans again when the destination
changes, when the graph is rebuilt under its route, as when its point moves or
is disabled, and when the mesh that is loaded is not the one its route was planned on: a mesh
landing, being swapped, or its component removed. `NavRoute::planned_on()`
names that mesh, or `None` for the graph. Remove the destination, or the `NavAgent`, to stop
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
NPC that patrols a square of four points this way, routing around a box on each
side of the square on a navmesh the level bakes at startup with `bake` and adds
with `NavMesh::baked`.

### Drawing the graph

The `concerto-navigation-gizmos` crate, which a game adds itself, draws each
point as a ring with a line along its facing, green when enabled and grey when
not, each link as a line with an arrowhead on a one-way link, and the route
each agent has left, and the polygon edges of each loaded navmesh in amber.
Register `NavigationGizmosPlugin` after `DefaultPlugins`; the
`NavigationGizmos` resource turns the graph, routes and mesh layers off. The
graph is drawn in the editor's scene view as well. The navmesh is drawn there
by `NavMeshGizmo` once the game lists `concerto-navigation-editor` among its
editor plugins, while its entity is selected or with Navmesh set to Always in
the viewport's Gizmos menu.
Its [README](../../crates/navigation-gizmos/README.md) says more.

### Limits

- Agents do not avoid each other or the player. Their capsules collide in
  physics, and that is all.
- The navmesh is baked offline and does not change while the game runs. A
  physics body that moves onto a route is not walked around, and a blocked
  agent does not plan again.
- On the graph, the nearest point an agent starts from is found by distance,
  without a check that it can be seen or reached in a straight line.
- Links are walked, nothing else: no doors, ladders or vaults. A navmesh has
  no links at all yet; off-mesh connections are tracked in #699.

Next: [User interface](08-ui.md).

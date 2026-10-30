# concerto-motion-matching

A character simulation that drives a physics body and predicts where it is
going. It replaces `CharacterMovement` from `concerto-physics`.

`MotionMatchingPlugin` is part of `DefaultPlugins`; the root crate re-exports
the crate as `concerto::motion_matching`.

## Components

- `DesiredMotion`: a world velocity in m/s and a world facing. Other systems
  write it. It holds no gait or strafe. Default: zero velocity, identity.
- `CharacterSimulation`: requires `DesiredMotion` and a physics body.

| Field | Meaning | Default |
|---|---|---|
| `state` | position, velocity and acceleration of the velocity spring | zero |
| `rotation`, `angular_velocity` | facing and its scaled angle-axis rate | identity, zero |
| `velocity_halflife` | spring halflife for velocity | 0.27 s |
| `rotation_halflife` | spring halflife for facing | 0.27 s |
| `sample_dt`, `sample_count` | trajectory sampling | 1/3 s, 3 |
| `trajectory` | predicted `TrajectorySample`s, `sample_dt * (i + 1)` ahead | empty |

`sample_dt` and `sample_count` must match the motion database that queries the
trajectory.

## Motion database

`MotionDatabaseDefinition` says what to bake; `bake` turns it into a
`MotionDatabase`. Both are assets.

### Definition

| Field | Meaning | Default |
|---|---|---|
| `skeleton` | the skeleton the clips animate | none |
| `entries` | `MotionEntry`s: a clip and an optional `TimeRange` in seconds; no range means the whole clip | empty |
| `rate` | frames per second of the database | 60 |
| `speed` | clip seconds consumed per second of database time | 1.1 |
| `simulation` | `SimulationBoneSettings` | see below |
| `contacts` | `ContactSettings` | see below |
| `channels` | `StoredChannel`s: a registered type name and a reflected value, in feature order | empty |

An entry bakes `floor((end - start) * rate / speed)` frames.

`SimulationBoneSettings`: `position_joint` (`"Spine2"`) and `facing_joint`
(`"Hips"`) name skeleton joints; `facing_axis` (`Vec3::Z`) is the facing
joint's forward in its own space; `position_window` (31) and `facing_window`
(61) are Savitzky-Golay windows. The simulation bone is the position joint's
ground projection smoothed, with the smoothed yaw of the facing joint.

`ContactSettings`: `bones` names skeleton joints (empty); a bone touches the
ground in a frame when its model-space speed is below `speed_threshold`
(0.15 m/s), after a median filter of `median_window` (6) frames.

### Bake

```rust
let database = bake(&definition, &skeleton, &clips, &registry)?;
```

`clips` holds one resolved clip per entry, in order; `registry` is the
`TypeRegistry` that knows the channel types. Each entry is baked on its own, so
no frame blends across two entries. Root bones are re-expressed relative to the
simulation bone. `bake` returns `BakeError`:

| Error | Cause |
|---|---|
| `MissingJoint` | a simulation or contact joint name the skeleton lacks |
| `ClipHasNoBones` | the clip animates none of the skeleton's bones |
| `ClipCountMismatch` | `clips.len()` differs from `entries.len()` |
| `UnregisteredChannel` | a channel type name not in the registry, or without `ReflectFeatureChannel` |
| `ChannelValue` | the stored value does not apply to the channel |
| `RangeTooShort` | an entry bakes fewer than 2 frames |
| `InvalidRange` | a range outside its clip, or with `start >= end` |
| `InvalidWindow` | a window that is even or at most 3 |
| `InvalidRate`, `InvalidSpeed` | not finite and positive |

### Feature channels

A `FeatureChannel` writes `dims()` values per frame from a `FrameContext`
(skeleton, local and model pose, model velocities, all indexed by skeleton joint, the simulation bone, the
simulation bones of the whole range and the frame's index in it). The channels
of a definition are concatenated in order into one feature vector per frame.

```rust
#[derive(Reflect, Default)]
#[reflect(FeatureChannel)]
struct RootSpeed {
    scale: f32,
}

impl FeatureChannel for RootSpeed {
    fn dims(&self) -> usize { 1 }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.model_velocities[0].length() * self.scale;
    }
}

app.register_type::<RootSpeed>();
```

A channel needs `Reflect` and `Default`. The registered type name is what a
`StoredChannel` refers to; a stored value is applied over `Default`.

### Layout

`MotionDatabase` is dense and frame-major. Bone 0 is the simulation bone; bone
`i + 1` is skeleton joint `i`, and `names` and `parents` follow that numbering
(a parentless joint has parent 0).

- `positions`, `rotations`, `velocities`, `angular_velocities`: `bone_count`
  entries per frame, local to the parent; root joints are relative to the
  simulation bone. `pose(frame)` slices them.
- `contact_bones`: database bone indices (skeleton joint index + 1), not
  skeleton joint indices. `contacts` holds one `bool` per contact bone per
  frame, in that order; `contacts(frame)` slices it.
- `range_starts`, `range_stops`: the frames of each entry, half-open;
  `range_of(frame)` finds the entry.
- `features`, `feature_dims`: `feature_dims` values per frame;
  `features(frame)` slices them. `channels` lists each `ChannelSpan`
  (`type_name`, `offset`, `dims`).

## Systems

- `drive_body`, in `FixedUpdate`, before `step_simulation`: advances the spring
  one fixed step toward `DesiredMotion::velocity`, with zero vertical velocity,
  advances the facing, and sets the body's horizontal velocity to the
  spring's. The body keeps its own vertical velocity.
- `read_back_body`, in `LateFixedUpdate`, after `step_simulation`: sets the
  simulation's horizontal position to the body's after collisions.
- `predict_trajectory_system`, in `LateFixedUpdate`, after `read_back_body`:
  fills `trajectory`.

An entity without a `BodyId` is skipped.

`predict_trajectory` is the pure function behind the last system. It holds the
current `DesiredMotion` constant: there is no input extrapolation and no
collision, so the prediction does not see walls.

## Example

```rust
commands.spawn((
    Player,
    CharacterSimulation::default(),
    DesiredMotion::default(),
));

fn walk(mut players: Query<&mut DesiredMotion, With<Player>>) {
    for mut desired in players.iter() {
        desired.velocity = Vec3::new(0.0, 0.0, -5.0);
    }
}
```

## Not here

Capsule traces for collision-aware prediction, past samples, gamepad input,
and synchronising the skeleton and the capsule with spring dampers are
separate work. The simulation does not rotate the capsule.

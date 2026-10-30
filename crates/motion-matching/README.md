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
`MotionDatabase`. Both are assets. The database stores no poses: it stores
features, the simulation bone and, per frame, which clip and time the frame
plays. `MotionDatabase::pose` samples the pose from the clip.

### Definition

| Field | Meaning | Default |
|---|---|---|
| `skeleton` | the skeleton the clips animate | none |
| `entries` | `MotionEntry`s: a clip and an optional `TimeRange` in seconds; no range means the whole clip | empty |
| `rate` | frames per second of the database | 60 |
| `speed` | clip seconds consumed per second of database time | 1.1 |
| `simulation` | `SimulationBoneSettings` | see below |
| `trajectory` | `TrajectorySettings`: `sample_dt` (1/3 s) and `sample_count` (3) | 1/3 s, 3 |
| `channels` | `StoredChannel`s: a registered type name and a reflected value, in feature order | `reference_channels()` |

An entry bakes `floor((end - start) * rate / speed)` frames.

`SimulationBoneSettings`: `position_joint` (`"Spine2"`) and `facing_joint`
(`"Hips"`) name skeleton joints; `facing_axis` (`Vec3::Z`) is the facing
joint's forward in its own space; `position_window` (31) and `facing_window`
(61) are Savitzky-Golay windows. The simulation bone is the position joint's
ground projection smoothed, with the smoothed yaw of the facing joint. A
window longer than an entry's range is clamped to the largest odd size that
fits the range; a range too short for a window above 3 is left unsmoothed.

`TrajectorySettings` defaults match `CharacterSimulation`'s `sample_dt` and
`sample_count`. Offline, sample `k` sits `round(sample_dt * (k + 1) * rate)`
frames ahead: offsets are rounded to whole database frames. A sample past the
end of an entry's range is clamped to the range's last frame.

### Bake

```rust
let database = bake(&definition, &skeleton, &clips, &registry)?;
```

`clips` holds one resolved clip per entry, in order; `registry` is the
`TypeRegistry` that knows the channel types. Each entry is baked on its own, so
no frame blends across two entries. `bake` returns `BakeError`:

| Error | Cause |
|---|---|
| `MissingJoint` | a simulation joint name the skeleton lacks |
| `ClipHasNoBones` | the clip animates none of the skeleton's bones |
| `ClipCountMismatch` | `clips.len()` differs from `entries.len()`, or from the database's clip count in `pose` |
| `BoneCountMismatch` | `pose` was given a skeleton whose bone count differs from the baked one |
| `FrameOutOfRange` | `pose` was given a frame the database did not bake |
| `UnregisteredChannel` | a channel type name not in the registry, or without `ReflectFeatureChannel` |
| `ChannelValue` | the stored value does not apply to the channel, or the channel's `validate` failed |
| `InvalidWeight` | a channel weight, or one given to `set_weights`, that is not finite and positive |
| `WeightCountMismatch` | `set_weights` was given a different number of weights than the database has channels |
| `ChannelCountMismatch` | a `FeatureQuery` was used with a database of a different channel count |
| `TrajectoryTooShort` | `FeatureQuery::build` was given fewer trajectory samples than the database's `sample_count` |
| `RangeTooShort` | an entry bakes fewer than 2 frames |
| `InvalidRange` | a range outside its clip, or with `start >= end` |
| `InvalidWindow` | a window that is even or at most 3 |
| `InvalidRate`, `InvalidSpeed`, `InvalidSampleDt` | `rate`, `speed` or `trajectory.sample_dt` not finite and positive |

### Feature channels

A `FeatureChannel` writes `dims()` values per frame from a `FrameContext`. The
channels of a definition are concatenated in order into one feature vector per
frame. `extract_features(&channels, &frame, &mut out)` runs them; `out` must
hold exactly the sum of their `dims()`. `build_channels` turns a definition's
`StoredChannel`s into live channels.

`FrameContext` fields:

| Field | Meaning |
|---|---|
| `skeleton` | the skeleton |
| `local_pose` | local `JointPose`s by skeleton joint; parentless joints are re-expressed relative to `simulation` |
| `model_pose` | model-space `Transform`s by skeleton joint |
| `model_velocities` | model-space linear velocity by skeleton joint |
| `simulation` | the simulation bone as a `Transform` |
| `body` | `BodyMotion`: the simulation bone's `velocity` and `angular_velocity` |
| `trajectory` | `TrajectorySample`s (position, velocity, rotation) ahead of the frame, future only |

`simulation`, `body` and `trajectory` share one space, and trajectory
positions are absolute in it, not relative to the frame. A channel that wants
offsets subtracts `simulation.translation`, or converts with the inverse of
`simulation`.

The context is filled in two ways:

- Offline, by `bake`, per database frame: the space is the clips' model space.
  `simulation` is the smoothed simulation bone, `body` its central-difference
  velocities per database second (one-sided at the ends of a range) and
  `trajectory` the `TrajectorySettings` samples above.
- Online, by the runtime query: the space is the world. `simulation` and `body`
  come from `CharacterSimulation` and `trajectory` is its `trajectory`; pose
  and velocities come from the animation system.

Because both fills go through the same `extract`, a channel written in terms of
the inverse of `simulation` yields the same features in both.

```rust
#[derive(Reflect, Default)]
#[reflect(FeatureChannel)]
struct RootSpeed {
    scale: f32,
}

impl FeatureChannel for RootSpeed {
    fn dims(&self) -> usize { 1 }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.body.velocity.length() * self.scale;
    }
}

app.register_type::<RootSpeed>();
```

A channel needs `Reflect` and `Default`. The registered type name is what a
`StoredChannel` refers to; a stored value is applied over `Default`.

A channel may also override three methods:

- `weight()` (default 1.0): how much its group counts in the search. It must be
  finite and positive, or `bake` returns `InvalidWeight`.
- `source()` (default `ChannelSource::Pose`): a `Trajectory` channel reads only
  `simulation` and `trajectory` from its `FrameContext`, so the runtime query
  can compute it from a predicted trajectory.
- `validate(&skeleton, &trajectory_settings)` (default `Ok`): `bake` calls it
  before extracting; an `Err` becomes `ChannelValue`.

### Reference channels

`reference_channels()` is the default `channels` of a definition: 27 floats,
all in the simulation bone's frame, after `database_build_matching_features`
in orangeduck/Motion-Matching.

| Channel | Floats | Weight |
|---|---|---|
| `BonePositions { bones: ["LeftFoot", "RightFoot"] }` | 6 | 0.75 |
| `BoneVelocities { bones: ["LeftFoot", "RightFoot"] }` | 6 | 1.0 |
| `BoneVelocities { bones: ["Hips"] }` | 3 | 1.0 |
| `TrajectoryPositions { samples: [0, 1, 2] }` | 6 (x, z) | 1.0 |
| `TrajectoryDirections { samples: [0, 1, 2] }` | 6 (x, z) | 1.5 |

`samples` index the trajectory; 0 is one `sample_dt` ahead. With the default
`TrajectorySettings` at 60 Hz they sit at +20, +40 and +60 frames. A missing
bone or an index at or past `sample_count` makes `bake` return `ChannelValue`.

### Normalisation

Each channel is a group. The bake stores the mean of every feature dimension
(`offset`) and, per group, the mean over its dimensions of the standard
deviation. A group's scale is `max(deviation, 1e-6) / weight`, and
`normalised = (x - offset) / scale`, so the group's deviation after
normalising equals its weight and the group keeps its geometry. A group whose
deviation is below `1e-6` normalises to 0. `features(frame)` stays raw.

- `offset()`, `scales()`: per feature dimension.
- `normalise(&mut features)`, `normalised_features(frame)`.
- `set_weights(&[f32])`: one weight per channel; changes the scales only, with
  no re-bake. It does not rewrite the weight inside each channel's stored
  value; the span's weight is the one in use.

### FeatureQuery

```rust
let query = FeatureQuery::new(&database, &registry)?;
let vector = query.build(&database, &skeleton, pose_frame, &simulation, &trajectory)?;
```

`build` returns the normalised query vector: `Pose` channels are copied from
database frame `pose_frame`, as the reference does, and `Trajectory` channels
are extracted from `trajectory` expressed in `simulation` (the character's
world transform). It returns `FrameOutOfRange`, `BoneCountMismatch`,
`ChannelCountMismatch` or `TrajectoryTooShort` instead of panicking.

### Layout

`MotionDatabase` fields are private; read it through accessors. Frames are
dense and frame-major. Nothing is indexed by a database-only bone numbering:
every bone index is a skeleton joint index.

- `skeleton()`, `bone_count()`, `clips()`: the skeleton handle, its bone count
  at bake time and one clip handle per entry, in entry order.
- `rate()`, `speed()`, `trajectory()`: the sampling the bake used.
- `frame_count()`; `entry_of(frame)`, `range_of(frame)`: the entry holding a
  frame and its half-open frame range.
- `clip_time(frame)`: the entry and the clip time in seconds the frame plays,
  `clip_start + (frame - range_start) * speed / rate`.
- `simulation(frame)`: the simulation bone, a `Transform` in model space.
- `features(frame)`, `feature_dims()`, `channels()`: `feature_dims()` values
  per frame; `channels()` lists each `ChannelSpan` (`type_name`, `offset`,
  `dims`, `weight`, `source`, `value`).

`simulation(frame)` and `features(frame)` panic on a frame outside
`0..frame_count()`. `entry_of`, `range_of` and `clip_time` return `None` for
it, and `pose` returns `Err(FrameOutOfRange)`.

`pose(frame, &skeleton, &clips)` samples the frame's local pose from
`clips[entry]` at `clip_time` and re-expresses each parentless joint relative
to the simulation bone. `clips` is one clip per entry, as given to `bake`. It
returns `Err(BoneCountMismatch)`, `Err(ClipCountMismatch)` or
`Err(FrameOutOfRange)` instead of panicking.

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

## Playing a database

`MotionMatchingNode` is an animation graph node that plays a `MotionDatabase`.
`MotionMatchingPlugin` registers it and its systems; the plugin is in
`DefaultPlugins`, so a game needs nothing more than a graph that holds the
node.

| Field | Meaning | Default |
|---|---|---|
| `database` | the `MotionDatabase` to play | none |
| `search_period` | seconds between searches | 0.1 |
| `velocity_change_threshold`, `rotation_change_threshold` | the change per second of the first trajectory sample's velocity and facing that forces a search once it settles | 50 |
| `same_frame_tolerance` | a search result this many frames or fewer from the playhead is ignored | 1 |
| `blend_halflife` | inertialization halflife after a jump | 0.1 s |

Each fixed step `publish_trajectory` copies a character's
`CharacterSimulation` into its `AnimationPlayer`'s blackboard as a
`MotionMatchingTrajectory` under the key `"trajectory"`. A player whose
character has no simulation, or a preview `App`, has no trajectory; the node
then plays the database without searching.

Per frame `run_motion_matching` advances the playhead at the database `rate`,
searches when the timer runs out, when a settled change in the trajectory asks
for it, or when the playhead is in the last `ceil(sample_count * sample_dt *
rate)` frames of its range, and on a better frame jumps with inertialization.
The search is brute force: a squared distance over the normalised features of
every frame, with an early out. `run_motion_matching` runs after
`update_animation_players` and before `animate_targets`.

`MotionMatchingInstance::root_motion` is the simulation bone's velocity and
angular velocity at the playhead. Nothing reads it yet.

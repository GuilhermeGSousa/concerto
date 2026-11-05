# concerto-motion-matching

A character simulation that drives a physics body and predicts where it is
going. It replaces `CharacterMovement` from `concerto-physics`.

`MotionMatchingPlugin` is part of `DefaultPlugins`; the root crate re-exports
the crate as `concerto::motion_matching`.

## Components

- `DesiredMotion`: a world velocity in m/s and a world facing. Other systems
  write it. It holds no gait or strafe. Default: zero velocity, identity.
- `CharacterSimulation`: requires `DesiredMotion` and a physics body. Its fields
  are its tuning. The springs it steps and the trajectory it predicts are
  internal: no field or reflection reaches them. `interpolated_facing(alpha)` is
  the one read of that state: the facing `alpha` of the way from the last fixed
  step to the current one, the facing `apply_character_displacement` uses.

| Field | Meaning | Default |
|---|---|---|
| `velocity_halflife` | spring halflife for velocity | 0.27 s |
| `rotation_halflife` | spring halflife for facing | 0.27 s |
| `sample_dt`, `sample_count` | trajectory sampling | 1/3 s, 3 |
| `max_pull_speed` | how fast a pull can take the body; a spring already faster keeps its speed | 8 m/s |

A halflife here is the one in orangeduck/Motion-Matching: a spring at rest covers
about 40% of the distance in it, where
`Spring::critically_damped_with_halflife` covers 50%.

`sample_dt` and `sample_count` must match the motion database that queries the
trajectory.

`max_pull_speed` must keep one fixed step under the capsule's radius: bodies use
discrete collision, so a faster pull could carry the capsule through a wall. The
default is about 0.27 m a step, so a capsule thinner than that needs a lower one.
A pull never takes the body past that speed, or past the spring's own speed when
that is higher, so a fast spring needs the same care.

A game that wants to know how a character moves asks its body:
`PhysicsState::linear_velocity` and `body_transform` say where it went after
collisions, which the springs do not know.

## Motion database

`MotionDatabase` is one asset: the definition it persists, the fields below,
and a runtime part, `baked: Option<BakedDatabase>`, that is never saved. It is
a `CreatableAsset` whose `new_asset()` is its `Default`, which is unbaked.
The baked part stores no poses: it stores features, the simulation bone and,
per frame, which clip and time the frame plays. `MotionDatabase::pose` samples
the pose from the clip.

The baked part is derived data (see "Derived data" in
[docs/asset-importing.md](../../docs/asset-importing.md)). The plugin registers
the type with `register_derived_asset`, so a load through the `AssetServer`
bakes it from the skeleton and clips the definition names, or takes the blob
cached under the same key, before the handle reads as loaded:

- `derived_inputs()`: the skeleton and each entry's clip.
- `derived_key(&inputs)`: `weightless_hash` of the definition and the inputs'
  file bytes; see [Derived key](#derived-key).
- `derive(&inputs, progress)`: decodes the skeleton and clips, calls
  `bake_with_progress`, and encodes the `BakedDatabase` with bincode.
- `apply_derived(&blob)`: decodes it into `baked`, refusing a blob whose
  channel or entry count differs from the definition's.
- `DERIVED_VERSION` is 1.

`is_baked()` says whether the part is there. Code that bakes inline, such as a
test or an `AssetServer::add` caller, attaches it with
`database.with_baked(bake(&database, &skeleton, &clips)?)`; `add` derives
nothing.

A database loaded through the `AssetServer`, or added with
`AssetServer::add`, loads its skeleton and clips in `on_load` and holds them
by strong handles, as `StandardMaterial` does its textures. A game loads only
the database; it needs no handles of its own to the clips for it to play.

### Definition

| Field | Meaning | Default |
|---|---|---|
| `skeleton` | the skeleton the clips animate | none |
| `entries` | `MotionEntry`s: a clip, an optional `TimeRange` in seconds and `mirrored` (`false`); no range means the whole clip | empty |
| `rate` | frames per second of the database | 60 |
| `speed` | clip seconds consumed per second of database time | 1.1 |
| `simulation` | `SimulationBoneSettings` | see below |
| `trajectory` | `TrajectorySettings`: `sample_dt` (1/3 s) and `sample_count` (3) | 1/3 s, 3 |
| `channels` | `FeatureChannelData`s, in feature order: see [Loading channels](#loading-channels) | `reference_channels()` |
| `mirror_axis` | a `MirrorAxis`, `X` or `Z`: the horizontal model-space axis normal to the plane mirrored entries are mirrored across | `MirrorAxis::X` |

An entry bakes `floor((end - start) * rate / speed)` frames.

`SimulationBoneSettings`: `position_joint` (`"Spine2"`) and `facing_joint`
(`"Hips"`) are `JointName`s; `facing_axis` (`JointAxis::Y`) is the facing
joint's forward in its own space, one of `X`, `NegX`, `Y`, `NegY`, `Z` and
`NegZ`; `position_window` (31) and `facing_window`
(61) are Savitzky-Golay windows. The simulation bone is the position joint's
ground projection smoothed, with the smoothed yaw of the facing joint. A
window longer than an entry's range is clamped to the largest odd size that
fits the range; a range too short for a window above 3 is left unsmoothed.

`JointName`, from `concerto-mesh`, is a `String` newtype that reflects as a
leaf, with `Deref<Target = str>`, `From<&str>` and a JSON form. Stored, it is
the bytes of its text.

`TrajectorySettings` defaults match `CharacterSimulation`'s `sample_dt` and
`sample_count`. Offline, sample `k` sits `round(sample_dt * (k + 1) * rate)`
frames ahead: offsets are rounded to whole database frames. A sample past the
end of an entry's range is clamped to the range's last frame.

### Bake

```rust
let baked: BakedDatabase = bake(&database, &skeleton, &clips)?;
```

`bake` reads the persisted fields of a `MotionDatabase` and returns only the
computed part. `clips` holds one resolved clip per entry, in order. The
channels must be loaded. Each entry is baked on its own, so no frame blends across two
entries.

`bake_with_progress` takes one more argument, `on_entry(finished, total)`,
called after each entry with `finished` counting from 1. It is not called for a
definition that is refused before the first entry.

`bake` returns `BakeError`:

| Error | Cause |
|---|---|
| `MissingJoint` | a simulation joint name the skeleton lacks |
| `ClipHasNoBones` | the clip animates none of the skeleton's bones |
| `ClipCountMismatch` | `clips.len()` differs from `entries.len()`, in `bake` or `pose` |
| `BoneCountMismatch` | `pose` was given a skeleton whose bone count differs from the baked one |
| `FrameOutOfRange` | `pose` was given a frame the database did not bake |
| `UnregisteredChannel` | loading a channel whose type name is not in the registry, or lacks `ReflectFeatureChannel` |
| `UnloadedChannel` | a channel still `Stored`, read where a live one is needed |
| `ChannelValue` | the stored value does not apply to the channel, or the channel's `validate` failed |
| `InvalidWeight` | a channel weight, from `check_weights`, that is not finite and positive |
| `ChannelCountMismatch` | a `FeatureQuery` was used with a database of a different channel count |
| `TrajectoryTooShort` | `FeatureQuery::build` was given fewer trajectory samples than the database's `sample_count` |
| `RangeTooShort` | an entry bakes fewer than 2 frames |
| `InvalidRange` | a range outside its clip, or with `start >= end` |
| `InvalidWindow` | a window that is even or at most 3 |
| `InvalidRate`, `InvalidSpeed`, `InvalidSampleDt` | `rate`, `speed` or `trajectory.sample_dt` not finite and positive |
| `Mirror` | a `MirrorError`: the skeleton cannot be mirrored across `mirror_axis`, in `bake` or `pose` |
| `NotBaked` | `pose` or a `FeatureQuery` was given a database with no baked part, or one whose channels can't normalise |

### Mirroring

A `mirrored` entry plays its clip reflected across the plane through the
model-space origin normal to `mirror_axis`, so a clip added twice, once
mirrored, gives both sides of the motion. Nothing mirrored is stored: `bake`
and `pose` pass each sampled local pose through `PoseMirror`, from
`concerto_animation::mirror`, before model space, so the simulation bone, the
trajectory and every feature come from the mirrored motion. Scale is copied
from the twin, which is exact only for uniform scale.

A joint whose name contains `Left` and not `Right` takes its pose from the
joint named with every `Left` replaced by `Right`, and the other way round; any
other joint is its own twin. Matching is case-sensitive. `PoseMirror::new`
fails with `MissingTwin` when a twin is not in the skeleton, `TwinParents` when
a joint's parent's twin is not its twin's parent, and `Asymmetric` when the rest
pose is not symmetric: a bone with a parent whose rest translation is more than
`1e-3 + 1e-2 * |t|` from its twin's mirrored. `Asymmetric` names the worst
joint and its distance. Parentless bones are not checked.

`MirrorAxis` offers only the horizontal axes, `X` and `Z`, because the
simulation bone is a Y-up ground projection with a yaw. LAFAN1's BVH skeleton
needs `MirrorAxis::Z`. A mirrored entry's simulation bone faces the mirror of the
original's forward only when the facing joint's rest axis, its model-space rest
rotation times `facing_axis`, lies in the plane; the bake does not check this.
With no mirrored entry no mirror is built.

`MotionDatabase` is at version 3, and implements `Asset` by hand. Its former
name is `MotionDatabaseDefinition`: a `MotionDatabaseDefinition` version-2 file
loads through `migrate_former` as a `MotionDatabase`, since the layouts are the
same, and is written back under the new name when it is saved. A
`MotionDatabase` file at version 1 or 2 is an old bake and fails to load with
"a baked motion database is now derived from its definition; delete this
file". A `MotionDatabaseDefinition` version-1 file fails to load.

### Feature channels

A `FeatureChannel` writes `dims()` values per frame from a `FrameContext`. The
channels of a definition are concatenated in order into one feature vector per
frame. `extract_features(&channels, &frame, &mut out)` runs them; `out` must
hold exactly the sum of their `dims()`.

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
- Online, by the runtime query: the space is the world. `simulation` is the
  body's position with the simulation's facing and `trajectory` is the published
  path. `body`, the pose and the velocities are left empty, since only
  `Trajectory` channels run online and they do not read them.

Because both fills go through the same `extract`, a channel written in terms of
the inverse of `simulation` yields the same features in both.

```rust
#[derive(Reflect, Default, Clone)]
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

A channel needs `Reflect`, `Default` and `Clone`. The registered type name is
what a stored channel refers to; a stored value is applied over `Default`.

### Loading channels

`FeatureChannelData` is one channel as an asset holds it: `Stored { type_name,
value }`, as decoded, or `Live { type_name, channel }`, the boxed
`FeatureChannel`. Both encode as the type name and the reflected value, and
decoding always gives `Stored`. `FeatureChannelData::live(channel)` makes a live
one under the type's short name, which is how code builds a definition.

`load(&registry)` turns a `Stored` channel `Live`, returning
`UnregisteredChannel` or `ChannelValue` when it can't. A database's
`on_load` loads its channels with the asset server's type registry and logs a
warning for each that fails, leaving it `Stored`; outside the asset server,
call `load_channels(&registry)`. After that nothing needs the registry: `bake`,
`FeatureQuery::new`, `weights_of` and the normalisation read the live channels,
and a channel still `Stored` there is `UnloadedChannel`, or no normalisation.

A channel may also override four methods:

- `weight()` (default 1.0): how much its group counts in the search. It must be
  finite and positive, or the search finds nothing. `bake` does not read it,
  so a cache hit and a miss give the same database whatever the weight.
  `check_weights()` returns `InvalidWeight` for the first loaded channel whose
  weight is not; `on_load` logs it as a warning, and the editor shows it as the
  derive failing. A top-level field named `weight` means this and nothing
  else: see [Derived key](#derived-key).
- `source()` (default `ChannelSource::Pose`): a `Trajectory` channel reads only
  `simulation` and `trajectory` from its `FrameContext`, so the runtime query
  can compute it from a predicted trajectory.
- `validate(&skeleton, &trajectory_settings)` (default `Ok`): `bake` calls it
  before extracting; an `Err` becomes `ChannelValue`.
- `preview(&frame, features, &mut out)` (default nothing): adds the
  `FeatureShape`s that draw `features`. See [Previewing](#previewing).

### Previewing

`preview` says how a channel looks for a given feature vector, so the same code
can draw a baked frame or a runtime query beside the frame it selected.
`preview_features(&channels, &frame, &features)` gives each channel its span of a
raw vector and returns one list of shapes per channel, in order.

`FeatureShape` is a `Ring { center, radius }`, a `Line { start, end }` or an
`Arrow { start, end }`, relative to the simulation bone. A `Ring` lies flat in the
bone's ground plane. The caller decides where the shapes go and their colour, and
a channel gets no drawing handle, so this crate does not depend on the renderer.

- What is drawn comes from `features`, which are raw: a normalised vector is not
  in metres. `FeatureQuery::raw` and `LastQuery::features` hold raw values;
  `FeatureQuery::build` returns normalised ones.
- `frame` only places what the values hold no position for, such as where a bone
  or a trajectory sample is. A shape the context cannot place is left out.

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

| Channel | Preview |
|---|---|
| `BonePositions` | a `Ring` of radius 0.05 at each stored position |
| `BoneVelocities` | an `Arrow` from the bone's position in `model_pose` along the stored velocity scaled by 0.1 |
| `TrajectoryPositions` | `Line`s from the origin through the stored positions, with a `Ring` of radius 0.12 at each |
| `TrajectoryDirections` | an `Arrow` 0.3 long from the sample's position in `trajectory` along the stored direction |

### Normalisation

Each channel is a group. The bake stores the mean of every feature dimension
(`offset`) and, per group, the mean over its dimensions of the standard
deviation. A group's scale is `max(deviation, 1e-6) / weight`, and
`normalised = (x - offset) / scale`, so the group's deviation after
normalising equals its weight and the group keeps its geometry. A group whose
deviation is below `1e-6` normalises to 0. `features(frame)` stays raw.

The weight is read from the live channel in `channels` each time the scales
are computed, not stored in the bake, so changing a channel's weight takes
effect in the next search with no re-bake. `scales()`, `normalise` and the
search give `None` while a channel is not live, a weight is not finite and
positive, or the channel count differs from the bake's.

- `offset()`, `scales()`: per feature dimension.
- `normalise(&mut features)`, `normalised_features(frame)`.

### Derived key

```rust
let key = weightless_hash(&database, &skeleton_bytes, &clip_bytes);
```

`clip_bytes` holds the stored bytes of one clip per entry. `derived_key` passes
the whole files of the skeleton and clips, read from the content root. The hash is FNV-1a,
so it is stable across Rust versions. It covers the skeleton's asset id, the
skeleton's and clips' bytes, the entries, `rate`, `speed`, `simulation`,
`trajectory`, `mirror_axis` when some entry is mirrored, and each channel's type
name and value with its top-level `weight` field left out.

`weights_of(&database)` gives each live channel's `weight()`, in order.

A stored and a live channel with the same value hash the same.

A channel's top-level `weight` field means the search weight and nothing else.
A field called `weight` that feeds `extract` changes no key, so a cached blob
baked from its old value is reused. A weight stored under another name, or
nested deeper, just changes the key and derives again.

### FeatureQuery

```rust
let query = FeatureQuery::new(&database)?;
let vector = query.build(&database, &skeleton, pose_frame, &simulation, &trajectory)?;
```

`raw` takes the same arguments as `build` and returns the query vector in the
units `preview` reads: `Pose` channels are copied from database frame
`pose_frame`, as the reference does, and `Trajectory` channels are extracted
from `trajectory` expressed in `simulation` (the character's world transform).
`build` is `raw` followed by `database.normalise`, the form the search compares.
Both return `FrameOutOfRange`, `BoneCountMismatch`, `ChannelCountMismatch`,
`TrajectoryTooShort` or `NotBaked` instead of panicking.

`new` copies the database's live channels; `channels()` returns them in the
database's order, which is what `preview_features` takes.

### Layout

The persisted fields of `MotionDatabase` are public. The baked part is read
through accessors on the database, which combine it with the persisted
settings and return `None` (or, for `pose`, `Err(NotBaked)`) while the
database is not baked. Frames are dense and frame-major. Nothing is indexed by
a database-only bone numbering: every bone index is a skeleton joint index.

- `clips()`: one clip handle per entry, in entry order.
- `bone_count()`: the skeleton's bone count at bake time.
- `frame_count()`; `entry_of(frame)`, `range_of(frame)`: the entry holding a
  frame and its half-open frame range; `ranges()`: every entry's range;
  `frames_of_entry(index)`: the frames an entry baked, or `None` past the last
  entry.
- `clip_time(frame)`: the entry and the clip time in seconds the frame plays,
  `clip_start + (frame - range_start) * speed / rate`.
- `trajectory_samples(frame)`: the trajectory samples of `frame` as the bake saw
  them, clamped to its range's last frame. `TrajectorySettings::frame_offset(sample, rate)`
  is the rounded offset the bake and this share.
- `body_motion(frame)`: the simulation bone's velocities at `frame`.
- `live_channels()`: the live channels, in order; `UnloadedChannel` before
  `load_channels`.
- `simulation_bone(frame)`: the simulation bone, a `Transform` in model space.
- `features(frame)`, `feature_dims()`: `feature_dims()` values per frame;
  `channel_range(index)`: the feature indices channel `index` wrote.
- `deviations()`: each channel's mean standard deviation, in channel order.
- `size_in_bytes()`: the bytes held by the feature, simulation and range arrays.

The frame accessors return `None` for a frame outside `0..frame_count()`, and
`pose` returns `Err(FrameOutOfRange)`.

`pose(frame, &skeleton, &clips)` samples the frame's local pose from
`clips[entry]` at `clip_time`, passes it through the mirror if the entry is
mirrored, and re-expresses each parentless joint relative to the simulation
bone. `clips` is one clip per entry, as given to `bake`. It returns
`Err(NotBaked)`, `Err(BoneCountMismatch)`, `Err(ClipCountMismatch)`,
`Err(FrameOutOfRange)` or `Err(Mirror)` instead of panicking.

## Systems

- `drive_body`, in `FixedUpdate`, before `step_simulation`: advances the
  velocity spring one fixed step toward `DesiredMotion::velocity`, with zero
  vertical velocity, advances the facing, and sets the body's horizontal
  velocity to the spring's. It takes the simulation's pull, the world offset
  from the body to where it should be, and adds the velocity that closes half
  of it in the step, at no more than `max_pull_speed` or the spring's own speed,
  whichever is higher. The springs never see the pull, so the prediction is
  undisturbed. The body keeps its own vertical velocity.
- `publish_trajectory`, in `LateFixedUpdate`, after `step_simulation`: predicts
  the path from where the body ended the step and writes it to the
  `AnimationPlayer` on the skeleton of each `CharacterLink`.
- `apply_character_displacement`, in `LateUpdate`, after `animate_targets` and
  before `update_simple_entities` and `propagate_global_transforms`: places each
  character; see [Displacement](#displacement).

An entity without a `BodyId` is skipped. The simulation holds no position: the
body has it.

The prediction starts from the springs and the body's position and holds the
current `DesiredMotion` constant, sampling the same springs `drive_body` steps,
so sample `i` is where the simulation gets to `sample_dt * (i + 1)` from now
while that motion holds. There is no input extrapolation and no collision, so
the prediction does not see walls. Its height is the body's, unchanged.

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

Capsule traces for collision-aware prediction, past samples and gamepad input
are separate work. The simulation does not rotate the capsule.

## Playing a database

`MotionMatchingNode` is an animation graph node that plays a `MotionDatabase`.
`MotionMatchingPlugin` registers it with `register_animation_node`, and its
systems; the plugin is in `DefaultPlugins`, so a game needs nothing more than a
graph that holds the node.

| Field | Meaning | Default |
|---|---|---|
| `database` | the `MotionDatabase` to play | none |
| `search_period` | seconds between searches | 0.1 |
| `velocity_change_threshold`, `rotation_change_threshold` | the change per second of the first trajectory sample's velocity and facing that forces a search once it settles | 50 |
| `same_frame_tolerance` | a search result this many frames or fewer from the playhead is ignored | 1 |
| `ignore_surrounding` | a search returns no other frame fewer than this many frames from the playhead, though the playhead's own frame stays when nothing beats it; at the last frame of a range nothing is excluded around the playhead | 20 |
| `ignore_range_end` | a search never returns the last this many frames of a range; at 0 the playhead can hold the last frame of a range, and a small value can loop it over the last few | 20 |
| `blend_halflife` | inertialization halflife after a jump | 0.1 s |
| `play_rate` | how fast the playhead advances, as a multiple of the database `rate` | 1 |
| `play_rate_param` | a blackboard float param that multiplies `play_rate`; empty, missing or not an `f32` leaves `play_rate` alone | empty |

The node plays at `r = effective_play_rate(play_rate, play_rate_param,
blackboard)` from `concerto_animation::node`: the product, or 0 when it is
negative, NaN or infinite. With `dt` the frame's delta, the playhead and the
root motion advance by `dt * r`. The search timer, the forced-search timer and
the change per second that forces a search run on `dt`, in real time, as do the
inertializer's decay and the blended pose's velocity. A database pose velocity
is per animation second, so it is multiplied by `r` before it is handed to
`Inertializer::transition`, both for the frame jumped to and for the source when
there is no previous blended pose to take the velocity from.

A database frame's future trajectory samples are reached after `1 / r` of
their authored time, so before a search the published samples are rescaled
into animation time: each sample's position relative to the simulation
translation, and its velocity, are divided by `r`, and its rotation is kept.
This is exact for a constant velocity and an approximation while the path
curves. At `r == 1` the samples are used as published. The forced-search check
reads the published trajectory, in real time. At `r == 0` the playhead holds,
the root delta is `RootMotion::IDENTITY`, and no search runs: a search that
comes due, from the timer, a settled change or the range end, is dropped, the
timer restarts and the forced-search cooldown is spent as usual, and the next
search is the next one due while `r` is above 0. The held pose still settles
through the inertializer.

Each fixed step `publish_trajectory` writes the character's predicted path to
the blackboard of its `AnimationPlayer`, where only this crate reads it. A
`CharacterLink` says where both are: `capsule` holds the simulation and
`skeleton` the player. When the player is on the capsule, both fields name that
entity. A player no link names, or a preview `App`, has no trajectory; the node
then plays the database without searching.

Each frame the graph walk reaches it, the node's `update` advances the playhead
at the database `rate` times `r`,
searches when the timer runs out, when a settled change in the trajectory asks
for it, or when the playhead is in the last `ignore_range_end` frames of its
range, and on a better frame jumps with inertialization. The search is brute
force: a squared distance over the normalised features of every frame, with an
early out. The playhead's frame seeds the best, and a candidate replaces it only
when its cost is strictly lower. Frames within `ignore_surrounding` of the
playhead and the last `ignore_range_end` of each range are not candidates, as
in orangeduck's Motion-Matching. At the last frame of a range nothing seeds the
search and nothing around the playhead is excluded, and any frame it finds other
than the playhead's is taken, so the playhead leaves unless `ignore_range_end`
is 0. A query that holds a NaN or an infinity finds nothing.

The step runs inside `update_animation_players`, where the graph is walked, so
a node the walk does not reach, such as one in a state machine state that is
not playing, does not advance. Its `UpdateInput` reads the stores of motion
databases, skeletons and clips, and the player's skeleton comes from the graph
context; when any of them is missing, or the database is not baked yet, the
node does not step and its root motion is `RootMotion::IDENTITY`. `animate_targets` then only copies the pose the step
left. No other system declares the `MotionDatabase` store.

`MotionMatchingState` is the node's per-player state. Its fields are
private: the step that advances it is its own method, which the node's `update`
calls. Other crates read it
through `is_initialised()`, `has_failed()`, `frame()`, `pose()`, `query()` and
`last_query()`.

`MotionMatchingState::last_query()` is a `LastQuery`, kept by each search:
`features` is the raw query vector, `frame` the database frame it copied its
`Pose` channels from, `simulation` and `trajectory` what it was built with
(`trajectory` holds the samples rescaled by the rate, as the features read
them), and
`best_cost` the squared distance to the frame the search found, or `None` when it
found none: a query that holds a NaN or an infinity, or a search at a range's
last frame when every range is no longer than `ignore_range_end`. All five come from one moment, so a
drawn query does not mix bones from one frame with arrows from a later one. It
is `None` until a search runs and after
`reset`, and a search that fails with `TrajectoryTooShort` clears it, so a stale
query is not drawn. A frame with no search due keeps the last one.

The state keeps the simulation bone's velocity and angular velocity at the
playhead, in model space. Each step turns them back by
the root joint's rotation away from rest, `q = simulation * root * rest⁻¹`, and
scales them by the frame's animation time into a `RootMotion` on `state.output`:
`translation = q⁻¹ * (velocity * dt * r)` and `rotation =
from_scaled_angle_axis(q⁻¹ * (angular_velocity * dt * r))`. That is the frame a
clip's root motion reads in, so it reaches `AnimationRootBone` through the graph
the same way. The root joint is the database skeleton's first without a parent,
and `rest` is that bone's rest on the skeleton playing it, so a database baked on
one skeleton plays on another with the same bone ids. A frame with no step, on a
missing asset, gives `RootMotion::IDENTITY`, and so does a frame that holds the
pose at the end of its range.

The root joint is written to the root bone's `Transform` like any other joint,
relative to the simulation bone. Ground motion and yaw belong to the simulation
bone, so what shows on the bone is the root's height, lean and offset from it.

## Displacement

The visible character and the capsule are two entities, and they drift apart:
root motion moves one and the player moves the other. `CharacterDisplacement`
goes on the character and reconciles the two each frame. `CharacterLink`, also
on the character, names the entities it works with. Both fields are
`SceneEntityRef`s, so a link is a scene component: set it in the editor's
inspector and it is saved with the scene, or insert it from code naming
`SceneEntityRef::Entity(..)`. Its default leaves both fields unresolved.

- `capsule` holds `CharacterSimulation`, `DesiredMotion` and the body, and needs
  `ColliderOffset::bottom_origin` for the character to stand on the ground.
- `skeleton` holds the `SkeletonComponent` with a root and its
  `AnimationPlayer`. A scene-spawned skeleton exists only once its scene has, so
  a game that builds the link in code inserts it then.

```rust
commands.spawn((
    CharacterDisplacement::Clamp(Clamp::default()),
    CharacterLink {
        capsule: SceneEntityRef::Entity(capsule),
        skeleton: SceneEntityRef::Entity(skeleton),
    },
    Transform::default(),
));
```

The system reads the space root motion is in from the skeleton's
`GlobalTransform`. On a glTF skeleton that is the skinned node beside the root
joint, so the space is the one the root's parent has. The root's parent must
have no transform of its own: the bake treats a parentless joint's local pose as
model space, and the simulation bone stands at Y = 0 in it, so a transform there
puts the root bone somewhere the database never saw and moves the character the
wrong way. UAL's `Armature` has none, and LAFAN1 has no armature.

A BVH scene puts `SkeletonComponent` on the root joint itself, so its space is
the root's own global, which holds the rotation written to the root. That is on
purpose. A delta has the root's rotation turned out of it, `q⁻¹` above and the
same in a clip's root motion, and a space holding that rotation puts back what
stays in the bone. So a root that stands across Y, as LAFAN1's hips do at about
90°, moves along the body, where a space without it would read forward motion as
vertical and drop it. The two cancel exactly only for a clip. In motion matching
the delta reads this frame's pose and the space holds last frame's blended one,
so they miss by a little every frame, and by more during an inertialized jump.

A `skeleton` with no `SkeletonComponent`, or no root, gives no root motion, and
the character still gets its method's correction. An unresolved `skeleton` is
the same: no trajectory is published, and the character is corrected with no
root motion. A `capsule` with no `CharacterSimulation`, or an unresolved
`capsule`, leaves the character where it is and publishes no trajectory, and
so does having no `CharacterLink`.

A character's `Transform` is written in world space. Do not make it a child of
its capsule: nothing checks, and it would move twice.

### Methods

The variants are mutually exclusive. orangeduck's demo combines adjustment with
clamping; here a character has one or the other. Each holds a struct, so a
variant switch in the inspector gives that struct's defaults and loses tuned
numbers in the old variant.

| Variant | Each frame | Against a wall |
|---|---|---|
| `Synchronization` | The character's position and yaw are a weighted average of its own, moved by root motion, and the capsule's, and the capsule gets the same target. | The capsule stops. The character goes on as far as `data_factor` lets it, and the pull does not take the capsule through the wall while `max_pull_speed` keeps one step under its radius. |
| `Adjustment` | The character moves by root motion and is damped towards the capsule. | The capsule stops. The character keeps going on its root motion, pulled back only as far as the ratios allow. |
| `Clamp` | The character moves by root motion and is kept within reach of the capsule. | The capsule stops, and the character stops within `max_distance` of it. |

`data_factor` is clamped to 0..=1. The average is of positions, taken once a
frame, so at 0 and at 1 the character moves at the code's speed or the data's,
and between them its speed depends on the frame rate and is not the same average
of the two speeds.

Give each character its own capsule: a second character naming the same one
overwrites the first's pull.

| Field | Meaning | Default |
|---|---|---|
| `Synchronization::data_factor` | 0 puts the character on the capsule, code-driven; 1 follows root motion and pulls the capsule to it, data-driven | 1.0 |
| `Adjustment::by_velocity` | cut each correction to its ratio of this frame's root motion, so a standing character is not dragged | on |
| `Adjustment::position_halflife`, `rotation_halflife` | seconds to close half the gap | 0.1 s, 0.2 s |
| `Adjustment::position_max_ratio`, `rotation_max_ratio` | the most a correction may be, as a ratio of this frame's root motion, with `by_velocity` | 0.5 |
| `Clamp::max_distance` | the furthest the character gets from the capsule, on the plane | 0.15 m |
| `Clamp::max_angle` | the furthest its yaw gets from the simulation's | `FRAC_PI_2` radians |

`CharacterDisplacement` defaults to `Adjustment`.

A clip whose root does not move gives no root motion, so a `Clamp` or
`Adjustment` character on such clips stands still while the capsule walks off,
as far as the method lets it. Such a game wants `Synchronization { data_factor:
0.0 }`.

### Height, pull and yaw

Height is always the capsule's, in every method. The vertical part of root
motion is ignored, as is any turn out of yaw.

Only `Synchronization` moves the capsule. `apply_character_displacement` sets
the simulation's pull to the offset from the capsule to the target each frame,
and every other method leaves it at zero. `drive_body` takes it and adds half
of it as velocity, within the limit above, so physics still resolves
collisions. The capsule shows the body up to a step late, which is why it adds
half: a full correction would be issued again before the last one showed. So
the capsule closes the gap over a few steps, and lags a character running ahead
of its spring by about two fixed steps.

Root motion past `max_pull_speed` leaves the capsule behind, and the gap grows
until the character slows down: the default clears LAFAN1's sprints, which reach
6.8 m/s at a database `speed` of 1.1. Only a frame's first fixed step takes the
pull, so a frame long enough for two steps halves what the pull can reach.

Yaw needs no pull, since the facing is not physics. Under `Synchronization` the
system turns the simulation's facing and the facing it interpolates from by the
same amount, so the simulation's yaw lands on the target while the spring keeps
its velocity. At a `data_factor` of 0 that turn is zero and the spring is left
to itself.

The capsule's position is its interpolated `Transform`, and the simulation's
yaw is interpolated between fixed steps the same way, so neither ticks at the
fixed rate.

### Transform order

Writing the character's `Transform` before propagation means the character, its
skeleton and its model reach rendering in the same frame, whether the character
has children or not. The orderings on
`apply_character_displacement` also put `update_simple_entities` and
`propagate_global_transforms` after `animate_targets` in every `App` with both
`MotionMatchingPlugin` and `AnimationPlugin`, so bones reach their globals a
frame sooner there than in an `App` with `AnimationPlugin` alone.

The system is not marked `.at_preview_time()`, since a preview has no physics.
Its orderings still hold there, so a preview has the same transform order.

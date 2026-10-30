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

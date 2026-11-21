# ragdoll-test

A manual visual check of physics ragdolls: a three-part capsule chain is
dropped, tilted, onto a floor and swings on its joints until it rests.

Keys:

- `R` puts the chain back at its start pose.
- `Space` pokes the last part with an impulse.
- `M` hangs the chain from a kinematic root and drives it to a bent pose with
  its motors.
- `L` lets it go: the root turns dynamic and the motors turn off.

A humanoid character stands to the right of the chain, walking in a circle
with swinging arms. Its ragdoll's parts are kinematic and follow the bones,
and white lines and crosses draw the skeleton, so the parts and the bones can
be compared. Its bones are reset to the rest pose every frame, as an animation
would rewrite them.

- `H` lets the humanoid go limp: it falls and the simulated pose is drawn on
  the skeleton.
- `J` puts it back to following its animation with weight 0.
- `B` sets the weight to 0.5, to see the blend.
- `P` powers the humanoid with the `flinch` profile: pelvis and legs follow the
  animation exactly, everything above is held to it by motors.
- `O` powers it with the `braced` profile: every part on motors, standing on
  its own feet until it loses its balance.

Set `RAGDOLL_TEST_LIMP_AFTER=<seconds>` and it goes limp that long after
startup, for capturing without a keyboard.

Run it with `cargo run -p ragdoll-test`. It needs the Jolt backend, so it does
not run on the web.

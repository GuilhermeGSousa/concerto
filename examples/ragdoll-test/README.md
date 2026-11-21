# ragdoll-test

A manual visual check of physics ragdolls: a three-part capsule chain is
dropped, tilted, onto a floor and swings on its joints until it rests.

Keys:

- `R` puts the chain back at its start pose.
- `Space` pokes the last part with an impulse.
- `M` hangs the chain from a kinematic root and drives it to a bent pose with
  its motors.
- `L` lets it go: the root turns dynamic and the motors turn off.

Run it with `cargo run -p ragdoll-test`. It needs the Jolt backend, so it does
not run on the web.

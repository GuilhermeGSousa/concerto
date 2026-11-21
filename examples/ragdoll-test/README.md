# ragdoll-test

A manual visual check of physics ragdolls: a three-part capsule chain is
dropped, tilted, onto a floor and swings on its joints until it rests.

Keys:

- `R` puts the chain back at its start pose.
- `Space` pokes the last part with an impulse.

Run it with `cargo run -p ragdoll-test`. It needs the Jolt backend, so it does
not run on the web.

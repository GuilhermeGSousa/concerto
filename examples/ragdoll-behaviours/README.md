# ragdoll-behaviours

One humanoid standing in its rest pose, and a key for every physical
behaviour of `concerto-ragdoll`. The panel lists the keys and shows, live, the
body's mode, its state, the behaviours running with the parts they claim, and
what it senses. The body is drawn by the physics gizmos: its parts, its
contacts, its centre of mass, support and capture point.

| Key | What it does | What to look for |
|---|---|---|
| `1` | Adds `Balance`, which never settles here | The body stands on its own feet, state `Reacting` |
| `2` | Pushes the chest with 70 N·s | While balancing: a step or two, then standing |
| `3` | The same push with a `Stagger` of severity 0.5 | More, shorter steps, the trunk bent, the arms circling |
| `4` | A shot to the chest, 40 N·s | The chest thrown back, a hand to the wound, the head turning |
| `5` | A shot to the left thigh | That knee sags and its foot leaves the ground |
| `6` | Shots to both thighs | The knees fold and the body goes down |
| `7` | A shot to the left upper arm | That arm hangs |
| `8` | Pushes the chest with 300 N·s | Balance gives up, then the fall: hands out, head held away, limp once down |
| `9` | `Die` over one second | Strength drains from the hands and feet inwards; `Dead`, then settled |
| `0` | A shot to the head and `Die` at once | Limp on the spot |
| `F` | Adds `Fall` | The braced fall from standing |
| `H` | A 300 N·s hit on the pelvis | A settled body is thrown and settles again |
| `D` | Turns the pushes and shots: towards the body's back, front, left, right | |
| `G` | Shows or hides the gizmos | |
| `R` | Replaces the humanoid with a fresh one | |

The humanoid has a `ShotReaction` and a `FallReaction`, so `4` to `7` work
from standing and bring their own balance, and any lost balance ends in a
fall. `2`, `3` and `8` push a simulated part, so they do nothing until `1` or
a shot has made the body physical.

Run it with `cargo run -p ragdoll-behaviours`. It needs the Jolt backend, so
it does not run on the web.

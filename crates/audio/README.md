# concerto-audio

Sound for Concerto games, on [kira](https://docs.rs/kira/0.12): imported clips
played by a component on an entity, mixed on four buses under a master.

`DefaultPlugins` registers `AudioPlugin` after `WindowPlugin`, re-exported as
`concerto::audio`. On its own it needs an `AssetServer` (`AssetManagerPlugin`).

## Clips

`AudioImporter` imports `.ogg` and `.wav` sources. Each becomes one
`AudioClip`, `main`, holding the file's encoded bytes, which `on_load` decodes
once to PCM. `duration()` and `sample_rate()` read a loaded clip. See
[docs/asset-importing.md](../../docs/asset-importing.md).

## Playing a sound

Spawn an `AudioPlayer` with a clip from `AssetServer::load`:

```rust
let clip = server.load::<AudioClip>(asset_id!("content/audio/step/main.gasset"));
cmd.spawn(AudioPlayer::new(clip).with_volume(0.6));
```

| Field | Meaning | Default |
|---|---|---|
| `clip` | the clip to play | set by `new` |
| `bus` | the bus it plays on | `Effects` |
| `volume` | linear: 0 or less is silent, 1 unchanged | 1 |
| `speed` | playback rate, which also shifts the pitch | 1 |
| `looping` | loop the whole clip; a looping sound never ends | `false` |
| `end` | what happens when the sound ends | `Despawn` |

Each field has a `with_<field>` builder. A player waits until its clip is
loaded and the `Audio` resource exists, then starts. Changing `volume`,
`speed` or `looping` on a playing player applies to the sound with a 10 ms
tween; changing `clip` or `bus` stops it and plays the new setting afresh.

Despawning the entity, removing its `AudioPlayer`, or inserting a new one over
it stops the sound with a 10 ms fade. A new player then starts from the
beginning.

### The end of a sound

When a sound ends, a `SoundFinished { entity }` event is sent once, and then
`end` acts:

- `PlaybackEnd::Despawn` despawns the entity.
- `PlaybackEnd::Remove` removes the `AudioPlayer` and leaves the entity.
- `PlaybackEnd::Keep` keeps the `AudioPlayer` and plays nothing more until a
  new one is inserted.

A player whose clip fails to load warns and ends the same way, with its
`SoundFinished`. One spawned after its clip failed waits silently, since the
failure event has passed.

## Buses

`AudioMixer` holds a linear volume for `master`, `music`, `ambience`,
`effects` and `dialogue`, each 1 by default. Writing the resource changes the
mix with a 10 ms tween, so an options menu writes it and nothing else.
`AudioBus` names the four buses.

## The device and the mock backend

`AudioPlugin::finish` opens the output named by the `AudioBackend` resource
and inserts `Audio`:

- `AudioBackend::Device`, the default, opens the default output device. With
  no device (CI, a container, a server) it warns once and runs silently on
  kira's mock backend.
- `AudioBackend::Mock` always uses the mock backend, at 1000 frames a second.
  `concerto::headless` inserts it before the recipe; a test that registers
  `AudioPlugin` itself inserts it before the plugin.

An edit `App` (`App::for_editing`) gets no `Audio` and plays nothing. The
`Audio` resource is dropped with its `App`, which stops every sound.

On the web a browser starts audio only after a user gesture, so `finish` opens
nothing there. The first key or mouse button press opens the output and
inserts `Audio` on that frame. A gamepad press is not a gesture and does not
open it. Players spawned before then wait, and start once `Audio` exists, so a
title screen's music begins on the first click.

On the mock backend nothing renders on its own: `Audio::advance(seconds)`
renders that much audio, so sounds move forward and end. On a device it does
nothing. `Audio::bus_sound_count(bus)` says how many sounds a bus is playing.

`AudioPlayer`, `AudioMixer`, `AudioBus` and `PlaybackEnd` are reflected, so
the inspector and MCP's `get_component` see them on a playing game's entities.

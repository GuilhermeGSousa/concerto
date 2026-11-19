# The level authors the main camera

Status: approved by the owner on 2026-11-19.

Follows the procedural sky (`.agents/specs/2026-11-18-sky-design.md`). A level
camera with a `Sky` never draws in Play, because the camera on screen is the
one `CameraDirectorPlugin` spawns at startup, outside any scene. Neither a
person nor an agent can give that camera a `Sky` or a `CameraEnvironment`, and
the level's own cameras overlap it.

## Goal

The real camera is authored in the level, marked `MainCamera`; the director
drives it from virtual cameras and no longer spawns one. Cameras imported from
glTF become dormant virtual cameras, so an import adds shots, not window
cameras.

## Decisions taken with the owner

| Question | Decision |
|---|---|
| Who owns the real camera | The level: an entity with `MainCamera`. The director spawns nothing. |
| glTF cameras | Imported as `VirtualCamera`, disabled, priority 0, with the glTF lens. No `Camera`. |
| Editor viewport mirror source | The scene's `MainCamera` only. Without one, the viewport copies nothing. |
| Tech demo | The owner adds a `MainCamera` to its level in the editor. Until then its Play view is empty. |
| render-test | Re-imported after a backup; its Sky Camera gets `MainCamera`. |

## How Bevy, Unity and Unreal do it

Bevy spawns no camera: the game spawns `Camera3d`, and glTF cameras import as
real cameras. Unity's scenes hold a Main Camera, and Cinemachine's virtual
cameras drive it through a brain on that camera; that is this design. Unreal
spawns its camera manager in code and keeps the sky and post-processing in
level actors, the other consistent design, rejected for the sky's sake.

## Public API (`concerto-director`)

```rust
/// Marks the level's camera that the director drives from the live virtual camera.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Camera)]
pub struct MainCamera;
```

- `CameraDirectorPlugin` registers it. It stops spawning a camera and the
  lowest-priority fallback `VirtualCamera`.
- Removed: `CameraDirector::with_main_camera`, `spawns_main_camera`,
  `set_main_camera`, and `main_camera::spawn_main_camera`.
- `CameraDirector::main_camera()` stays and returns the camera being driven.
- **Which camera.** The `MainCamera` with the lowest `Entity::index()`, as the
  sky picks its active sun. Others log one warning each and are not driven.
  They still draw, so the renderer's overlap warning applies as it does
  today. The choice is recomputed each frame by `adopt_main_camera`, before the drive,
  so a level loading or a Play/Stop respawn needs no lifecycle bookkeeping.
- **No live virtual camera.** The director leaves the main camera's
  `Transform` and `Camera` lens alone, so the authored pose shows. A virtual
  camera going live from there cuts to its pose, as a takeover with no
  outgoing virtual camera does today. When the last live camera leaves, the main
  camera stays where it was.
- **Parented cameras.** A scene's nodes are children of the scene's root, so
  the main camera usually has a parent. The director writes the pose in the
  parent's space, computed from the camera's own `Transform` and
  `GlobalTransform` (`local' = local × global⁻¹ × world'`), with no lookup of
  the parent. The `MainCamera` doc's "never despawned or re-parented" note
  goes.

## The glTF importer

`crates/gltf/src/gltf_importer.rs:550-573`: a node's camera becomes
`VirtualCamera::new(0).disabled()` with
`lens: Some(Lens { fovy: yfov, znear, zfar })`. A glTF with no `zfar` takes
`Lens::default()`'s. The glTF aspect ratio is dropped, because the main camera
follows its target. An orthographic camera warns, as today, and gets the
default lens. The importer no longer emits `Camera`.

Existing imports keep their `Camera` components until they are re-imported.
Content is marked outdated by asset format version, not by importer, so
nothing flags them. The book says to re-import.

## Editor mirror

`crates/editor/src/tools/environment.rs`: the source of `Sky` and
`CameraEnvironment` is the scene's `MainCamera` with the lowest
`Entity::index()`, among entities without `EditorHelper`. Without one, the
editor camera holds neither. The lowest-index-camera rule goes. The editor
gains a `concerto-director` dependency.

## Games

- **One More Night** (`../one-more-night/src/players.rs:96`): drop
  `.insert_resource(CameraDirector::default().with_main_camera(false))`. Its
  split-screen cameras are spawned in code without `MainCamera`, so the
  director drives none of them, as before. The edit is left uncommitted,
  because that repository is on `master`.
- **Tech demo:** no code change. Its `body.rs` test spawns `MainCamera`
  itself. The owner adds a `MainCamera` to its level.

## Tests

| Test | Change |
|---|---|
| `the_plugin_spawns_one_main_camera_unless_the_game_turned_it_off_first` (director lib) | Replaced by `the_director_drives_the_level_main_camera`: no camera is spawned; of two `MainCamera`s the lower index is driven; with no live virtual camera the authored pose stays; a virtual camera takes over and, once removed, leaves the camera where it was; a parented main camera ends at the virtual camera's world pose. Net 0. |
| `the_highest_enabled_camera_drives_the_main_camera_…` (director) | Its fixture spawns the `MainCamera` instead of calling `set_main_camera`. |
| glTF import scenario | One assertion: a camera node imports as a disabled `VirtualCamera` with the glTF lens and no `Camera`. The fixture gains a camera node if none has one. |
| `the_viewport_mirrors_the_scene_cameras_sky_and_environment` (editor) | The sources carry `MainCamera`; a camera without it is not copied. |

## Documentation

- `docs/book/06-rendering.md`, *Cameras* and *Split screen*: a level holds a
  `MainCamera`; the director-without-camera passages go; imported glTF
  cameras are dormant virtual cameras; re-import old content.
- The `concerto-director` crate docs (`lib.rs`, `MainCamera`).
- `crates/editor/README.md` and `.agents/reference/gotchas.md`: the mirror
  copies from the `MainCamera`.
- `crates/skybox/README.md`: put `Sky` on the level's `MainCamera`.

## render-test (after the code lands)

Back up `content/main.gasset`, `content/sponza/` and `content/sponza_curtains/`
to `.bak` copies. Re-import the two Sponza sources through the editor, add
`MainCamera` to "Sky Camera", save, then look at the editor viewport and Play
with the `visual-verification` skill.

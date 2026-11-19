# Procedural sky with time of day

Status: approved by the owner on 2026-11-19, with the answers recorded in
*Decisions confirmed*. Reviewed once by `spec-reviewer`; its findings are
folded in.

Closes #512 (the skybox draws nothing) and #886 (the editor viewport shows
none of the scene camera's environment). Builds the sky that #655 (One More
Night's dusk-to-night lighting) needs, without the moon. Makes #866 obsolete in
its current form, because the cube-map material it describes is deleted; see
*Follow-up work*. Leaves the HDR target to #427.

## Goal

The Sponza scenes get a physically based sky. One directional light marked
`Sun` sets the time of day: turning it moves the sun across the sky and drives
dawn, noon, sunset and night. The sky also lights the scene. Ambient light comes
from the sky instead of one flat color, the sun dims and reddens near the horizon,
and distance fog takes the sky's color in the direction you look.

## Decisions taken with the owner

| Question | Decision |
|---|---|
| Sky model | Hillaire 2020 atmosphere using only 2D lookup tables (transmittance, multiple scattering, sky view). No 3D aerial-perspective volume. |
| Lighting | Sky irradiance (L2 spherical harmonics) replaces the flat ambient, and atmospheric transmittance tints the Sun light. |
| Night | Fades to a configurable night sky color and ambient floor. No stars or moon. |
| Skybox crate | Fix it and build the sky in it. Cube maps are follow-up work. |
| Placement | A `Sky` component on a camera placed in the level turns the sky on for that camera, and names its sun with an entity reference. |
| Units | Relative to the Sun: sky radiance scales with the Sun light's color and intensity under the existing per-material ACES. No HDR target or exposure in this change. |
| Where lighting is computed | On the CPU, when the Sun, its atmosphere or a `Sky` changes (change detection). |
| Fog | Per pixel: with a sky, fog takes the sky radiance in the view direction. |
| Web | Nice to have. The design is fragment-only and works on WebGL2. |

Facts found while writing this changed three of those answers slightly; see
*Decisions confirmed* at the end.

## How Bevy does it, and what differs

Bevy 0.16 added an `Atmosphere` component on the camera, implemented the
Hillaire way. Compute shaders build a transmittance LUT, a multiple-scattering
LUT, a sky-view LUT and a 3D aerial-perspective volume every frame. It needs an
HDR camera and doesn't run on WebGL2. In 0.16 the atmosphere only draws the sky
and hazes geometry; it does not light the scene. Bevy 0.17 adds lighting
through an environment map generated from the atmosphere. In both, the sun's
color is not changed for you. A separate `Skybox` component draws a cube map
as a fullscreen triangle, rebuilding each pixel's view direction from the
inverse view-projection.

Here:

- **No compute shaders.** The transmittance and multiple-scattering LUTs depend
  only on the atmosphere's settings, so the CPU builds them and uploads them.
  The sky-view LUT depends on the sun, so the GPU renders it in a fragment pass,
  and only when the sun or the atmosphere changes. Everything runs on WebGL2.
- **No aerial-perspective volume.** The existing exponential fog colored by the
  sky in the view direction replaces it.
- **No HDR target.** The sky shader applies the same ACES curve the standard
  material does.
- **Lighting through spherical harmonics.** Diffuse light from the sky comes
  from L2 coefficients computed on the CPU, not from an environment map.
- **The sun's light is tinted for you.** Transmittance multiplies the `Sun`
  light's color.
- **Medium on the light.** The atmosphere's medium (scattering coefficients,
  planet size) lives on the `Sun` entity, not the camera (see *Decisions to
  confirm*).

## Public API

### `concerto-skybox`

```rust
/// Marks the directional light that is the sun; its rotation sets the time of day.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Light = Light::directional_light())]
pub struct Sun {
    pub atmosphere: Atmosphere,
    /// Angular radius of the drawn sun disk, in degrees.
    pub disk_radius: f32,
}

/// The air the sunlight passes through, in kilometres and per-kilometre coefficients.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct Atmosphere {
    pub planet_radius: f32,
    pub atmosphere_height: f32,
    pub ground_albedo: Color,
    pub rayleigh_scattering: Vec3,
    pub rayleigh_scale_height: f32,
    pub mie_scattering: f32,
    pub mie_absorption: f32,
    pub mie_scale_height: f32,
    pub mie_asymmetry: f32,
    pub ozone_absorption: Vec3,
    pub ozone_center_height: f32,
    pub ozone_width: f32,
}

/// Draws a sky behind everything this camera sees and lights the scene with it.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Camera)]
pub struct Sky {
    /// The directional light with a `Sun` this sky follows.
    pub sun: SceneEntityRef,
    /// Radiance added to the sky and to sky-colored fog, so night is not black.
    pub night_color: Color,
    /// Light added to the ambient term, in the units of `CameraEnvironment::ambient`.
    pub night_ambient: Color,
}
```

- **Defaults.**
  - `Atmosphere::default()` is Earth, with Hillaire's and Bevy's values:
    - planet radius 6360 km, top of the atmosphere at 100 km;
    - Rayleigh scattering `(5.802, 13.558, 33.1) × 1e-3 /km`, 8 km scale height;
    - Mie scattering `3.996e-3`, absorption `4.4e-3`, 1.2 km scale height,
      `g = 0.8`;
    - ozone `(0.650, 1.881, 0.085) × 1e-3`, in a 30 km-wide tent centred at
      25 km;
    - ground albedo 0.3.
  - `disk_radius` is 0.27°.
  - `night_color` and `night_ambient` are dark blue-grey, at 0.01 and 0.02
    linear, tuned once on Sponza.
- **Builders.** Each struct gets `Default` and a consuming `with_<field>` per
  field (repository rule for wide structs).
- **Requirements.**
  - `Sun` requires a *directional* `Light` with the documented
    `#[require(Type = expr)]` form. A `Light` the user already supplied wins
    (`docs/required-components.md`). So a `Sun` on a spot or point light is
    possible, and is ignored with one warning.
  - `Sky` requires `Camera`, which brings `SyncWithRenderWorld`.
- **Naming the sun.** `Sky::sun` is a `SceneEntityRef`, as `CharacterLink`
  names its capsule. The editor's entity picker edits it and `set_property`
  writes it. Saving fails while it names a deleted entity, as for any
  reference.
  - A `Sky` whose `sun` is unset, or names an entity without `Sun` and a
    directional `Light`, draws night (`night_color` and `night_ambient`
    only), with one warning per camera.
- **One active sun per frame.** There is one set of atmosphere tables and
  one sky-radiance map.
  - The active sun is the one named by the `Sky` camera with the lowest
    entity index whose reference resolves.
  - A `Sky` camera that names a different sun draws with the active sun and
    warns once.
- **Every `Sun` tints its own light.** Each directional `Sun` writes its
  `LightTint` from its own `atmosphere`, whether or not a `Sky` names it.
- **Sun direction.** The direction *toward* the sun is `rotation * Vec3::Z`,
  the opposite of the light's travel direction (`-(rotation * Z)`, as
  `Light::attenuation_at` and the shadows use).
- **The sun tints its light even without a `Sky`.**
  - A `Sun` writes `LightTint` on its light whether or not any camera has a
    `Sky`, because the sun sits in its atmosphere regardless of who looks.
  - Marking a light `Sun` is therefore what opts a scene in to the warmer,
    dimmer low sun.
  - A scene with neither component looks exactly as it does today.

### `concerto-render`

The render crate can't depend on the skybox crate. It gains the slots the sky
fills and the standard material reads:

```rust
/// One radiance sample over the sphere, for building spherical harmonics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadianceSample { pub direction: Vec3, pub radiance: Vec3, pub solid_angle: f32 }

/// Irradiance as spherical harmonics up to band 2, one RGB coefficient per basis function.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SphericalHarmonics { pub coefficients: [Vec3; 9] }

/// Light a camera's sky casts on surfaces, kept up to date by the sky; not saved.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(SyncWithRenderWorld)]
pub struct CameraSkyLight {
    pub irradiance: SphericalHarmonics,
    pub night_color: LinearRgba,
    pub night_ambient: LinearRgba,
}

/// A factor multiplied into a light's color, kept up to date by systems such as the sky; not saved.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(SyncWithRenderWorld)]
pub struct LightTint(pub LinearRgba);

/// Render world: sky radiance in every world direction, sampled by sky-colored fog.
#[derive(Resource)]
pub struct SkyRadianceMap {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub format: wgpu::TextureFormat,
}
```

- **Not saved.** `CameraSkyLight` and `LightTint` are not reflected and not
  scene components, so computed lighting never lands in a saved scene or shows
  as editable in the inspector.
- **`SphericalHarmonics`.**
  - `SphericalHarmonics::from_radiance(samples: &[RadianceSample])` projects
    the samples and stores *irradiance* coefficients: the cosine convolution
    is folded in, so the shader does none.
  - `irradiance(normal) -> Vec3` evaluates them.
  - The coefficients are an array of nine `Vec3`s, in the standard real-SH order.
- **Camera uniform.**
  - `extract_cameras` reads `Option<&CameraSkyLight>`.
  - When present, it writes the nine coefficients, `night_color`,
    `night_ambient` and a "sky lit" flag into `CameraUniform`.
  - When absent, the flag is clear and the flat `CameraEnvironment::ambient`
    applies, as today.
  - `CameraUniform` also gains `inverse_view_proj` for the sky pass.
  - The existing test `camera_uniform_keeps_its_wgsl_layout` is updated for
    the new size.
- **Light uniform.** `extract_lights` reads `Option<&LightTint>` and multiplies
  it into the `RenderLight` color.
- **Sky-radiance map, bound per camera.**
  - The camera bind-group layout (`@group(1)`) gains binding 1, a filterable
    `texture_2d<f32>`, and binding 2, a filtering sampler.
  - `RenderPlugin` inserts a 1×1 black `SkyRadianceMap`. The skybox crate
    replaces it once at startup with its own map, then renders into that same
    texture in place, so the resource changes only when it is replaced.
  - `extract_cameras` takes `Res<SkyRadianceMap>`, 11 parameters of the 12
    allowed, and binds the map when it creates a camera's bind group.
  - A new render system, `rebind_sky_radiance`, runs in `Render` before
    `Background`. When `SkyRadianceMap` `is_changed()`, it rebuilds every
    `RenderCamera`'s bind group.
  - Every existing user of `CameraLayout` keeps working:
    - debug-gizmos binds the camera's own group;
    - every shader declares a prefix of `CameraUniform`;
    - a pipeline may use a subset of a layout's bindings.
- **Map encoding (the contract).**
  - The map is equirectangular in world space: `u = atan2(d.x, -d.z) / 2π + 0.5`,
    `v = 0.5 - asin(d.y) / π`.
  - Texels hold radiance in the same relative units as light colors.
  - The format is `Rgba16Float`. Where that format can't be a render
    attachment, it is `Rgba8Unorm` holding `sqrt(radiance / SKY_RADIANCE_SCALE)`.
    - A new `RenderCapabilities::float_render_targets` decides which, read
      from the adapter's format features for `Rgba16Float`.
    - The shader def `SKY_RADIANCE_SQRT` selects the decode.
    - The implementer confirms wgpu 30's API for this, and what the GL
      backend reports with and without `EXT_color_buffer_float`, before
      writing it.
  - The standard shader and the sky shader both use this one mapping, written
    once in each WGSL file and documented on `SkyRadianceMap`.
- **Render set.**
  - A new `RenderSet::Background` is chained between `Shadows` and `Draw`.
  - `clear_cameras` moves into it. Today it runs before `Draw` only because
    it shares `ResMut<RenderDevice>` with the material passes and is
    registered first. The set makes that order explicit.
  - Inside it, `BackgroundSet { Clear, Draw }` are chained. `clear_cameras` is in `Clear` and the sky pass in `Draw`, so neither names the other's system.

### Standard shader changes

- **Ambient.** When the camera is sky lit, ambient is
  `(irradiance(N) / π + night_ambient) × base_color × occlusion`, with `N` the
  normal-mapped normal. Otherwise ambient is unchanged.
- **Fog.** When the camera is sky lit and has fog, the fog color is the
  sky-radiance map sampled in the direction from the camera to the fragment,
  plus `night_color`.
  - The direction's `y` is clamped to at least 0, so fog below the horizon
    takes the horizon color.
  - Fog density and start still apply, and `Fog::color` is ignored.
- **Tonemapping.** It stays where it is (ACES at the end of `fs_main`).

## How it runs

### Plugins

`concerto-skybox` exports two plugins, and `SkyboxPlugin` registers both:

- **`SkyPlugin`** is the main world only and needs no GPU. It registers the
  types, the `AtmosphereLuts` resource and the systems and listeners below.
  The feature scenario builds an `App` with `MainSchedulePlugin`,
  `TransformPlugin` and `SkyPlugin`.
- **`SkyRenderPlugin`** is the render world. It owns the LUT textures, the
  sky-view pass, the sky pass and the replacement `SkyRadianceMap`, and
  requires `RenderPlugin`.

`SkyboxPlugin` stays in `DefaultPlugins`.

### Main world (`SkyPlugin`)

Every system and listener here is marked `.at_edit_time()`, so it runs in the
editor's edit `App` as well as in a game.

| System | Runs when | Does |
|---|---|---|
| `build_atmosphere_luts` | Every frame; rebuilds only when the active sun's `Atmosphere` differs from the one the tables were built from (a value comparison, which also covers the active sun changing); with no active sun it builds nothing | Builds the transmittance LUT (256×64) and the multiple-scattering LUT (32×32) on the CPU from the active sun's `atmosphere` into `AtmosphereLuts`. The resource is inserted empty at build and holds the LUT data and the `Atmosphere` they were built from; readers see a rebuild through `is_changed()`. It writes the resource directly through `ResMut`, not through commands. |
| `update_sun_lighting` | Chained after `build_atmosphere_luts`; does work when any of these changed: `AtmosphereLuts`, `Changed<Sky>` (covers added), `Changed<Sun>`, `Changed<Light>` or `Changed<GlobalTransform>` on a `Sun` entity, or a `Sun` was removed (`RemovedComponents<Sun>`, after `App::track_removals::<Sun>()`). | Resolves each `Sky`'s sun and the active sun (*Naming the sun*). Inserts or updates `LightTint` on every directional `Sun`, from its own atmosphere: transmittance from the ground toward the sun, zero below the horizon, with a smooth fade across the disk. Integrates the sky on the CPU into `SphericalHarmonics`: 256 Fibonacci directions, 16 march steps each, through the CPU LUTs. The upper hemisphere is sky radiance; the lower is ground bounce, `ground_albedo × irradiance on the ground / π`. Scales by the Sun light's color × intensity and inserts or updates `CameraSkyLight` on every `Sky` camera. A `Sky` whose sun does not resolve gets zero irradiance (night). |
| listener `On<Remove<Sky>>` | a `Sky` is removed | Removes `CameraSkyLight` from that camera. |
| listener `On<Remove<Sun>>` | a `Sun` is removed | Removes `LightTint` from that light. |

The CPU maths lives in `crates/skybox/src/atmosphere.rs` as plain functions
over `Atmosphere`: density at height, optical depth, transmittance, the phase
functions, single scattering, and Hillaire's multiple-scattering approximation.
The GPU shaders port the same functions. Observer height is fixed at 200 m
above the ground; world units don't map to the planet's kilometres, and without
aerial perspective camera height has no visible effect.

### Render world (`SkyRenderPlugin`)

Both GPU passes are plain wgpu pipelines owned by the skybox crate, not
material-macro materials:

- The sky-view pass targets the map's format, not the surface format that
  material pipelines use.
- The sky pass needs a hand-built bind group for the sun. It reads the map through the camera bind group and needs no LUTs: the disk's transmittance is the sun's `LightTint`, so the disk and the light it casts agree.

| System | Set | Does |
|---|---|---|
| `extract_sky` | `Extract` | Copies the sun direction, `Sun::disk_radius`, the sun light's color × intensity, its `LightTint`, which cameras have a `Sky` with their night terms, and `AtmosphereLuts` when it `is_changed()`. |
| `upload_atmosphere_luts`, then `render_sky_view` | `Render`, chained, before `Background` | `upload_atmosphere_luts`: when the LUTs changed, converts them to f16 (the `half` crate, already in `Cargo.lock` through wgpu, becomes a direct dependency) and writes them to two `Rgba16Float` textures. `render_sky_view`: renders the sky-radiance map (256×128, allocated once at startup) in place. A fragment pass raymarches 32 steps per texel through the LUTs. It runs when the sun direction, light or LUTs differ from the values it last rendered with, which it keeps in a `SystemLocal`. |
| `draw_sky` | `RenderSet::Background` and `BackgroundSet::Draw`, after the clears in `BackgroundSet::Clear` | For each camera with a `Sky`, it draws a fullscreen triangle into the camera's viewport rect. There is no depth test or write; opaque geometry covers it in `Draw`. The view direction is rebuilt from `inverse_view_proj`. The output is map radiance, plus the analytic sun disk, plus `night_color`, then ACES. The sun disk is `LightTint` × illuminance / disk solid angle, with limb darkening. |

Deleted from `crates/skybox`:

- `SkyboxCube`, its mesh, and the cube vertex and index arrays.
- `SkyboxMaterial`, its shader (which rotated by the top 3×3 of `view_proj`,
  projection included), and its `MaterialPlugin::<SkyboxMaterial>::new()`
  registration. That registration also removes the empty
  `material_renderpass::<SkyboxMaterial>` pass that every camera opens each
  frame today, and the `MaterialComponent<SkyboxMaterial>` reflected type.

`crates/skybox/Cargo.toml` gains `concerto-reflect`, `concerto-color` and
`half`, and drops what the deleted code alone used. The `Cargo.lock` that cargo
rewrites is committed with it.

Without a `Sun` and a `Sky`, the sky adds no per-frame cost:

- The LUTs are built only when a `Sun` changes.
- The sky-view pass runs only when its inputs change.
- Nothing is drawn until a camera has a `Sky`.
- The one allocated cost is the 256×128 map.

## Editor and agents

- **Reachable today.** `Sun`, `Atmosphere` and `Sky` are reflected scene
  components, so the inspector shows and edits them. The existing MCP tools
  `add_component` and `set_property` reach every setting:
  - marking a light `Sun`;
  - adding `Sky` to a camera in the level, and pointing its `sun` at that
    light with `set_property` (an entity id, as for any `SceneEntityRef`);
  - turning the sun through its transform;
  - tuning the atmosphere.

  No new tool is needed.
- **What shows when.** An edit `App` draws only the editor camera
  (`ExclusiveCamera`), which is outside the open scene. The viewport
  environment mirror below copies the scene camera's `Sky` and
  `CameraEnvironment` onto it, so the editor viewport shows the sky, its
  ambient, the sun's tint and the fog while editing, as Play does.
- **README.** `crates/skybox/README.md` gives the agent recipe:
  `add_component Sky` on the camera, `add_component Sun` on a directional
  light, `set_property` on the light's rotation. The viewport shows the
  result while editing and in Play.

### Viewport environment mirror (`concerto-editor`)

Closes #886. Bevy has no equivalent: its editor-less scenes render through
whichever camera the game spawns.

- **Plugin.** `ViewportEnvironmentPlugin`, in
  `crates/editor/src/tools/environment.rs`, registered by `build_viewport`
  next to the camera tools. A play `App` gets only `EditToolsCorePlugin`, so
  the mirror runs only where the editor camera exists. The editor gains a
  `concerto-skybox` dependency, as it has `concerto-world-grid`.
- **What it copies.** `Sky` and `CameraEnvironment`, each on its own.
- **From where.** For each of the two, the source is the scene camera with the
  lowest `Entity` that has it, among cameras without `EditorHelper`. Different
  cameras may supply the two.
- **How.** One `Update` system, `mirror_scene_environment`, marked
  `.at_edit_time()`. Each frame it compares each source value with the editor
  camera's copy. When they differ, it inserts the source's value on the editor
  camera with `CommandQueue::insert`. When there is no source and the editor
  camera has a copy, it removes the copy. It compares values, so it writes
  nothing while nothing changed, and the skybox's `Changed<Sky>` systems stay
  idle.
- **What follows.** The skybox's `update_sun_lighting` sees the editor
  camera's `Sky` like any other and writes its `CameraSkyLight`. Removing the
  mirrored `Sky` triggers the skybox's `On<Remove<Sky>>` listener.
- **Agents.** There's no new tool. An agent changes the scene camera with
  `add_component`, `set_property` and `remove_component`, and the viewport
  follows.
- **Not covered.** Time-of-day animation is the owner's to drive, for example by
  animating the Sun's rotation. This change adds no clock.

## Tests

Following `docs/testing.md`: maths gets a table test and one property, the
feature gets one scenario, and the GPU output gets one readback test that skips
without an adapter.

**New tests in `crates/skybox`**, with its integration tests in
`crates/skybox/tests/skybox/main.rs`:

| Test | Kind | Asserts |
|---|---|---|
| `atmosphere::tests::transmittance_and_scattering_behave_physically` (unit, table) | Algorithm (maths) | See below. |
| `a_sun_sets_the_sky_lighting_from_noon_to_night` | Feature scenario, no GPU | See below. |
| `the_sky_draws_behind_geometry_and_follows_the_sun` | Feature (render), GPU readback | See below. |
| `sky_shaders_compose_for_webgl2` | Invariant | The sky-view and sky shaders compose and validate with WebGL2 defs, with and without `SKY_RADIANCE_SQRT`. |

**New test in `crates/editor`:**

| Test | Kind | Asserts |
|---|---|---|
| `the_viewport_mirrors_the_scene_cameras_sky_and_environment` | Feature scenario, through the edit signals | An open scene with a camera holding `Sky` and `CameraEnvironment`: after a frame, the editor camera holds equal values. A property edit to the scene camera's `Sky` reaches the editor camera. Removing the component from the scene camera removes it from the editor camera. With no source, the editor camera holds neither. |

- **`transmittance_and_scattering_behave_physically`:**
  - Transmittance is 1 at zero path length and falls monotonically toward the
    horizon.
  - At the horizon, transmittance is lower for blue than for red.
  - Transmittance is 0 through the planet.
  - At noon, single scattering is bluer at the zenith than near the sun.
  - Every value is non-negative.
- **`a_sun_sets_the_sky_lighting_from_noon_to_night`.** The `App` has
  `MainSchedulePlugin`, `TransformPlugin` and `SkyPlugin`, a camera with
  `Sky` whose `sun` names a light spawned with only `Sun`.
  - A `Sky` with an unset `sun` gets zero irradiance.
  - The light is directional.
  - At noon, `CameraSkyLight`'s upward irradiance is bluer than its downward
    one, and `LightTint` is near white.
  - At 2° elevation, `LightTint` is redder (r > g > b) and dimmer.
  - At −10° (past twilight), `LightTint` is zero and the irradiance is near zero.
  - Editing `Sky::night_ambient` updates `CameraSkyLight`.
  - Removing `Sun` removes `LightTint` and makes the irradiance zero.
  - Removing `Sky` removes `CameraSkyLight`.
  - A spot light with `Sun` is ignored.
- **`the_sky_draws_behind_geometry_and_follows_the_sun`.** Render a camera with
  `Sky` and one cube at noon.
  - Pixels above the cube are blue-dominant.
  - The cube's pixels are not sky colored.
  - The pixel where the sun projects is near white.
  - With the sun at 1° elevation, horizon pixels toward the sun are
    red-dominant.
  - With the sun below the horizon, sky pixels are dark.
  - Without an adapter, the test skips with a `SKIPPED` line.

**Changes in `crates/render`:**

| Test | Kind | Asserts |
|---|---|---|
| `spherical_harmonics_reproduce_irradiance` (new, proptest; `proptest` becomes a render dev-dependency) | Algorithm (law) | For a constant radiance `L`, projecting and evaluating gives `π L` for every normal. For a random field in bands 0–2, evaluation matches the analytic cosine-convolved irradiance within tolerance. |
| `an_environment_fills_the_uniform_with_its_fog_and_ambient_and_none_restores_the_default` (existing CPU table, one more row) | Invariant | A camera with `CameraSkyLight` writes its coefficients, night terms and the sky flag. One without keeps the flat ambient. |
| `camera_uniform_keeps_its_wgsl_layout` (existing, updated) | Invariant | The new size and field offsets match the WGSL struct. |
| `webgl2_shaders.rs` (existing, updated) | Invariant | Builds `RenderCapabilities` with the new field. Composes the standard shader with a second def set that includes `SKY_RADIANCE_SQRT`. |

Budget: `crates/skybox` gets a new entry at 4, `crates/render` goes from
79 to 80, and `crates/editor` from 176 to 177. The added tests are the sky
maths, the scenario, the readback and the WebGL2 check in skybox, the SH law
in render, and the mirror scenario in the editor. The other changes edit
existing tests.

The red step comes first in each task: write the scenario, watch it fail on the
missing components, then implement.

## Visual checks (owner, on the local Sponza scenes)

Use the `visual-verification` skill. Setup in render-test's `main.gasset`,
after copying it to `main.gasset.bak`: its own `Sun` light becomes a
directional light marked `Sun` (Sponza's imported `SUN` is a read-only linked
node with intensity 0), and a new camera in the level gets a `Sky` whose `sun`
names it. Then:

1. **Noon.** Blue sky above the atrium, a white sun disk, and shaded areas lit
   by a cool sky ambient instead of grey.
2. **Low sun (~5°).** Warm orange sunlight on the walls, a warm horizon glow
   toward the sun, deeper blue opposite.
3. **Sunset to night.** The scene darkens smoothly, with no pop as the sun
   crosses the horizon. Night stays readable through `night_ambient`.
4. **Fog on.** Distant fog is warm toward the sun and blue away from it.
5. **Turning the sun live.** While playing, turn the Sun with `set_property`
   on its rotation, or with the gizmo if Play allows it. The sky follows
   within a frame.
6. **Editor.** While editing (not playing), the viewport shows the sky, its
   ambient and the fog, and the sky and the sun's tint follow as the Sun
   turns.
7. **Web build** (`wasm-run` skill). The sky matches native. If the
   `Rgba8Unorm` fallback is taken, it shows no visible banding.

## Documentation

- **`crates/skybox/README.md` (new).** What `Sun` and `Sky` do, the spawn
  recipe, the agent recipe and the units.
- **`crates/editor/README.md`.** The viewport mirrors the scene camera's
  `Sky` and `CameraEnvironment`.
- **`docs/book/06-rendering.md`.**
  - Lines 218–227, which name `SkyboxCube` and say `SkyboxPlugin` draws the
    skybox, are rewritten for `Sun` and `Sky`.
  - The `Light` section notes `LightTint`.
- **`docs/book/10-crate-map.md:25` and
  `.agents/reference/architecture.md:11`.** Update the `skybox` description.
- **`docs/scheduling.md:116`.** Lists the new `RenderSet::Background`.
- **`CameraEnvironment` doc comments.** State that under a sky, `ambient` and
  `Fog::color` are replaced by the sky's.
- **`.agents/reference/gotchas.md`.** Gains two entries:
  - The editor camera mirrors `Sky` and `CameraEnvironment` from the scene
    camera with the lowest `Entity` that has each. This amends the existing
    "Only the editor camera draws in an edit `App`" entry.
  - The `SkyRadianceMap` encoding is shared between the standard shader and
    the skybox shaders.

## Risks

- **CPU cost while animating the sun.** Every frame the sun turns,
  `update_sun_lighting` runs about 4k LUT-sampled march steps, well under
  1 ms native. If wasm frames suffer, skip updates when the sun moved less
  than 0.1° since the last computed direction. That is a value comparison,
  not a counter.
- **WebGL2 float targets.** Rendering `Rgba16Float` on WebGL2 needs
  `EXT_color_buffer_float`. Almost every desktop browser has it; the sqrt
  `Rgba8Unorm` fallback covers the rest. Sampling `Rgba16Float` with filtering,
  which the LUTs need, is core WebGL2.
- **Brightness.** Today's scenes tune `Light::intensity` against flat ambient.
  Marking a light `Sun` changes its color with the time of day. Adding `Sky`
  changes the ambient and fog. A scene with neither looks as it does today.

## Decisions confirmed

1. **The atmosphere's medium lives on `Sun`, not on `Sky`.** You chose "Sky on
   the camera" when its settings were described as the atmosphere's. Lights are
   one global uniform, so the sun's tint can't differ per camera; if each
   camera's `Sky` held its own medium, two cameras could disagree about the
   sun's color.
   - This spec puts the medium on `Sun`.
   - `Sky` keeps what is per view: that the camera draws the sky, and the
     night terms.
2. **No cube-map mode in this change.** You chose to keep a cube-map mode
   with import as follow-up. `Texture` is always single-layer, so no texture
   can be a cube; the variant could only warn, and it would need a dummy cube
   binding.
   - This spec deletes `SkyboxMaterial`.
   - Adding `Sky` cube maps goes in the follow-up (#866, reworded).
3. **The sky shows while editing too.** The owner chose the edit-time mirror
   over Play-only. They also chose to mirror `CameraEnvironment` (closing
   #886), to take each component from the lowest-`Entity` scene camera that
   has it, and to put the mirror in the editor crate. See *Viewport
   environment mirror*.

Decisions 1 and 2 were confirmed as written.

4. **`Sky` names its sun** (owner, 2026-11-19, after the first build). The
   level gets a camera of its own with a `Sky` whose `sun` is a
   `SceneEntityRef` to the `Sun`, replacing the implicit "first `Sun` by
   entity index" rule. One sun is active per frame: the one named by the
   lowest-index `Sky` camera. The editor mirror keeps its lowest-index rule.

## Follow-up work

- **Cube-map skies: #866, reworded.**
  - `Texture` with six layers and a cube dummy.
  - An equirectangular `.hdr` importer that cooks a cube map.
  - A cube-map source for `Sky`.
- **HDR target: #427.** An HDR camera target with exposure and one
  tonemapping pass, replacing per-material ACES. When it lands, the sky
  shader drops its own ACES and the `Rgba8Unorm` fallback can be reconsidered.
- **Moonlight: #655.** A moon disk, and a second light that follows it, build
  on this sky.
- **Specular reflections.** Reflections from the sky through a prefiltered
  environment map. No issue yet; it gets one when the owner wants it.

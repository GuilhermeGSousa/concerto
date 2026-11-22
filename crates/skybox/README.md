# concerto-skybox

A physically based sky for Concerto, after Hillaire's 2020 atmosphere: a sun
that sets the time of day, a sky drawn behind the scene, and the light that sky
casts. `DefaultPlugins` registers `SkyboxPlugin` after `RenderPlugin`,
re-exported as `concerto::skybox`.

## Sun and Sky

- **`Sun`** marks the directional light that is the sun. The sun sits where
  the light comes from, `rotation * Vec3::Z`, so turning the light moves it
  across the sky through dawn, noon, sunset and night. `Sun` requires a
  directional `Light`; a `Light` you give yourself wins, and a `Sun` on a
  point or spot light is ignored with a warning. Every directional `Sun`
  tints its own light from its own atmosphere.
- **`Sky`** on a camera draws the sky behind everything that camera sees and
  lights the scene with it. It requires `Camera`. Its `sun` is a
  `SceneEntityRef` naming the directional light with `Sun` it follows; in the
  editor, the entity picker sets it. One sun is lit per frame: the one named
  by the `Sky` camera with the lowest entity index. A `Sky` naming another sun
  shows that one and warns.

```rust
use concerto::{director::MainCamera, ecs::component::scene::SceneEntityRef, skybox::{Sky, Sun}};

let sun = cmd.spawn((
    Sun::default(),
    Light::directional_light().with_intensity(2.0),
    Transform::from_rotation(Quat::from_rotation_x(-0.8)),
)).entity();
cmd.spawn((
    MainCamera,
    Sky::default().with_sun(SceneEntityRef::Entity(sun)),
    Transform::from_translation(Vec3::new(0.0, 1.7, 5.0)),
));
```

What each does:

| With | Happens |
|---|---|
| `Sun` | Its light's color is multiplied by a `LightTint`, the light left after crossing the atmosphere: near white at noon, warmer and dimmer as the sun sets, zero below the horizon. This holds whether or not any camera has a `Sky`. |
| `Sky` | The camera draws the sky and the sun's disk behind the scene. Its ambient light comes from the sky, as spherical harmonics, instead of `CameraEnvironment::ambient`. The standard material reflects the sky from the radiance map, whose mip levels are the blur a rough surface shows. Its fog takes the sky's color in the direction you look instead of `Fog::color`. |
| `Sky` whose `sun` is unset or names no directional `Sun` | Night: only `night_color` and `night_ambient`, with a warning. |
| neither | The scene draws as it did before. |

| Field | Meaning | Default |
|---|---|---|
| `Sun::atmosphere` | the air the sunlight crosses, an `Atmosphere` | Earth |
| `Sun::disk_radius` | angular radius of the drawn sun disk, in degrees | 0.27 |
| `Sky::night_color` | radiance added to the sky and to sky-colored fog | dark blue-grey |
| `Sky::night_ambient` | light added to the ambient term, in the units of `CameraEnvironment::ambient` | dark blue-grey |

`Atmosphere` is in kilometres and per-kilometre coefficients: the planet's
radius and the atmosphere's height, ground albedo, Rayleigh scattering and
scale height, Mie scattering, absorption, scale height and asymmetry, and the
ozone layer's absorption, centre height and width. Its default is Hillaire's
and Bevy's Earth. Each of `Sun`, `Atmosphere` and `Sky` has a `with_<field>`
builder per field.

## Units

The sky is measured relative to the sun. The sun light's color times its
intensity is the sunlight's illuminance; the sky's radiance and its ambient
light are in the same units, so a brighter sun gives a brighter sky. Each
material and the sky apply the same ACES curve at the end, so there is no
exposure to set: tune `Light::intensity`, and the sky follows. The viewer
stands 200 m above the ground whatever the camera's height.

## How it runs

`SkyboxPlugin` registers two plugins:

- **`SkyPlugin`**, the main world, needs no GPU. When the sun's atmosphere
  changes it builds the transmittance and multiple-scattering tables on the
  CPU (`AtmosphereLuts`). When the sun, its light, its transform, the tables
  or a `Sky` change, it writes the sun's `LightTint` and each `Sky` camera's
  `CameraSkyLight`. It runs while editing as well as in a game.
- **`SkyRenderPlugin`**, the render world, registered only when `RenderPlugin`
  was registered first; `SkyboxPlugin` warns when `RenderPlugin` came after
  it. It uploads the tables, renders the sky-radiance map
  (256×128, equirectangular) when the sun, its light or the atmosphere
  changed, and draws the sky into each `Sky` camera's viewport after the
  camera is cleared and before the scene. The map is the
  `SkyRadianceMap` that sky-colored fog samples.

Everything is fragment shaders and 2D textures, so the sky runs on WebGL2.
Where `Rgba16Float` cannot be rendered to, the map is `Rgba8Unorm` holding
`sqrt(radiance / SKY_RADIANCE_SCALE)`.

## Agents

`Sun`, `Atmosphere` and `Sky` are reflected scene components, so the
inspector edits them and the editor's MCP tools reach every setting:

1. `add_component` `Sun` on a directional light.
2. `add_component` `Sky` on the level's `MainCamera`, and `set_property` its
   `sun` to that light's entity id.
3. `set_property` on the light's `Transform` rotation to move the sun, or on
   `Sun`'s `atmosphere` and `Sky`'s night terms to tune them.

The viewport shows the result while editing, because the editor camera mirrors
the `MainCamera`'s `Sky` and `CameraEnvironment`, and the game shows it in
Play through that same camera.

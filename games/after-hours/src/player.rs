//! The night-shift closer: a first-person capsule with a flashlight.
use concerto::{
    audio::Audio,
    color::Color,
    director::MainCamera,
    ecs::{CommandQueue, Component, Entity, Query, Res, ResMut, Resource, With, Without},
    foundation::{time::Time, transform::Transform},
    physics::{
        collider::{Collider, ColliderOffset},
        rigid_body::{AllowedDofs, MotionType, RigidBody},
    },
    render::components::{
        camera::{Camera, Fog},
        light::Light,
    },
    window::input::{Input, KeyCode, PhysicalKey},
};
use glam::{Quat, Vec2, Vec3};

use crate::{
    body::{Mover, flat},
    game::{Game, Rand},
    lighting,
    sfx::{Sfx, Sounds},
    store::StoreEntity,
};

pub const EYE_HEIGHT: f32 = 1.62;
pub const RADIUS: f32 = 0.3;
const WALK_SPEED: f32 = 3.0;
const SPRINT_SPEED: f32 = 5.4;
const STAMINA_SECONDS: f32 = 4.0;
/// Full battery lasts this long with the light on.
const BATTERY_SECONDS: f32 = 150.0;
/// Below this charge the light starts to stutter.
const LOW_BATTERY: f32 = 0.22;
pub const FLASHLIGHT_CONE: f32 = 0.42;
pub const FLASHLIGHT_RANGE: f32 = 16.0;
/// Light at the brightest battery level.
pub const FLASHLIGHT_INTENSITY: f32 = 26.0;
/// Where the torch sits relative to the eye (held low and to the right).
pub const FLASHLIGHT_OFFSET: Vec3 = Vec3::new(0.18, -0.22, -0.1);
pub const FOV_Y: f32 = 1.2;
/// Darkness swallowing the far end of every aisle.
pub const FOG: Fog = Fog {
    color: Color::BLACK,
    density: 0.085,
    start: 4.0,
};
/// Ambient light: next to nothing, so unlit means unseen. The observation
/// rule counts it (see `lighting::AMBIENT_IRRADIANCE`).
pub const AMBIENT: f32 = lighting::AMBIENT_LIGHT;
/// Largest mouse movement, in pixels per axis, accepted in one frame.
const MAX_LOOK_STEP: f32 = 250.0;

#[derive(Resource, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Radians per pixel of mouse movement.
    pub sensitivity: f32,
    pub invert_y: bool,
    /// Master volume, 0..=1.
    pub volume: f32,
}

const DEFAULT_SENSITIVITY: f32 = 0.0022;

impl Default for Settings {
    fn default() -> Self {
        Self {
            sensitivity: DEFAULT_SENSITIVITY,
            invert_y: false,
            volume: 0.8,
        }
    }
}

impl Settings {
    /// Saved settings, or the defaults.
    pub fn load() -> Self {
        let mut settings = Self::default();
        let get = |key: &str| crate::platform::load(key).and_then(|v| v.parse::<f32>().ok());
        if let Some(v) = get("sensitivity") {
            settings.sensitivity = v.clamp(0.0003, 0.012);
        }
        if let Some(v) = get("volume") {
            settings.volume = v.clamp(0.0, 1.0);
        }
        settings.invert_y = crate::platform::load("invert-y").is_some_and(|v| v == "1");
        settings
    }

    fn save(&self) {
        crate::platform::save("sensitivity", &self.sensitivity.to_string());
        crate::platform::save("volume", &self.volume.to_string());
        crate::platform::save("invert-y", if self.invert_y { "1" } else { "0" });
    }

    /// Sensitivity relative to the default, for display.
    pub fn sensitivity_scale(&self) -> f32 {
        self.sensitivity / DEFAULT_SENSITIVITY
    }
}

/// On the title and pause screens: `[` / `]` sensitivity, `-` / `=` volume,
/// `I` invert look. Saved as they change.
pub fn adjust_settings(
    mut settings: ResMut<Settings>,
    input: Res<Input>,
    game: Res<Game>,
    mut audio: ResMut<Audio>,
    mut sounds: ResMut<Sounds>,
) {
    let menu = game.paused || matches!(game.phase, crate::game::Phase::Title);
    let before = *settings;
    if menu {
        let pressed = |key| input.is_just_pressed(PhysicalKey::Code(key));
        if pressed(KeyCode::BracketRight) {
            settings.sensitivity = (settings.sensitivity * 1.15).min(0.012);
        }
        if pressed(KeyCode::BracketLeft) {
            settings.sensitivity = (settings.sensitivity / 1.15).max(0.0003);
        }
        if pressed(KeyCode::Equal) || pressed(KeyCode::NumpadAdd) {
            settings.volume = (settings.volume + 0.1).min(1.0);
        }
        if pressed(KeyCode::Minus) || pressed(KeyCode::NumpadSubtract) {
            settings.volume = (settings.volume - 0.1).max(0.0);
        }
        if pressed(KeyCode::KeyI) {
            settings.invert_y = !settings.invert_y;
        }
    }
    if *settings != before {
        settings.save();
        sounds.play(&mut audio, Sfx::Click, 0.5);
    }
    if (audio.master_volume() - settings.volume).abs() > f32::EPSILON {
        audio.set_master_volume(settings.volume);
    }
}

#[derive(Component)]
pub struct Player {
    pub yaw: f32,
    pub pitch: f32,
    pub stamina: f32,
    /// Stamina only refills after a moment of not sprinting.
    stamina_rest: f32,
    pub sprinting: bool,
    bob_phase: f32,
    bob_amount: f32,
    pub head: Entity,
    pub keys: usize,
}

#[derive(Component)]
pub struct Head;

#[derive(Component)]
pub struct Flashlight {
    pub switched_on: bool,
    /// 0..=1.
    pub battery: f32,
    /// Whether light is actually coming out this frame (off during flicker).
    pub emitting: bool,
    flicker_timer: f32,
    flicker_off: f32,
    /// The intensity actually emitted this frame.
    pub intensity: f32,
}

impl Flashlight {
    pub fn is_emitting(&self) -> bool {
        self.emitting
    }
}

/// The eye this frame and last frame, for the observation test. Rendering
/// uses exactly this pose (see `place_camera`), and the previous one covers
/// anything still on screen from the frame before.
#[derive(Resource, Default, Clone, Copy)]
pub struct Eye {
    pub position: Vec3,
    pub rotation: Quat,
    pub aspect: f32,
    pub previous_position: Vec3,
    pub previous_rotation: Quat,
    pub valid: bool,
}

impl Eye {
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }
}

pub fn spawn_player(cmd: &mut CommandQueue, feet: Vec3, yaw: f32) -> Entity {
    let collider = Collider::capsule(EYE_HEIGHT * 0.5 - RADIUS, RADIUS);
    let offset = ColliderOffset::bottom_origin(&collider);
    let head = cmd
        .spawn((
            StoreEntity,
            Head,
            Transform::from_translation(Vec3::Y * EYE_HEIGHT),
        ))
        .entity();
    let flashlight = cmd
        .spawn((
            Flashlight {
                switched_on: true,
                battery: 1.0,
                emitting: true,
                flicker_timer: 0.0,
                flicker_off: 0.0,
                intensity: 0.0,
            },
            {
                let light = Light::spot_light(FLASHLIGHT_CONE)
                    .with_color(Color::srgba(1.0, 0.93, 0.8, 1.0))
                    .with_intensity(0.0)
                    .with_range(FLASHLIGHT_RANGE);
                if crate::platform::debug_flag("noshadow") {
                    light
                } else {
                    light.with_shadows()
                }
            },
            // Held low and to the right, like a real torch; shadows then
            // fall visibly behind things instead of hiding right behind them.
            Transform::from_translation(FLASHLIGHT_OFFSET),
        ))
        .entity();
    cmd.add_child(head, flashlight);

    let root = cmd
        .spawn((
            StoreEntity,
            Player {
                yaw,
                pitch: 0.0,
                stamina: STAMINA_SECONDS,
                stamina_rest: 0.0,
                sprinting: false,
                bob_phase: 0.0,
                bob_amount: 0.0,
                head,
                keys: 0,
            },
            Mover::new(12.0),
            RigidBody {
                density: 1000.0,
                allowed_dofs: AllowedDofs::TRANSLATION,
                motion_type: MotionType::Dynamic,
            },
            collider,
            offset,
            Transform::from_translation(feet),
        ))
        .entity();
    cmd.add_child(root, head);
    root
}

fn key_held(input: &Input, key: KeyCode) -> bool {
    input.is_held(PhysicalKey::Code(key))
}

pub fn control_player(
    players: Query<(&mut Player, &mut Mover)>,
    flashlights: Query<&mut Flashlight>,
    input: Res<Input>,
    settings: Res<Settings>,
    game: Res<Game>,
    time: Res<Time>,
    mut audio: ResMut<Audio>,
    mut sounds: ResMut<Sounds>,
) {
    let dt = time.delta().as_secs_f32();
    for (mut player, mut mover) in players.iter() {
        if !game.is_live() {
            mover.desired = Vec3::ZERO;
            player.sprinting = false;
            continue;
        }

        // Browsers can deliver one wild jump as the pointer lock engages;
        // no real flick moves this far in a single frame.
        let delta = input
            .mouse_delta()
            .clamp(Vec2::splat(-MAX_LOOK_STEP), Vec2::splat(MAX_LOOK_STEP));
        let invert = if settings.invert_y { -1.0 } else { 1.0 };
        player.yaw -= delta.x * settings.sensitivity;
        player.pitch = (player.pitch - delta.y * settings.sensitivity * invert).clamp(-1.35, 1.35);

        let mut wish = Vec3::ZERO;
        let forward = Vec3::new(-player.yaw.sin(), 0.0, -player.yaw.cos());
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        if key_held(&input, KeyCode::KeyW) || key_held(&input, KeyCode::ArrowUp) {
            wish += forward;
        }
        if key_held(&input, KeyCode::KeyS) || key_held(&input, KeyCode::ArrowDown) {
            wish -= forward;
        }
        if key_held(&input, KeyCode::KeyD) || key_held(&input, KeyCode::ArrowRight) {
            wish += right;
        }
        if key_held(&input, KeyCode::KeyA) || key_held(&input, KeyCode::ArrowLeft) {
            wish -= right;
        }
        let moving = wish.length_squared() > 0.0;
        let wants_sprint =
            key_held(&input, KeyCode::ShiftLeft) || key_held(&input, KeyCode::ShiftRight);
        player.sprinting = moving && wants_sprint && player.stamina > 0.0;
        if player.sprinting {
            player.stamina = (player.stamina - dt).max(0.0);
            player.stamina_rest = 1.0;
        } else {
            player.stamina_rest = (player.stamina_rest - dt).max(0.0);
            if player.stamina_rest <= 0.0 {
                player.stamina = (player.stamina + dt * 0.8).min(STAMINA_SECONDS);
            }
        }
        let speed = if player.sprinting {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };
        mover.desired = wish.normalize_or_zero() * speed;

        // Head bob and footsteps, driven by actual speed.
        let actual = flat(mover.velocity()).length();
        let target_bob = (actual / WALK_SPEED).min(1.6);
        player.bob_amount += (target_bob - player.bob_amount) * (1.0 - (-8.0 * dt).exp());
        let before = player.bob_phase;
        player.bob_phase += dt * actual * 2.1;
        if actual > 0.5
            && (before / std::f32::consts::PI).floor()
                != (player.bob_phase / std::f32::consts::PI).floor()
        {
            sounds.play(
                &mut audio,
                Sfx::Footstep,
                if player.sprinting { 0.55 } else { 0.35 },
            );
        }

        if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyF)) {
            for mut light in flashlights.iter() {
                if light.battery > 0.0 {
                    light.switched_on = !light.switched_on;
                    sounds.play(&mut audio, Sfx::FlashlightClick, 0.6);
                }
            }
        }
    }
}

/// Stamina as 0..=1, for the HUD.
pub fn stamina_fraction(player: &Player) -> f32 {
    player.stamina / STAMINA_SECONDS
}

pub fn update_flashlight(
    flashlights: Query<(&mut Flashlight, &mut Light)>,
    game: Res<Game>,
    time: Res<Time>,
    mut rand: ResMut<Rand>,
    mut audio: ResMut<Audio>,
    mut sounds: ResMut<Sounds>,
) {
    let dt = time.delta().as_secs_f32();
    for (mut flashlight, mut light) in flashlights.iter() {
        if game.is_live() && flashlight.switched_on {
            let before = flashlight.battery;
            flashlight.battery = (flashlight.battery - dt / BATTERY_SECONDS).max(0.0);
            if before > 0.0 && flashlight.battery <= 0.0 {
                flashlight.switched_on = false;
                sounds.play(&mut audio, Sfx::FlashlightDie, 0.8);
            }
        }

        // Low battery: now and then the beam cuts out for a moment. Those
        // moments are darkness, and the mannequins know it.
        flashlight.flicker_off = (flashlight.flicker_off - dt).max(0.0);
        if flashlight.switched_on && flashlight.battery < LOW_BATTERY && game.is_live() {
            flashlight.flicker_timer -= dt;
            if flashlight.flicker_timer <= 0.0 {
                let severity = 1.0 - flashlight.battery / LOW_BATTERY;
                flashlight.flicker_off = rand.range(0.05, 0.12 + 0.45 * severity);
                flashlight.flicker_timer = rand.range(0.4, 4.0 - 3.0 * severity);
                sounds.play(&mut audio, Sfx::Flicker, 0.25);
            }
        }

        flashlight.emitting =
            flashlight.switched_on && flashlight.battery > 0.0 && flashlight.flicker_off <= 0.0;
        // Dims as the battery drains.
        let strength = 0.45 + 0.55 * (flashlight.battery / 0.5).min(1.0);
        flashlight.intensity = if flashlight.emitting {
            FLASHLIGHT_INTENSITY * strength
        } else {
            0.0
        };
        light.intensity = flashlight.intensity;
    }
}

/// Places the head and writes the main camera's pose directly (instead of
/// through the director) so the frame shows exactly the pose `Eye` records.
pub fn place_camera(
    players: Query<(&Player, &Transform), Without<Head>>,
    heads: Query<&mut Transform, With<Head>>,
    main_cameras: Query<
        (&mut Transform, &mut Camera),
        (With<MainCamera>, Without<Head>, Without<Player>),
    >,
    mut eye: ResMut<Eye>,
    shake: Res<crate::scare::CameraOverride>,
) {
    let Some((player, body)) = players.iter().next() else {
        return;
    };
    let bob = Vec3::new(
        (player.bob_phase * 0.5).sin() * 0.03,
        -(player.bob_phase.sin().abs()) * 0.045,
        0.0,
    ) * player.bob_amount;
    let rotation = Quat::from_rotation_y(player.yaw) * Quat::from_rotation_x(player.pitch);
    let local = Vec3::Y * EYE_HEIGHT + rotation * Vec3::new(bob.x, 0.0, 0.0) + Vec3::Y * bob.y;
    if let Some(mut head) = heads.get_entity(player.head) {
        head.translation = local;
        head.rotation = rotation;
    }

    let (position, rotation) = shake.apply(body.translation + local, rotation);
    for (mut transform, mut camera) in main_cameras.iter() {
        transform.translation = position;
        transform.rotation = rotation;
        camera.fovy = FOV_Y;
        camera.znear = 0.05;
        camera.zfar = 80.0;
        camera.clear_color = Color::BLACK;
        camera.fog = Some(FOG);
        camera.ambient = Some(Color::rgba(AMBIENT, AMBIENT, AMBIENT, 1.0));
        eye.aspect = camera.aspect;
    }
    if eye.valid {
        eye.previous_position = eye.position;
        eye.previous_rotation = eye.rotation;
    } else {
        eye.previous_position = position;
        eye.previous_rotation = rotation;
        eye.valid = true;
    }
    eye.position = position;
    eye.rotation = rotation;
}

use concerto_ecs::{CommandQueue, Component, Entity, Query, Res, ResMut, Without};
use concerto_foundation::{time::Time, transform::Transform};
use concerto_physics::{
    body::BodyId,
    carried::Carried,
    collider::{Collider, ColliderOffset, GravityScale},
    layers::CollisionLayers,
    physics_state::PhysicsState,
    query::QueryFilter,
    rigid_body::{AllowedDofs, MotionType, RigidBody},
};
use concerto_window::input::{Input, PlayerIndex, Players, actions::ActionMap};
use glam::{Quat, Vec2, Vec3};

use crate::actions::{Look, Move, Stroll};

const GROUND_PROBE_SCALE: f32 = 0.9;
const GROUND_FOLLOW_RATE: f32 = 15.0;
const GRAVITY: f32 = 9.81;

/// A first-person character with no jump: a capsule held `step_height` above the ground, so it walks up steps that high and stops at anything taller.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct FirstPersonWalker {
    /// Metres per second with the `Move` stick pushed to `hurry_threshold`.
    pub walk_speed: f32,
    /// Metres per second with the `Move` stick pushed all the way.
    pub hurry_speed: f32,
    /// The `Move` deflection, in `0..1`, up to which the walker walks; past it the speed rises to `hurry_speed`.
    pub hurry_threshold: f32,
    /// The tallest step the walker climbs, in metres.
    pub step_height: f32,
    /// The steepest ground it stands on, in radians from level.
    pub max_slope_angle: f32,
    /// The capsule's radius.
    pub radius: f32,
    /// From the feet to the top of the capsule.
    pub height: f32,
    /// From the feet to the eyes, where a `FirstPersonView` sits.
    pub eye_height: f32,
    /// How far ahead of the body's centre the eyes are, so a view looks out past a body drawn on the walker; keep it inside `radius`.
    pub eye_forward: f32,
    /// Radians per second the view turns with the `Look` stick pushed all the way.
    pub look_rate: f32,
    /// Radians the view turns per unit of mouse motion.
    pub mouse_sensitivity: f32,
    /// How far the view pitches up or down, in radians.
    pub pitch_limit: f32,
    /// Pushing the `Look` stick up looks down.
    pub invert_y: bool,
    /// Whether the player's `Move` and `Look` drive it; while false it stands still and keeps its view, as when a menu is open.
    pub controlled: bool,
    yaw: f32,
    pitch: f32,
    push: Vec2,
    grounded: bool,
}

impl Default for FirstPersonWalker {
    fn default() -> Self {
        Self {
            walk_speed: 1.6,
            hurry_speed: 3.4,
            hurry_threshold: 0.7,
            step_height: 0.35,
            max_slope_angle: 50f32.to_radians(),
            radius: 0.3,
            height: 1.8,
            eye_height: 1.65,
            eye_forward: 0.0,
            look_rate: 2.5,
            mouse_sensitivity: 0.002,
            pitch_limit: 80f32.to_radians(),
            invert_y: false,
            controlled: true,
            yaw: 0.0,
            pitch: 0.0,
            push: Vec2::ZERO,
            grounded: false,
        }
    }
}

impl FirstPersonWalker {
    pub fn with_walk_speed(mut self, walk_speed: f32) -> Self {
        self.walk_speed = walk_speed;
        self
    }

    pub fn with_hurry_speed(mut self, hurry_speed: f32) -> Self {
        self.hurry_speed = hurry_speed;
        self
    }

    pub fn with_hurry_threshold(mut self, hurry_threshold: f32) -> Self {
        self.hurry_threshold = hurry_threshold;
        self
    }

    pub fn with_step_height(mut self, step_height: f32) -> Self {
        self.step_height = step_height;
        self
    }

    pub fn with_max_slope_angle(mut self, max_slope_angle: f32) -> Self {
        self.max_slope_angle = max_slope_angle;
        self
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn with_eye_height(mut self, eye_height: f32) -> Self {
        self.eye_height = eye_height;
        self
    }

    pub fn with_eye_forward(mut self, eye_forward: f32) -> Self {
        self.eye_forward = eye_forward;
        self
    }

    pub fn with_look_rate(mut self, look_rate: f32) -> Self {
        self.look_rate = look_rate;
        self
    }

    pub fn with_mouse_sensitivity(mut self, mouse_sensitivity: f32) -> Self {
        self.mouse_sensitivity = mouse_sensitivity;
        self
    }

    pub fn with_pitch_limit(mut self, pitch_limit: f32) -> Self {
        self.pitch_limit = pitch_limit;
        self
    }

    pub fn with_invert_y(mut self, invert_y: bool) -> Self {
        self.invert_y = invert_y;
        self
    }

    /// Faces `yaw` radians about +Y, where 0 looks along -Z and a positive yaw turns left.
    pub fn with_yaw(mut self, yaw: f32) -> Self {
        self.yaw = yaw;
        self
    }

    /// Sets the push the walker moves by until the next call: x to its right, y forward, of length at most 1.
    pub fn steer(&mut self, push: Vec2) {
        self.push = push.clamp_length_max(1.0);
    }

    /// Turns the view left by `yaw` and up by `pitch`, in radians, keeping the pitch within `pitch_limit`.
    pub fn look(&mut self, yaw: f32, pitch: f32) {
        self.yaw += yaw;
        self.pitch = (self.pitch + pitch).clamp(-self.pitch_limit, self.pitch_limit);
    }

    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Whether the last fixed step found ground it can stand on within reach of its feet.
    pub fn is_grounded(&self) -> bool {
        self.grounded
    }

    /// The way the eyes look.
    pub fn view_rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    /// The speed a push of `deflection` asks for: up to `walk_speed` at `hurry_threshold`, then up to `hurry_speed` at 1.
    pub fn speed_for(&self, deflection: f32) -> f32 {
        let deflection = deflection.clamp(0.0, 1.0);
        let threshold = self.hurry_threshold.clamp(0.0, 1.0);
        if deflection <= threshold {
            if threshold <= 0.0 {
                return self.walk_speed;
            }
            return self.walk_speed * deflection / threshold;
        }
        let over = (deflection - threshold) / (1.0 - threshold);
        self.walk_speed + (self.hurry_speed - self.walk_speed) * over
    }

    /// The horizontal velocity the current push asks for, in world space.
    pub fn desired_velocity(&self) -> Vec3 {
        let deflection = self.push.length();
        if deflection <= f32::EPSILON {
            return Vec3::ZERO;
        }
        let direction = Vec3::new(self.push.x, 0.0, -self.push.y) / deflection;
        Quat::from_rotation_y(self.yaw) * direction * self.speed_for(deflection)
    }

    fn capsule_half_height(&self) -> f32 {
        ((self.height - self.step_height) / 2.0 - self.radius).max(0.0)
    }
}

/// Puts the entity at the eyes of `walker`, looking where it looks; give it a `Camera` or a `VirtualCamera`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct FirstPersonView {
    pub walker: Entity,
}

/// Spawns `walker` with its body, standing with its feet at `transform`, driven by `player`'s `Move` and `Look`.
pub fn spawn_first_person_walker(
    cmd: &mut CommandQueue,
    player: PlayerIndex,
    transform: Transform,
    walker: FirstPersonWalker,
) -> Entity {
    let collider = Collider::capsule(walker.capsule_half_height(), walker.radius);
    let offset = ColliderOffset(Vec3::Y * (collider.bottom_offset() + walker.step_height));
    cmd.spawn((
        player,
        RigidBody {
            density: 1000.0,
            allowed_dofs: AllowedDofs::TRANSLATION,
            motion_type: MotionType::Dynamic,
        },
        collider,
        offset,
        GravityScale(0.0),
        CollisionLayers::character(),
        transform,
        walker,
    ))
    .entity()
}

/// Steers and turns each walker from its player's `Move` and `Look` actions and, for the player who holds it, the mouse.
pub fn read_first_person_input(
    mut walkers: Query<(&PlayerIndex, &mut FirstPersonWalker)>,
    input: Res<Input>,
    actions: Res<ActionMap>,
    players: Res<Players>,
    time: Res<Time>,
) {
    let dt = time.delta().as_secs_f32();
    for (player, mut walker) in walkers.iter_mut() {
        if !walker.controlled {
            if walker.push != Vec2::ZERO {
                walker.steer(Vec2::ZERO);
            }
            continue;
        }
        let actions = actions.player(*player, &input, &players);
        let push = actions.axis2d(Move);
        let push = if actions.is_held(Stroll) {
            push.clamp_length_max(walker.hurry_threshold)
        } else {
            push
        };
        if walker.push != push {
            walker.steer(push);
        }
        let stick = actions.axis2d(Look) * walker.look_rate * dt;
        let mouse = actions.mouse_delta() * walker.mouse_sensitivity;
        let up = if walker.invert_y { -1.0 } else { 1.0 };
        let yaw = -stick.x - mouse.x;
        let pitch = (stick.y - mouse.y) * up;
        if yaw != 0.0 || pitch != 0.0 {
            walker.look(yaw, pitch);
        }
    }
}

/// Sets each walker's body velocity for the fixed step: its push along the ground, and whatever holds its feet on the ground under it. A `Carried` walker is left alone.
pub fn step_first_person_walkers(
    mut walkers: Query<(&mut FirstPersonWalker, &BodyId), Without<Carried>>,
    mut physics: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (mut walker, body) in walkers.iter_mut() {
        let feet = physics.body_transform(*body).translation;
        let probe_radius = walker.radius * GROUND_PROBE_SCALE;
        let probe_from = feet + Vec3::Y * (walker.step_height + probe_radius);
        let reach = walker.step_height * 2.0;
        let filter = QueryFilter::ignoring(std::slice::from_ref(body));
        let ground = physics
            .cast_sphere(probe_from, probe_radius, Vec3::NEG_Y * reach, filter)
            .filter(|hit| hit.normal.y >= walker.max_slope_angle.cos())
            .map(|hit| walker.step_height - hit.fraction * reach);
        let vertical = match ground {
            Some(rise) => {
                let limit = rise.abs() / dt;
                (rise * GROUND_FOLLOW_RATE).clamp(-limit, limit)
            }
            None => physics.linear_velocity(*body).y - GRAVITY * dt,
        };
        if walker.grounded != ground.is_some() {
            walker.grounded = ground.is_some();
        }
        physics.set_linear_velocity(*body, walker.desired_velocity().with_y(vertical));
    }
}

/// Moves each `FirstPersonView` to its walker's eyes and turns it the way the walker looks.
pub fn place_first_person_views(
    mut views: Query<(&FirstPersonView, &mut Transform)>,
    walkers: Query<(&FirstPersonWalker, &Transform), Without<FirstPersonView>>,
) {
    for (view, mut transform) in views.iter_mut() {
        let Some((walker, body)) = walkers.get_entity(view.walker) else {
            continue;
        };
        let ahead = Quat::from_rotation_y(walker.yaw()) * Vec3::NEG_Z * walker.eye_forward;
        transform.translation = body.translation + Vec3::Y * walker.eye_height + ahead;
        transform.rotation = walker.view_rotation();
    }
}

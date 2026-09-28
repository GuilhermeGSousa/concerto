//! Getting caught: the camera is wrenched toward the mannequin as it lunges,
//! the flashlight stutters, a stinger hits, and the screen cuts to black.
use concerto::{
    animation::player::{AnimationHandleComponent, AnimationPlayer},
    audio::Audio,
    ecs::{CommandQueue, Entity, Query, Res, ResMut, Resource, Without},
    foundation::{time::Time, transform::Transform},
};
use glam::{Quat, Vec3};

use crate::{
    body::{flat, yaw_toward},
    game::{Game, Phase, Rand},
    mannequin::Mannequin,
    player::{Flashlight, Player},
    poses::PoseLibrary,
    sfx::{Sfx, Sounds},
};

/// How long the jumpscare runs before the death card.
pub const SCARE_SECONDS: f32 = 1.25;

#[derive(Resource, Default)]
pub struct Caught {
    pub active: bool,
    pub by: Option<Entity>,
    started: bool,
}

impl Caught {
    pub fn catch(&mut self, by: Entity) {
        self.active = true;
        self.by = Some(by);
        self.started = false;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Camera effects layered over the player's own view.
#[derive(Resource, Default)]
pub struct CameraOverride {
    look_at: Option<Vec3>,
    blend: f32,
    shake: f32,
    seed: u32,
}

impl CameraOverride {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn apply(&self, position: Vec3, rotation: Quat) -> (Vec3, Quat) {
        let mut rotation = rotation;
        if let Some(target) = self.look_at {
            let dir = (target - position).normalize_or_zero();
            if dir.length_squared() > 0.0 {
                let yaw = (-dir.x).atan2(-dir.z);
                let pitch = dir.y.clamp(-1.0, 1.0).asin();
                let look = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
                rotation = rotation.slerp(look, self.blend.clamp(0.0, 1.0));
            }
        }
        if self.shake > 0.0 {
            let s = self.seed as f32;
            let jitter = Vec3::new((s * 12.9898).sin(), (s * 78.233).sin(), (s * 37.719).sin());
            rotation *= Quat::from_euler(
                glam::EulerRot::XYZ,
                jitter.x * 0.04 * self.shake,
                jitter.y * 0.04 * self.shake,
                jitter.z * 0.03 * self.shake,
            );
        }
        (position, rotation)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_scare(
    mut caught: ResMut<Caught>,
    mut game: ResMut<Game>,
    mut camera: ResMut<CameraOverride>,
    players: Query<(&Transform, &Player), Without<Mannequin>>,
    mannequins: Query<(&mut Mannequin, &mut Transform), Without<Player>>,
    anim_players: Query<&mut AnimationPlayer>,
    flashlights: Query<&mut Flashlight>,
    library: Res<PoseLibrary>,
    (time, mut rand): (Res<Time>, ResMut<Rand>),
    (mut audio, mut sounds): (ResMut<Audio>, ResMut<Sounds>),
    mut cmd: CommandQueue,
) {
    let real_dt = time.real_delta().as_secs_f32();
    camera.seed = camera.seed.wrapping_add(1);
    if !caught.active || game.phase != Phase::Caught {
        camera.shake = (camera.shake - real_dt * 3.0).max(0.0);
        return;
    }
    let Some(by) = caught.by else {
        return;
    };
    let Some((player_transform, _)) = players.iter().next() else {
        return;
    };

    if !caught.started {
        caught.started = true;
        sounds.play(&mut audio, Sfx::Stinger, 1.0);
        audio.stop_music();
        if let Some((mut mannequin, mut transform)) = mannequins.get_entity(by) {
            let away =
                flat(transform.translation - player_transform.translation).normalize_or(Vec3::Z);
            transform.translation = player_transform.translation + away * 0.7;
            mannequin.yaw = yaw_toward(-away);
            mannequin.observed = false;
            if let (Some(anim), Some(lunge)) = (mannequin.anim_entity(), library.lunge.clone()) {
                cmd.insert(AnimationHandleComponent::new(lunge), anim);
                if let Some(mut player) = anim_players.get_entity(anim) {
                    player.set_paused(false);
                }
            }
        }
    }

    let t = game.phase_time;
    if let Some((_, transform)) = mannequins.get_entity(by) {
        camera.look_at = Some(transform.translation + Vec3::Y * 1.55);
    }
    camera.blend = (t / 0.12).min(1.0);
    camera.shake = 1.0 - (t / SCARE_SECONDS) * 0.5;

    for mut flashlight in flashlights.iter() {
        flashlight.switched_on = rand.unit() > (t / SCARE_SECONDS).powf(0.5);
    }

    if t >= SCARE_SECONDS {
        caught.active = false;
        game.set_phase(Phase::Dead);
        game.deaths += 1;
        sounds.play(&mut audio, Sfx::DeathDrone, 0.8);
    }
}

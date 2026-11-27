use std::time::Duration;

use concerto_ecs::{
    Changed, CommandQueue, Component, Entity, Query, Res, ResMut, Without,
    events::{Event, event_reader::EventReader, event_writer::EventWriter},
    reflect::ReflectComponent,
    signal::{
        On,
        lifecycle::{Remove, Replace},
    },
};
use concerto_foundation::assets::{
    AssetId,
    asset_store::AssetStore,
    handle::{AssetHandle, AssetLoadFailed},
};
use concerto_reflect::Reflect;
use kira::{
    Easing, StartTime, Tween,
    sound::{
        PlaybackState, Region,
        static_sound::{StaticSoundData, StaticSoundHandle},
    },
};

use crate::mixer::{AudioBus, decibels};
use crate::{Audio, AudioClip};

pub(crate) const FADE: Tween = Tween {
    start_time: StartTime::Immediate,
    duration: Duration::from_millis(10),
    easing: Easing::Linear,
};

/// Plays `clip` on `bus` while the entity lives; settings changed afterwards apply to the playing sound.
#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct AudioPlayer {
    /// The clip played.
    pub clip: AssetHandle<AudioClip>,
    /// The mixer bus the sound plays on.
    pub bus: AudioBus,
    /// The linear volume, on the mixer's scale: 0 or less is silent, 1 the clip's own level.
    pub volume: f32,
    /// The playback rate, 1 for the clip's own speed; it also shifts the pitch.
    pub speed: f32,
    /// Whether the whole clip loops, so the sound never ends on its own.
    pub looping: bool,
    /// What happens to the entity when the sound finishes.
    pub end: PlaybackEnd,
}

impl AudioPlayer {
    /// Plays `clip` once on `Effects` at volume and speed 1, despawning the entity at the end.
    pub fn new(clip: AssetHandle<AudioClip>) -> Self {
        Self {
            clip,
            bus: AudioBus::default(),
            volume: 1.0,
            speed: 1.0,
            looping: false,
            end: PlaybackEnd::default(),
        }
    }

    /// Plays on `bus`.
    pub fn with_bus(mut self, bus: AudioBus) -> Self {
        self.bus = bus;
        self
    }

    /// Plays at linear `volume`, the mixer's scale.
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Plays at `speed`, which also shifts the pitch.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Loops the whole clip, so the sound never ends on its own.
    pub fn with_looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Does `end` when the sound finishes.
    pub fn with_end(mut self, end: PlaybackEnd) -> Self {
        self.end = end;
        self
    }

    fn sound(&self, clip: &StaticSoundData) -> StaticSoundData {
        clip.volume(decibels(self.volume))
            .playback_rate(f64::from(self.speed))
            .loop_region(self.loop_region())
    }

    fn loop_region(&self) -> Option<Region> {
        self.looping.then(|| Region::from(..))
    }
}

/// What happens to a player's entity when its sound finishes.
#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackEnd {
    /// Despawns the entity; the default.
    #[default]
    Despawn,
    /// Removes the [`AudioPlayer`] and keeps the entity.
    Remove,
    /// Keeps the entity and its player, silent until the player is inserted again.
    Keep,
}

/// A sound reached its end; sent once per play, before `end` acts.
#[derive(Event, Clone, Debug)]
pub struct SoundFinished {
    /// The entity whose [`AudioPlayer`] finished.
    pub entity: Entity,
}

#[derive(Component)]
pub(crate) struct PlayingSound {
    handle: StaticSoundHandle,
    clip: AssetId,
    bus: AudioBus,
}

#[derive(Component)]
pub(crate) struct SoundDone;

fn end(
    entity: Entity,
    end: PlaybackEnd,
    finished: &mut EventWriter<SoundFinished>,
    cmd: &mut CommandQueue,
) {
    finished.write(SoundFinished { entity });
    match end {
        PlaybackEnd::Despawn => cmd.despawn(entity),
        PlaybackEnd::Remove => cmd.remove::<AudioPlayer>(entity),
        PlaybackEnd::Keep => {
            cmd.remove::<PlayingSound>(entity);
            cmd.insert(SoundDone, entity);
        }
    }
}

pub(crate) fn start_sounds(
    players: Query<(Entity, &AudioPlayer), (Without<PlayingSound>, Without<SoundDone>)>,
    clips: Res<AssetStore<AudioClip>>,
    audio: Option<ResMut<Audio>>,
    mut finished: EventWriter<SoundFinished>,
    mut cmd: CommandQueue,
) {
    let Some(mut audio) = audio else {
        return;
    };
    for (entity, player) in players.iter() {
        let Some(clip) = clips.get(&player.clip).and_then(AudioClip::sound) else {
            continue;
        };
        match audio.play(player.bus, player.sound(clip)) {
            Ok(handle) => cmd.insert(
                PlayingSound {
                    handle,
                    clip: player.clip.id(),
                    bus: player.bus,
                },
                entity,
            ),
            Err(error) => {
                log::warn!("{entity:?} plays nothing: {error}");
                end(entity, player.end, &mut finished, &mut cmd);
            }
        }
    }
}

pub(crate) fn fail_sounds(
    mut failed: EventReader<AssetLoadFailed>,
    players: Query<(Entity, &AudioPlayer), (Without<PlayingSound>, Without<SoundDone>)>,
    mut finished: EventWriter<SoundFinished>,
    mut cmd: CommandQueue,
) {
    for id in failed.read().map(|event| event.id) {
        for (entity, player) in players.iter() {
            if player.clip.id() == id {
                log::warn!("{entity:?} plays nothing: its clip {id:?} failed to load");
                end(entity, player.end, &mut finished, &mut cmd);
            }
        }
    }
}

pub(crate) fn update_sounds(
    mut players: Query<(Entity, &AudioPlayer, &mut PlayingSound), Changed<AudioPlayer>>,
    audio: Option<ResMut<Audio>>,
    mut cmd: CommandQueue,
) {
    if audio.is_none() {
        return;
    }
    for (entity, player, mut playing) in players.iter_mut() {
        if player.clip.id() != playing.clip || player.bus != playing.bus {
            playing.handle.stop(FADE);
            cmd.remove::<PlayingSound>(entity);
            continue;
        }
        playing.handle.set_volume(decibels(player.volume), FADE);
        playing
            .handle
            .set_playback_rate(f64::from(player.speed), FADE);
        playing.handle.set_loop_region(player.loop_region());
    }
}

pub(crate) fn finish_sounds(
    players: Query<(Entity, &AudioPlayer, &PlayingSound)>,
    audio: Option<ResMut<Audio>>,
    mut finished: EventWriter<SoundFinished>,
    mut cmd: CommandQueue,
) {
    if audio.is_none() {
        return;
    }
    for (entity, player, playing) in players.iter() {
        if playing.handle.state() == PlaybackState::Stopped {
            end(entity, player.end, &mut finished, &mut cmd);
        }
    }
}

fn stop(
    entity: Entity,
    sounds: &mut Query<&mut PlayingSound>,
    done: &Query<&SoundDone>,
    cmd: &mut CommandQueue,
) {
    if let Some(mut playing) = sounds.get_entity_mut(entity) {
        playing.handle.stop(FADE);
        cmd.remove::<PlayingSound>(entity);
    }
    if done.get_entity(entity).is_some() {
        cmd.remove::<SoundDone>(entity);
    }
}

pub(crate) fn stop_removed_player(
    on: On<Remove<AudioPlayer>>,
    mut sounds: Query<&mut PlayingSound>,
    done: Query<&SoundDone>,
    mut cmd: CommandQueue,
) {
    stop(on.signal().entity, &mut sounds, &done, &mut cmd);
}

pub(crate) fn stop_replaced_player(
    on: On<Replace<AudioPlayer>>,
    mut sounds: Query<&mut PlayingSound>,
    done: Query<&SoundDone>,
    mut cmd: CommandQueue,
) {
    stop(on.signal().entity, &mut sounds, &done, &mut cmd);
}

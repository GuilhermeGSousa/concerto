use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;

use crate::mixer::apply_mixer;
use crate::player::{
    fail_sounds, finish_sounds, start_sounds, stop_removed_player, stop_replaced_player,
    update_sounds,
};
use crate::{
    Audio, AudioBackend, AudioBus, AudioClip, AudioMixer, AudioPlayer, PlaybackEnd, SoundFinished,
};

/// Plays `AudioPlayer`s through the `Audio` it opens on `finish`, or on the web at the first key or click, except in an edit `App`; needs an `AssetServer`.
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset::<AudioClip>()
            .register_type::<AudioPlayer>()
            .register_type::<AudioMixer>()
            .register_type::<AudioBus>()
            .register_type::<PlaybackEnd>()
            .register_event::<SoundFinished>();
        if app.get_resource::<AudioMixer>().is_none() {
            app.insert_resource(AudioMixer::default());
        }
        if app.get_resource::<AudioBackend>().is_none() {
            app.insert_resource(AudioBackend::default());
        }
        app.add_system(Update, apply_mixer)
            .add_system(
                Update,
                (start_sounds, fail_sounds, update_sounds, finish_sounds).chain(),
            )
            .add_listener(stop_removed_player)
            .add_listener(stop_replaced_player);
        #[cfg(target_arch = "wasm32")]
        app.add_system(
            concerto_app::schedule_groups::First,
            crate::web::open_audio_on_input,
        );
    }

    fn finish(&self, app: &mut App) {
        if cfg!(target_arch = "wasm32") || app.is_editing() {
            return;
        }
        let backend = app
            .get_resource::<AudioBackend>()
            .copied()
            .unwrap_or_default();
        let mixer = app
            .get_resource::<AudioMixer>()
            .copied()
            .unwrap_or_default();
        app.insert_resource(Audio::open(backend, &mixer));
    }
}

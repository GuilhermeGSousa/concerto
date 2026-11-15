use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
};
use concerto_audio::{
    Audio, AudioBackend, AudioBus, AudioClip, AudioPlayer, AudioPlugin, PlaybackEnd, SoundFinished,
};
use concerto_ecs::{
    Entity,
    events::{event_channel::EventChannel, event_cursor::EventCursor},
};
use concerto_foundation::assets::{
    AssetId, ContentAssetRoot, asset_server::AssetServer, handle::AssetHandle,
    registry::AssetRegistry,
};

use crate::clip::{fixture, import, write_content};

struct Playback {
    app: App,
    cursor: EventCursor<SoundFinished>,
}

impl Playback {
    fn new(app: App) -> Self {
        Self {
            app,
            cursor: EventCursor::default(),
        }
    }

    fn game() -> Self {
        let mut app = App::new();
        app.insert_resource(AudioBackend::Mock);
        Self::new(build(app))
    }

    fn server(&self) -> AssetServer {
        self.app.get_resource::<AssetServer>().unwrap().clone()
    }

    fn load_tone(&mut self, project: &Path) -> AssetHandle<AudioClip> {
        let imported = import(project, "tone.wav", &fixture("tone.wav")).unwrap();
        write_content(project, "tone.wav", &imported);
        let server = self.server();
        server.publish_content_source(
            ContentAssetRoot::Directory(project.to_path_buf()),
            AssetRegistry::from_content_tree(project, "content", "gasset").unwrap(),
        );
        let clip = server.load::<AudioClip>(imported.asset_id);
        let started = Instant::now();
        while server.is_loading() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "load timed out"
            );
            thread::sleep(Duration::from_millis(1));
            self.app.update();
        }
        self.app.update();
        clip
    }

    fn spawn(&mut self, player: AudioPlayer) -> Entity {
        self.app.main_mut().world_mut().spawn(player)
    }

    fn player(&mut self, entity: Entity) -> Option<&mut AudioPlayer> {
        self.app
            .main_mut()
            .world_mut()
            .get_component_for_entity_mut::<AudioPlayer>(entity)
    }

    fn is_alive(&self, entity: Entity) -> bool {
        self.app.main().world().entity_is_valid(entity)
    }

    fn sounds_on(&self, bus: AudioBus) -> usize {
        self.app
            .get_resource::<Audio>()
            .unwrap()
            .bus_sound_count(bus)
    }

    fn step(&mut self, seconds: f32) -> Vec<Entity> {
        self.app
            .get_resource_mut::<Audio>()
            .unwrap()
            .advance(seconds);
        self.app.update();
        let mut finished: Vec<Entity> = self
            .app
            .get_resource::<EventChannel<SoundFinished>>()
            .unwrap()
            .read_from(&mut self.cursor)
            .map(|event| event.entity)
            .collect();
        finished.sort_by_key(Entity::to_bits);
        finished
    }
}

fn build(mut app: App) -> App {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(AudioPlugin);
    app.finish_plugin_build();
    app.update();
    app
}

fn sorted(mut entities: Vec<Entity>) -> Vec<Entity> {
    entities.sort_by_key(Entity::to_bits);
    entities
}

#[test]
fn a_player_plays_its_clip_on_its_bus_and_ends_as_its_settings_say() {
    let project = tempfile::tempdir().unwrap();
    let mut playback = Playback::game();
    let tone = playback.load_tone(project.path());
    let missing = playback.server().load::<AudioClip>(AssetId::new());

    let despawned = playback.spawn(AudioPlayer::new(tone.clone()));
    let removed = playback.spawn(AudioPlayer::new(tone.clone()).with_end(PlaybackEnd::Remove));
    let kept = playback.spawn(AudioPlayer::new(tone.clone()).with_end(PlaybackEnd::Keep));
    let looping = playback.spawn(AudioPlayer::new(tone.clone()).with_looping(true));
    let moved = playback.spawn(
        AudioPlayer::new(tone.clone())
            .with_looping(true)
            .with_bus(AudioBus::Music),
    );
    let cut = playback.spawn(
        AudioPlayer::new(tone.clone())
            .with_looping(true)
            .with_bus(AudioBus::Dialogue),
    );
    let failed = playback.spawn(AudioPlayer::new(missing).with_end(PlaybackEnd::Remove));

    assert_eq!(playback.step(0.0), []);
    assert_eq!(playback.sounds_on(AudioBus::Effects), 4);
    assert_eq!(playback.sounds_on(AudioBus::Music), 1);
    assert_eq!(playback.sounds_on(AudioBus::Dialogue), 1);

    assert_eq!(
        playback.step(0.05),
        [failed],
        "a clip that fails ends its player"
    );
    assert!(playback.player(failed).is_none());

    playback.app.main_mut().world_mut().despawn(cut);
    playback.player(moved).unwrap().bus = AudioBus::Ambience;
    assert_eq!(
        playback.step(0.02),
        [],
        "despawning a playing entity sends nothing"
    );
    assert_eq!(playback.sounds_on(AudioBus::Dialogue), 0);
    assert_eq!(playback.step(0.02), []);
    assert_eq!(playback.sounds_on(AudioBus::Music), 0);
    assert_eq!(playback.sounds_on(AudioBus::Ambience), 1);

    assert_eq!(
        playback.step(0.05),
        sorted(vec![despawned, removed, kept]),
        "each ending sound finishes once its clip's duration has passed"
    );
    assert!(!playback.is_alive(despawned));
    assert!(playback.is_alive(removed));
    assert!(playback.player(removed).is_none());
    assert!(playback.player(kept).is_some());

    assert_eq!(
        playback.step(0.2),
        [],
        "a looping sound and a kept one send nothing more"
    );
    assert_eq!(playback.sounds_on(AudioBus::Effects), 1);

    playback.app.main_mut().world_mut().insert(
        AudioPlayer::new(tone.clone()).with_end(PlaybackEnd::Keep),
        kept,
    );
    assert_eq!(playback.step(0.0), []);
    assert_eq!(playback.sounds_on(AudioBus::Effects), 2);
    assert_eq!(
        playback.step(0.15),
        [kept],
        "a re-inserted player plays again"
    );
    assert!(playback.is_alive(looping));

    playback
        .app
        .main_mut()
        .world_mut()
        .insert(AudioPlayer::new(tone).with_looping(true), looping);
    playback.step(0.0);
    assert_eq!(
        playback.step(0.02),
        [],
        "a player inserted over a playing one sends nothing"
    );
    assert_eq!(
        playback.sounds_on(AudioBus::Effects),
        1,
        "a player inserted over a playing one stops the old sound"
    );
}

#[test]
fn an_edit_app_opens_no_audio_and_leaves_players_alone() {
    let mut playback = Playback::new(build(App::for_editing()));
    assert!(playback.app.get_resource::<Audio>().is_none());

    let missing = playback.server().load::<AudioClip>(AssetId::new());
    let player = playback.spawn(AudioPlayer::new(missing));
    for _ in 0..4 {
        playback.app.update();
    }

    assert!(playback.player(player).is_some());
}

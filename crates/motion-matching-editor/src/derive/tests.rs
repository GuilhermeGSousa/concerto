use std::path::PathBuf;

use concerto_app::App;
use concerto_ecs::events::event_channel::EventChannel;
use concerto_editor::{
    asset_editor::EditorDocument, hosted::testing::SceneFixture, project::AssetsWritten,
};
use concerto_foundation::assets::{AssetId, handle::AssetHandle, registry::AssetRegistry};
use concerto_motion_matching::{
    BonePositions, FeatureChannel, FeatureChannelData, FrameContext, MotionDatabase,
    ReflectFeatureChannel, TimeRange, bake,
};
use concerto_reflect::{Reflect, ReflectedValue};
use concerto_ui::transform::UIValue;

use super::*;
use crate::{
    panels::tests::{Tab, clip_of, skeleton_of, stat_warnings, tab_of},
    playback::DatabasePlayback,
    tests::{broken_recipe, database_recipe, edit, fixture, fixture_with, save, write_asset},
};

fn head_positions(weight: f32) -> FeatureChannelData {
    FeatureChannelData::live(BonePositions {
        bones: vec!["Head".into()],
        weight,
    })
}

fn derivable(definition: &mut MotionDatabase) {
    definition.simulation.position_joint = "Spine".into();
    definition.channels = vec![head_positions(1.0)];
}

fn ready_with(f: SceneFixture, change: impl FnOnce(&mut MotionDatabase)) -> Tab {
    let mut tab = tab_of(f, |definition| {
        derivable(definition);
        change(definition);
    });
    tab.until_read();
    tab.until_settled();
    tab
}

fn ready(f: SceneFixture) -> Tab {
    ready_with(f, |_| {})
}

fn status(tab: &Tab) -> String {
    tab.f.status(tab.document)
}

fn frames(tab: &Tab) -> Option<usize> {
    tab.state()
        .playback
        .as_ref()
        .and_then(|playback| playback.database.frame_count())
}

fn preview_shows_a_database(tab: &Tab) -> bool {
    tab.f
        .scene_world(tab.document)
        .get_resource::<DatabasePlayback>()
        .unwrap()
        .shown
        .is_some()
}

fn cache_files(tab: &Tab) -> Vec<PathBuf> {
    let directory = tab
        .f
        .project_root
        .join("derived-cache")
        .join(MotionDatabase::name())
        .join(format!("v{}", MotionDatabase::DERIVED_VERSION));
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries.map(|entry| entry.unwrap().path()).collect()
}

fn resaved(tab: &mut Tab) {
    save(&mut tab.f, tab.document);
    tab.until_settled();
}

fn written(tab: &mut Tab, ids: Vec<AssetId>) {
    tab.f
        .world
        .get_resource_mut::<EventChannel<AssetsWritten>>()
        .unwrap()
        .push_event(AssetsWritten { ids });
    tab.until_settled();
}

fn reopened(tab: &mut Tab) {
    let document = tab.document;
    let other = write_asset(
        &mut tab.f,
        "content/hero/other.gasset",
        &MotionDatabase::default(),
    );
    tab.f.request(&other);
    tab.f
        .frames_until("the other definition never opened", |f| {
            f.document(document).current.as_ref() == Some(&other)
        });
    tab.until_settled();
    let first = tab.asset.clone();
    tab.f.request(&first);
    tab.f.frames_until("the definition never reopened", |f| {
        f.document(document).current.as_ref() == Some(&first)
    });
    tab.until_settled();
}

fn gated_job(
    tab: &Tab,
    progress: Option<(usize, usize)>,
) -> (DeriveJob, std::sync::mpsc::Sender<()>) {
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let job = DeriveJob {
        handle: std::thread::spawn(move || {
            let _ = wait.recv();
            Err(anyhow!("stopped"))
        }),
        asset: tab.asset.clone(),
        progress: Arc::new(Mutex::new(progress)),
        line: String::new(),
    };
    (job, release)
}

#[derive(Reflect, Default, Clone)]
#[reflect(FeatureChannel)]
struct GameChannel {
    weight: f32,
}

impl FeatureChannel for GameChannel {
    fn dims(&self) -> usize {
        1
    }

    fn extract(&self, _: &FrameContext, out: &mut [f32]) {
        out[0] = 1.0;
    }

    fn weight(&self) -> f32 {
        self.weight
    }
}

fn game_recipe(app: &mut App) {
    database_recipe(app);
    app.register_type::<GameChannel>();
}

#[test]
fn progress_names_the_entry_being_derived() {
    assert_eq!(progress_line(0, 6), "Deriving 1 of 6");
    assert_eq!(progress_line(1, 6), "Deriving 2 of 6");
    assert_eq!(progress_line(5, 6), "Deriving 6 of 6");
    assert_eq!(progress_line(6, 6), "Deriving 6 of 6");
}

#[test]
fn opening_a_definition_derives_it_into_the_cache_and_the_preview() {
    let tab = ready(fixture());

    assert_eq!(frames(&tab), Some(81 + 109));
    assert!(preview_shows_a_database(&tab));
    assert_eq!(cache_files(&tab).len(), 1);
    assert_eq!(status(&tab), "");
    assert!(!tab.working());
}

#[test]
fn a_derive_shows_its_stats() {
    let mut tab = ready(fixture());

    let stats = tab.state().stats.clone().expect("a derive fills the stats");
    assert_eq!(
        stats,
        BakedStats::of(&tab.state().playback.as_ref().unwrap().database)
    );
    assert_eq!(stats.frames, 81 + 109);
    assert_eq!(
        stats
            .entries
            .iter()
            .map(|entry| (entry.clip, entry.frames, entry.short))
            .collect::<Vec<_>>(),
        [(tab.walk.id, 81, false), (tab.run.id, 109, false)]
    );
    assert!(stats.bytes > 0);
    assert_eq!(stats.channels.len(), 1);
    assert_eq!(stats.channels[0].name, "BonePositions");

    assert!(tab.shown(tab.panels().stats_view));
    assert_eq!(tab.height(tab.panels().stats_view), UIValue::Px(16.0 * 5.0));
    let list = tab.panels().stats;
    let labels: Vec<String> = tab
        .labels_under(vec![list])
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    for shown in [
        "Frames",
        "190",
        "Memory",
        "walk",
        "81",
        "run",
        "109",
        "BonePositions",
    ] {
        assert!(
            labels.iter().any(|label| label == shown),
            "{shown}: {labels:?}"
        );
    }
}

#[test]
fn an_entry_shorter_than_the_trajectory_gets_a_warning() {
    let tab = ready_with(fixture(), |definition| {
        definition.entries[1].range = Some(TimeRange {
            start: 0.0,
            end: 0.5,
        });
    });

    let stats = tab.state().stats.clone().unwrap();
    assert!(!stats.entries[0].short);
    assert!(stats.entries[1].short);
    assert_eq!(stats.horizon, 1.0);
    let assets = tab.world().get_resource::<AssetRegistry>().unwrap();
    assert_eq!(
        stat_warnings(&stats, assets),
        ["run", "Shorter than the 1.00 s trajectory"]
    );
}

#[test]
fn a_derive_error_shows_on_the_status_line_and_caches_nothing() {
    let tab = ready_with(fixture(), |definition| {
        definition.simulation.position_joint = "Nowhere".into();
    });

    assert_eq!(
        status(&tab),
        "Derive failed: the skeleton has no joint `Nowhere`"
    );
    assert!(tab.state().playback.is_none());
    assert!(tab.state().stats.is_none());
    assert!(cache_files(&tab).is_empty());
}

#[test]
fn a_missing_clip_fails_the_derive_naming_it() {
    let mut tab = ready(fixture());
    std::fs::remove_file(tab.f.project_root.join(&tab.run.address)).unwrap();

    resaved(&mut tab);

    assert!(
        status(&tab).starts_with("Derive failed: "),
        "{}",
        status(&tab)
    );
    assert!(status(&tab).contains(&tab.run.address), "{}", status(&tab));
    assert!(tab.state().playback.is_none());
    assert!(!preview_shows_a_database(&tab));
}

#[test]
fn a_definition_with_no_skeleton_or_no_entry_derives_nothing() {
    let tab = ready_with(fixture(), |definition| {
        definition.skeleton = AssetHandle::default();
    });
    assert!(tab.state().playback.is_none());
    assert_eq!(status(&tab), "");

    let tab = ready_with(fixture(), |definition| definition.entries.clear());
    assert!(tab.state().playback.is_none());
    assert_eq!(status(&tab), "");
    assert!(cache_files(&tab).is_empty());
}

#[test]
fn a_failed_preview_derives_nothing() {
    let mut tab = tab_of(fixture_with(broken_recipe), derivable);
    tab.until_read();
    tab.until_settled();

    assert!(tab.state().playback.is_none());
    assert!(cache_files(&tab).is_empty());
}

#[test]
fn saving_derives_again() {
    let mut tab = ready(fixture());
    let document = tab.document;

    edit(&mut tab.f, document, "Settings", "rate", 30.0);
    tab.until_settled();
    assert_eq!(tab.state().playback.as_ref().unwrap().database.rate, 60.0);

    resaved(&mut tab);

    assert_eq!(tab.state().playback.as_ref().unwrap().database.rate, 30.0);
    assert_eq!(frames(&tab), Some(40 + 54));
    assert_eq!(cache_files(&tab).len(), 2);
}

#[test]
fn an_unsaved_edit_keeps_playing_the_last_derived_database() {
    let mut tab = ready(fixture());
    let document = tab.document;

    edit(&mut tab.f, document, "BonePositions", "weight", 2.5);
    tab.until_settled();

    let playback = tab.state().playback.as_ref().unwrap();
    assert_eq!(playback.channels[0].weight(), 1.0);
    assert!(preview_shows_a_database(&tab));
}

#[test]
fn a_saved_weight_change_reads_the_cache_and_plays_the_new_weight() {
    let mut tab = ready(fixture());
    let document = tab.document;

    edit(&mut tab.f, document, "BonePositions", "weight", 2.5);
    resaved(&mut tab);

    let playback = tab.state().playback.as_ref().unwrap();
    assert_eq!(playback.channels[0].weight(), 2.5);
    assert_eq!(cache_files(&tab).len(), 1);
}

#[test]
fn a_second_open_reads_the_cache() {
    let mut tab = ready(fixture());
    let [cached] = &cache_files(&tab)[..] else {
        panic!("one blob was cached");
    };
    let mut shorter = tab.state().loaded.clone().unwrap();
    shorter.entries[1].range = Some(TimeRange {
        start: 0.0,
        end: 1.0,
    });
    let skeleton = skeleton_of(&["Hips", "Spine", "Head"]);
    let (walk, run) = (clip_of(10, 0..3), clip_of(2, 0..2));
    let planted = bake(&shorter, &skeleton, &[&walk, &run]).unwrap();
    let planted_frames = shorter.clone().with_baked(planted.clone()).frame_count();
    std::fs::write(cached, bincode::serialize(&planted).unwrap()).unwrap();

    reopened(&mut tab);

    assert_ne!(planted_frames, Some(81 + 109));
    assert_eq!(frames(&tab), planted_frames);
}

#[test]
fn a_written_clip_of_the_definition_derives_it_again() {
    let mut tab = ready(fixture());
    let address = tab.run.address.clone();
    let run = write_asset(&mut tab.f, &address, &clip_of(3, 0..2));
    assert_eq!(run.id, tab.run.id);

    written(&mut tab, vec![AssetId::new(), run.id]);

    assert_eq!(frames(&tab), Some(81 + 163));
    assert_eq!(cache_files(&tab).len(), 2);
}

#[test]
fn a_written_asset_the_definition_does_not_read_derives_nothing() {
    let mut tab = ready(fixture());
    let address = tab.run.address.clone();
    write_asset(&mut tab.f, &address, &clip_of(3, 0..2));

    let definition = tab.asset.id;

    written(&mut tab, vec![AssetId::new(), definition]);

    assert_eq!(frames(&tab), Some(81 + 109));
    assert_eq!(cache_files(&tab).len(), 1);
}

#[test]
fn a_game_registered_channel_derives() {
    let tab = ready_with(fixture_with(game_recipe), |definition| {
        definition.channels = vec![FeatureChannelData::live(GameChannel { weight: 2.0 })];
    });

    let playback = tab.state().playback.as_ref().unwrap();
    assert_eq!(playback.database.channels[0].type_name(), "GameChannel");
    assert_eq!(playback.database.features(0).unwrap(), [1.0]);
}

#[test]
fn the_status_counts_while_a_derive_runs() {
    let mut tab = ready(fixture());
    let (job, release) = gated_job(&tab, Some((1, 6)));
    tab.state_mut().derive_job = Some(job);

    tab.f.frame();

    assert_eq!(status(&tab), "Deriving 2 of 6");
    assert!(tab.shown(tab.panels().status));
    assert!(tab.working());

    release.send(()).unwrap();
    tab.until_settled();

    assert!(!tab.working());
    assert_eq!(status(&tab), "Derive failed: stopped");
}

#[test]
fn a_cache_hit_leaves_the_status_alone() {
    let mut tab = ready(fixture());
    let document = tab.document;
    tab.f
        .world
        .get_component_for_entity_mut::<EditorDocument>(document)
        .unwrap()
        .status = "Saved it".into();
    tab.state_mut().derive_wanted = true;

    tab.until_settled();

    assert_eq!(status(&tab), "Saved it");
}

#[test]
fn opening_another_definition_drops_the_derive() {
    let mut tab = ready(fixture());
    let (job, release) = gated_job(&tab, None);
    tab.state_mut().derive_job = Some(job);
    let other = write_asset(
        &mut tab.f,
        "content/hero/other.gasset",
        &MotionDatabase::default(),
    );

    tab.f.request(&other);
    let document = tab.document;
    tab.f
        .frames_until("the other definition never opened", |f| {
            f.document(document).current.as_ref() == Some(&other)
        });
    release.send(()).unwrap();
    tab.until_settled();

    assert_eq!(status(&tab), "");
    assert!(tab.state().playback.is_none());
}

#[test]
fn a_finished_job_for_a_replaced_definition_is_dropped() {
    let mut tab = ready(fixture());
    let (mut job, release) = gated_job(&tab, None);
    job.asset = AssetEntry {
        address: "content/hero/gone.gasset".into(),
        ..job.asset.clone()
    };
    tab.state_mut().derive_job = Some(job);
    release.send(()).unwrap();

    tab.until_settled();

    assert_eq!(status(&tab), "");
    assert!(tab.state().playback.is_some());
}

#[test]
fn a_derive_that_a_newer_one_replaces_is_dropped() {
    let mut tab = ready(fixture());
    let (job, release) = gated_job(&tab, None);
    tab.state_mut().derive_job = Some(job);
    tab.state_mut().derive_wanted = true;
    release.send(()).unwrap();

    tab.until_settled();

    assert_eq!(status(&tab), "");
    assert_eq!(frames(&tab), Some(81 + 109));
}

#[test]
fn a_cache_hit_after_a_replaced_derive_clears_its_progress_line() {
    let mut tab = ready(fixture());
    let release = counting(&mut tab);
    tab.state_mut().derive_wanted = true;
    release.send(()).unwrap();

    tab.until_settled();

    assert_eq!(status(&tab), "");
    assert_eq!(frames(&tab), Some(81 + 109));
}

fn counting(tab: &mut Tab) -> std::sync::mpsc::Sender<()> {
    let (job, release) = gated_job(tab, Some((1, 6)));
    tab.state_mut().derive_job = Some(job);
    tab.f.frame();
    assert_eq!(status(tab), "Deriving 2 of 6");
    release
}

#[test]
fn a_save_failure_during_a_derive_keeps_its_line() {
    let mut tab = ready(fixture());
    let document = tab.document;
    let release = counting(&mut tab);
    edit(&mut tab.f, document, "Settings", "speed", 2.0);
    tab.f.make_project_unwritable();

    save(&mut tab.f, document);
    let failed = status(&tab);
    assert!(failed.starts_with("Save failed: "), "{failed}");
    tab.f.frame();
    tab.f.frame();
    assert_eq!(status(&tab), failed);

    release.send(()).unwrap();
    tab.until_settled();

    assert_eq!(status(&tab), failed);
}

#[test]
fn a_refused_open_during_a_derive_keeps_its_error() {
    let mut tab = ready(fixture());
    let document = tab.document;
    let mut refused = tab.state().loaded.clone().unwrap();
    refused.channels.push(FeatureChannelData::Stored {
        type_name: "NeverRegistered".into(),
        value: ReflectedValue::Struct(vec![]),
    });
    let refused = write_asset(&mut tab.f, "content/hero/refused.gasset", &refused);
    let release = counting(&mut tab);

    tab.f.request(&refused);
    tab.f
        .frames_until("the refused definition never answered", |f| {
            f.document(document).pending.is_none()
        });
    let error = status(&tab);
    assert!(error.contains("NeverRegistered"), "{error}");
    tab.f.frame();
    tab.f.frame();
    assert_eq!(status(&tab), error);

    release.send(()).unwrap();
    tab.until_settled();

    assert_eq!(status(&tab), error);
}

#[test]
fn an_invalid_weight_fails_the_derive_on_a_miss_and_a_hit() {
    let invalid = "Derive failed: channel weight 0 must be positive and finite";
    let tab = ready_with(fixture(), |definition| {
        definition.channels = vec![head_positions(0.0)];
    });
    assert_eq!(status(&tab), invalid);
    assert!(tab.state().playback.is_none());

    let mut tab = ready(fixture());
    let document = tab.document;
    edit(&mut tab.f, document, "BonePositions", "weight", 0.0);
    resaved(&mut tab);

    assert_eq!(cache_files(&tab).len(), 1);
    assert_eq!(status(&tab), invalid);
    assert!(tab.state().playback.is_none());
}

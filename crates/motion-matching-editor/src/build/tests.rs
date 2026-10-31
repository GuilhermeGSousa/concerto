use std::{any::TypeId, sync::atomic::AtomicUsize, time::Duration};

use concerto_app::App;
use concerto_ecs::reflect::ReflectComponent;
use concerto_editor::{
    asset_editor::Unsaved,
    hosted::{testing::SceneFixture, trigger_on},
    inspector::{PropertyCommits, PropertyRow, commit_number},
    scene::{SaveFailed, SceneSaved},
    tools::{edits::SetValue, selection::Select},
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{
        CONTENT_FORMAT_VERSION, ContentAssetHeader, read_content_asset, write_content_asset,
    },
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_motion_matching::{
    BonePositions, FeatureChannel, FrameContext, MotionDatabase, MotionDatabaseDefinition,
    ReflectFeatureChannel, StoredChannel, TimeRange,
};
use concerto_reflect::{PropertyPath, Reflect, to_value};
use concerto_ui::transform::UIValue;
use serde_json::json;

use super::*;
use crate::{
    DatabaseSettings,
    joint::tests::with_inspector,
    panels::tests::{Tab, clip_of, stat_warnings, tab_of},
    read_definition,
    tests::{
        broken_recipe, channels, database_recipe, fixture, fixture_with, save, set,
        settings_entity, write_asset, write_definition,
    },
};

const BAKED: &str = "content/hero/database_baked.gasset";

fn head_positions(weight: f32) -> StoredChannel {
    StoredChannel {
        type_name: "BonePositions".into(),
        value: to_value(&BonePositions {
            bones: vec!["Head".into()],
            weight,
        })
        .unwrap(),
    }
}

fn buildable(definition: &mut MotionDatabaseDefinition) {
    definition.simulation.position_joint = "Spine".into();
    definition.channels = vec![head_positions(1.0)];
}

fn ready_with(f: SceneFixture, change: impl FnOnce(&mut MotionDatabaseDefinition)) -> Tab {
    let mut tab = tab_of(f, |definition| {
        buildable(definition);
        change(definition);
    });
    tab.until_read();
    tab.until_settled();
    tab
}

fn ready(f: SceneFixture) -> Tab {
    ready_with(f, |_| {})
}

fn press_build(tab: &mut Tab) {
    let build = tab.panels().build;
    tab.clicked(build);
    tab.until_settled();
}

fn built() -> Tab {
    let mut tab = ready(fixture());
    press_build(&mut tab);
    tab
}

fn baked_exists(tab: &Tab) -> bool {
    tab.f.project_root.join(BAKED).exists()
}

fn baked_database(tab: &Tab) -> MotionDatabase {
    read_baked(&tab.f.project_root, BAKED).expect("the baked file reads")
}

fn registered(tab: &Tab) -> Option<AssetEntry> {
    tab.world()
        .get_resource::<AssetRegistry>()
        .unwrap()
        .get(AssetId::from_path(BAKED))
        .cloned()
}

fn status(tab: &Tab) -> String {
    tab.f.status(tab.document)
}

fn weight_row(tab: &mut Tab, document: Entity) -> PropertyRow {
    let channel = channels(&mut tab.f, document)[0];
    tab.f.signal(Select(Some(channel)));
    for _ in 0..3 {
        tab.f.frame();
    }
    let world = &mut tab.f.world;
    world
        .query::<&PropertyRow, ()>()
        .iter(world)
        .find(|row| row.path.to_string() == "weight")
        .cloned()
        .expect("the inspector shows the channel's weight")
}

fn resaved(tab: &mut Tab) {
    save(&mut tab.f, tab.document);
    tab.until_settled();
}

fn gated_job(tab: &Tab, finished: usize, total: usize) -> (BuildJob, std::sync::mpsc::Sender<()>) {
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let job = BuildJob {
        handle: std::thread::spawn(move || {
            let _ = wait.recv();
            Err(anyhow!("stopped"))
        }),
        asset: tab.asset.clone(),
        progress: Arc::new(AtomicUsize::new(finished)),
        total,
    };
    (job, release)
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, FeatureChannel)]
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
fn the_baked_file_sits_next_to_the_definition() {
    let asset = |address: &str| AssetEntry {
        id: AssetId::from_path(address),
        address: address.into(),
        kind: MotionDatabaseDefinition::name().into(),
        provenance: None,
    };

    assert_eq!(
        baked_address(&asset("content/hero/database.gasset")),
        "content/hero/database_baked.gasset"
    );
    assert_eq!(
        baked_address(&asset("database.gasset")),
        "database_baked.gasset"
    );
}

#[test]
fn progress_names_the_entry_being_baked() {
    assert_eq!(progress_line(0, 6), "Baking 1 of 6");
    assert_eq!(progress_line(1, 6), "Baking 2 of 6");
    assert_eq!(progress_line(5, 6), "Baking 6 of 6");
    assert_eq!(progress_line(6, 6), "Baking 6 of 6");
}

#[test]
fn a_full_bake_writes_the_file_and_its_registry_entry() {
    let tab = built();

    let bytes = std::fs::read(tab.f.project_root.join(BAKED)).unwrap();
    let (header, _) = read_content_asset(&bytes).unwrap();
    assert_eq!(header.asset_id, AssetId::from_path(BAKED));
    assert_eq!(header.kind, MotionDatabase::name());

    let entry = registered(&tab).expect("the baked file is in the registry");
    assert_eq!(entry.address, BAKED);
    assert!(entry.is::<MotionDatabase>());

    let database = baked_database(&tab);
    assert_ne!(database.source_hash(), 0);
    assert_eq!(database.frame_count(), 81 + 109);

    assert_eq!(status(&tab), format!("Built {BAKED}"));
    assert_eq!(tab.state().baked, BakedState::UpToDate);
    assert_eq!(tab.text(tab.panels().baked).text, "Up to date");
}

#[test]
fn a_build_shows_its_stats() {
    let mut tab = built();

    let stats = tab.state().stats.clone().expect("a build fills the stats");
    assert_eq!(stats, BakedStats::of(&baked_database(&tab)));
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
    let mut tab = ready_with(fixture(), |definition| {
        definition.entries[1].range = Some(TimeRange {
            start: 0.0,
            end: 0.5,
        });
    });

    press_build(&mut tab);

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
fn a_bake_error_shows_on_the_status_line_and_writes_nothing() {
    let mut tab = ready_with(fixture(), |definition| {
        definition.simulation.position_joint = "Nowhere".into();
    });

    press_build(&mut tab);

    assert_eq!(
        status(&tab),
        "Build failed: the skeleton has no joint `Nowhere`"
    );
    assert!(!baked_exists(&tab));
    assert!(registered(&tab).is_none());
    assert_eq!(tab.state().baked, BakedState::NotBuilt);
}

#[test]
fn a_weight_only_change_takes_the_fast_path() {
    let mut tab = built();
    let channel = channels(&mut tab.f, tab.document)[0];

    set::<BonePositions>(&mut tab.f, channel, ["weight"], json!(2.5));
    press_build(&mut tab);

    assert_eq!(status(&tab), format!("Updated the weights in {BAKED}"));
    assert_eq!(baked_database(&tab).channels()[0].weight, 2.5);
    assert_eq!(tab.state().baked, BakedState::UpToDate);
}

#[test]
fn a_changed_setting_bakes_in_full() {
    let mut tab = built();
    let settings = settings_entity(&mut tab.f, tab.document);

    set::<DatabaseSettings>(&mut tab.f, settings, ["rate"], json!(30.0));
    press_build(&mut tab);

    assert_eq!(status(&tab), format!("Built {BAKED}"));
    assert_eq!(baked_database(&tab).rate(), 30.0);
}

#[test]
fn a_definition_pointed_at_another_skeleton_with_identical_bytes_bakes_in_full() {
    let mut tab = built();
    let bytes = std::fs::read(tab.f.project_root.join(&tab.skeleton.address)).unwrap();
    let id = AssetId::new();
    let (mut header, payload) = {
        let (header, payload) = read_content_asset(&bytes).unwrap();
        (header, payload.to_vec())
    };
    header.asset_id = id;
    let address = "content/hero/twin.gasset";
    std::fs::write(
        tab.f.project_root.join(address),
        write_content_asset(&header, &payload).unwrap(),
    )
    .unwrap();
    let twin = AssetEntry {
        id,
        address: address.into(),
        ..tab.skeleton.clone()
    };
    tab.f.add_to_project(&twin);
    tab.state_mut().skeleton = AssetHandle::weak(id);
    tab.f.world.insert(Unsaved, tab.document);

    press_build(&mut tab);

    assert_eq!(status(&tab), format!("Built {BAKED}"));
    assert_eq!(baked_database(&tab).skeleton().id(), id);
}

#[test]
fn closing_the_tab_during_a_build_writes_nothing() {
    let mut tab = ready(fixture());
    let build = tab.panels().build;

    tab.click(build);
    tab.f.frame();
    assert!(tab.state().build.is_running());
    tab.f.close(tab.document);
    tab.f.frame();
    std::thread::sleep(Duration::from_millis(200));
    tab.f.frame();
    tab.f.frame();

    assert!(!baked_exists(&tab));
    assert!(registered(&tab).is_none());
}

#[test]
fn opening_another_definition_drops_the_build() {
    let mut tab = ready(fixture());
    let other = write_definition(
        &mut tab.f,
        "content/hero/other.gasset",
        &MotionDatabaseDefinition::default(),
    );
    let build = tab.panels().build;

    tab.click(build);
    tab.f.frame();
    assert!(tab.state().build.is_running());
    tab.f.request(&other);
    let document = tab.document;
    tab.f
        .frames_until("the other definition never opened", |f| {
            f.document(document).current.as_ref() == Some(&other)
        });

    assert!(!tab.state().build.is_running());
    std::thread::sleep(Duration::from_millis(200));
    tab.f.frame();
    tab.f.frame();
    assert!(!baked_exists(&tab));
    assert!(registered(&tab).is_none());
}

#[test]
fn a_finished_job_for_a_replaced_definition_writes_nothing() {
    let mut tab = ready(fixture());
    let (mut job, release) = gated_job(&tab, 0, 1);
    job.asset = AssetEntry {
        address: "content/hero/gone.gasset".into(),
        ..job.asset.clone()
    };
    tab.state_mut().build.job = Some(job);
    release.send(()).unwrap();

    tab.until_settled();

    assert_eq!(status(&tab), "");
}

#[test]
fn a_game_registered_channel_bakes() {
    let mut tab = ready_with(fixture_with(game_recipe), |definition| {
        definition.channels = vec![StoredChannel {
            type_name: "GameChannel".into(),
            value: to_value(&GameChannel { weight: 2.0 }).unwrap(),
        }];
    });

    press_build(&mut tab);

    assert_eq!(status(&tab), format!("Built {BAKED}"));
    let database = baked_database(&tab);
    assert_eq!(database.channels()[0].type_name, "GameChannel");
    assert_eq!(database.features(0), [1.0]);
}

#[test]
fn a_failed_preview_disables_build() {
    let mut tab = tab_of(fixture_with(broken_recipe), buildable);
    tab.until_read();
    tab.f.frame();

    assert!(tab.disabled(tab.panels().build));
    assert_eq!(
        tab.text(tab.panels().note).text,
        "The preview is not running"
    );
    assert!(tab.shown(tab.panels().note));
    assert!(!tab.shown(tab.panels().baked));

    let build = tab.panels().build;
    tab.clicked(build);

    assert!(!tab.state().build.is_running());
}

#[test]
fn build_says_why_it_is_off() {
    let tab = ready_with(fixture(), |definition| {
        definition.skeleton = AssetHandle::default();
    });
    assert!(tab.disabled(tab.panels().build));
    assert_eq!(tab.text(tab.panels().note).text, "Pick a skeleton");

    let tab = ready_with(fixture(), |definition| definition.entries.clear());
    assert!(tab.disabled(tab.panels().build));
    assert_eq!(tab.text(tab.panels().note).text, "Add an entry");

    let tab = ready(fixture());
    assert!(!tab.disabled(tab.panels().build));
    assert!(!tab.shown(tab.panels().note));
}

#[test]
fn build_is_off_and_counts_while_it_runs() {
    let mut tab = ready(fixture());
    let (job, release) = gated_job(&tab, 1, 6);
    tab.state_mut().build.job = Some(job);

    tab.f.frame();

    assert!(tab.disabled(tab.panels().build));
    assert!(tab.shown(tab.panels().note));
    assert_eq!(tab.text(tab.panels().note).text, "Baking 2 of 6");
    assert!(tab.working());

    release.send(()).unwrap();
    tab.until_settled();

    assert!(!tab.disabled(tab.panels().build));
    assert!(!tab.shown(tab.panels().note));
    assert!(!tab.working());
    assert_eq!(status(&tab), "Build failed: stopped");
}

#[test]
fn a_clean_document_builds_without_saving() {
    let tab = built();

    assert!(tab.f.written::<SceneSaved>().is_empty());
}

#[test]
fn an_edited_document_saves_before_it_builds() {
    let mut tab = ready(fixture());
    let settings = settings_entity(&mut tab.f, tab.document);

    set::<DatabaseSettings>(&mut tab.f, settings, ["rate"], json!(30.0));
    press_build(&mut tab);

    assert_eq!(tab.f.written::<SceneSaved>().len(), 1);
    assert!(!tab.unsaved());
    let saved = read_definition(&tab.f.project_root, &tab.asset).unwrap();
    assert_eq!(saved.rate, 30.0);
    assert_eq!(baked_database(&tab).rate(), 30.0);
}

#[test]
fn an_edit_whose_event_has_not_arrived_counts_as_an_edit() {
    let mut tab = ready(fixture());
    let settings = settings_entity(&mut tab.f, tab.document);
    assert!(!is_edited(&tab.f.world, tab.document));

    trigger_on(
        &mut tab.f.world,
        tab.document,
        SetValue {
            entity: settings,
            component: TypeId::of::<DatabaseSettings>(),
            path: PropertyPath::new(["rate"]),
            value: json!(30.0),
        },
    );

    assert!(
        tab.world()
            .get_component_for_entity::<Unsaved>(tab.document)
            .is_none()
    );
    assert!(is_edited(&tab.f.world, tab.document));
}

#[test]
fn a_weight_typed_but_not_yet_committed_is_in_the_build() {
    let mut f = fixture();
    with_inspector(&mut f);
    let mut tab = ready(f);
    let document = tab.document;
    let row = weight_row(&mut tab, document);

    let commits = tab.f.world.get_resource_mut::<PropertyCommits>().unwrap();
    commit_number(commits, &row, 3.5).unwrap();
    press_build(&mut tab);

    assert_eq!(baked_database(&tab).channels()[0].weight, 3.5);
    assert!(!tab.unsaved());
}

#[test]
fn a_failed_save_stops_the_build() {
    let mut tab = ready(fixture());
    let settings = settings_entity(&mut tab.f, tab.document);
    set::<DatabaseSettings>(&mut tab.f, settings, ["rate"], json!(30.0));
    tab.f.make_project_unwritable();

    let build = tab.panels().build;
    tab.clicked(build);

    assert!(!tab.state().build.is_running());
    assert!(status(&tab).starts_with("Save failed: "));
    assert_eq!(tab.f.written::<SaveFailed>().len(), 1);
}

#[test]
fn with_no_baked_file_the_state_reads_not_built() {
    let tab = ready(fixture());

    assert_eq!(tab.state().baked, BakedState::NotBuilt);
    assert_eq!(tab.text(tab.panels().baked).text, "Not built");
    assert!(tab.state().stats.is_none());
    assert!(!tab.shown(tab.panels().stats_view));
}

#[test]
fn reopening_a_built_definition_reads_up_to_date_with_its_stats() {
    let mut tab = built();
    let other = write_definition(
        &mut tab.f,
        "content/hero/other.gasset",
        &MotionDatabaseDefinition::default(),
    );
    let document = tab.document;

    tab.f.request(&other);
    tab.f
        .frames_until("the other definition never opened", |f| {
            f.document(document).current.as_ref() == Some(&other)
        });
    tab.until_settled();
    assert_eq!(tab.state().baked, BakedState::NotBuilt);
    assert!(tab.state().stats.is_none());

    let first = tab.asset.clone();
    tab.f.request(&first);
    tab.f.frames_until("the definition never reopened", |f| {
        f.document(document).current.as_ref() == Some(&first)
    });
    tab.until_settled();

    assert_eq!(tab.state().baked, BakedState::UpToDate);
    assert!(tab.state().stats.is_some());
}

#[test]
fn a_changed_definition_reads_out_of_date_after_a_save() {
    let mut tab = built();
    let settings = settings_entity(&mut tab.f, tab.document);

    set::<DatabaseSettings>(&mut tab.f, settings, ["rate"], json!(30.0));
    tab.f.frame();
    assert_eq!(tab.state().baked, BakedState::UpToDate);
    resaved(&mut tab);

    assert_eq!(tab.state().baked, BakedState::OutOfDate);
    assert_eq!(tab.text(tab.panels().baked).text, "Out of date");
}

#[test]
fn a_changed_weight_reads_out_of_date_after_a_save() {
    let mut tab = built();
    let channel = channels(&mut tab.f, tab.document)[0];

    set::<BonePositions>(&mut tab.f, channel, ["weight"], json!(2.5));
    tab.f.frame();
    resaved(&mut tab);

    assert_eq!(tab.state().baked, BakedState::OutOfDate);
}

#[test]
fn a_changed_clip_reads_out_of_date_after_a_save() {
    let mut tab = built();
    let address = tab.run.address.clone();

    write_asset(&mut tab.f, &address, &clip_of(3, 0..3));
    resaved(&mut tab);

    assert_eq!(tab.state().baked, BakedState::OutOfDate);
}

#[test]
fn a_missing_clip_reads_cant_read() {
    let mut tab = built();

    std::fs::remove_file(tab.f.project_root.join(&tab.run.address)).unwrap();
    resaved(&mut tab);

    assert_eq!(tab.state().baked, BakedState::CantRead("run".into()));
    assert_eq!(tab.text(tab.panels().baked).text, "Can't read run");
}

#[test]
fn a_clip_gone_from_the_project_reads_cant_read() {
    let mut tab = built();
    let run = tab.run.id;

    tab.f
        .world
        .get_resource_mut::<AssetRegistry>()
        .unwrap()
        .remove(run);
    resaved(&mut tab);

    assert_eq!(
        tab.state().baked,
        BakedState::CantRead("the clip of entry 2".into())
    );
}

#[test]
fn a_missing_skeleton_reads_cant_read() {
    let mut tab = built();

    std::fs::remove_file(tab.f.project_root.join(&tab.skeleton.address)).unwrap();
    resaved(&mut tab);

    assert_eq!(tab.state().baked, BakedState::CantRead("skeleton".into()));
}

#[test]
fn a_baked_file_in_an_old_layout_reads_not_built() {
    let mut tab = built();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::from_path(BAKED),
        references: Vec::new(),
        kind: MotionDatabase::name().to_string(),
        provenance: None,
    };
    std::fs::write(
        tab.f.project_root.join(BAKED),
        write_content_asset(&header, b"an older layout").unwrap(),
    )
    .unwrap();

    resaved(&mut tab);

    assert_eq!(tab.state().baked, BakedState::NotBuilt);
    assert!(tab.state().stats.is_none());
}

#[test]
fn a_baked_file_in_an_old_layout_is_rebuilt_in_full() {
    let mut tab = built();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::from_path(BAKED),
        references: Vec::new(),
        kind: MotionDatabase::name().to_string(),
        provenance: None,
    };
    std::fs::write(
        tab.f.project_root.join(BAKED),
        write_content_asset(&header, b"an older layout").unwrap(),
    )
    .unwrap();

    press_build(&mut tab);

    assert_eq!(status(&tab), format!("Built {BAKED}"));
    assert_eq!(tab.state().baked, BakedState::UpToDate);
}

#[test]
fn every_worker_marks_the_document_until_it_ends() {
    let mut tab = ready(fixture());
    let (release, wait) = std::sync::mpsc::channel::<()>();
    tab.state_mut().baked_job = Some(std::thread::spawn(move || {
        let _ = wait.recv();
        (BakedState::NotBuilt, None)
    }));

    tab.f.frame();

    assert!(tab.working());

    release.send(()).unwrap();
    tab.until_settled();

    assert!(!tab.working());
}

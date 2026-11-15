use std::path::PathBuf;

use concerto_editor::hosted::testing::SceneFixture;
use concerto_motion_matching::{BonePositions, FeatureChannelData, MotionDatabase};
use serde_json::json;

use super::*;
use crate::{
    panels::tests::{Tab, tab_of},
    playback::DatabasePlayback,
    tests::{fixture, save, write_value},
};

fn ready(f: SceneFixture) -> Tab {
    let mut tab = tab_of(f, |definition| {
        definition.simulation.position_joint = "Spine".into();
        definition.channels = vec![FeatureChannelData::live(BonePositions {
            bones: vec!["Head".into()],
            weight: 1.0,
        })];
    });
    tab.until_read();
    tab.until_settled();
    tab
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

#[test]
fn a_definition_derives_into_the_cache_and_the_preview_on_open_and_again_on_save_but_not_on_an_edit()
 {
    let mut tab = ready(fixture());
    let document = tab.document;

    assert_eq!(frames(&tab), Some(81 + 109));
    assert!(preview_shows_a_database(&tab));
    assert_eq!(cache_files(&tab).len(), 1);

    write_value(&mut tab.f, document, "Settings", "rate", json!(30.0));
    tab.until_settled();
    assert_eq!(tab.state().playback.as_ref().unwrap().database.rate, 60.0);

    resaved(&mut tab);

    assert_eq!(tab.state().playback.as_ref().unwrap().database.rate, 30.0);
    assert_eq!(frames(&tab), Some(40 + 54));
    assert_eq!(cache_files(&tab).len(), 2);
    assert!(preview_shows_a_database(&tab));
}

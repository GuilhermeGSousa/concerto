use std::path::PathBuf;

use concerto_foundation::assets::AssetId;
use concerto_import::metadata::SourceMetadata;

fn sidecar(text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("source-metadata-{:?}", AssetId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("walk.bvh.import.toml");
    std::fs::write(&path, text).unwrap();
    path
}

const HEADER: &str =
    "version = 1\nsource_id = \"00000000-0000-0000-0000-000000000001\"\nimporter = \"bvh\"\n";

#[test]
fn settings_round_trip_through_load_and_the_writer() {
    let path = sidecar(&format!(
        "{HEADER}\n[settings]\nscale = 0.5\nframe_range = [0, 10]\n"
    ));
    let metadata = SourceMetadata::load(&path).unwrap();
    assert_eq!(metadata.settings["scale"].as_float(), Some(0.5));
    let written = toml::to_string_pretty(&metadata).unwrap();
    let reread = sidecar(&written);
    let again = SourceMetadata::load(&reread).unwrap();
    assert_eq!(again.settings, metadata.settings);
}

#[test]
fn a_sidecar_without_settings_parses_with_an_empty_table_and_writes_none() {
    let metadata = SourceMetadata::load(&sidecar(HEADER)).unwrap();
    assert!(metadata.settings.is_empty());
    assert!(!toml::to_string_pretty(&metadata)
        .unwrap()
        .contains("settings"));
}

#[test]
fn the_bvh_importer_name_is_accepted() {
    assert!(SourceMetadata::load(&sidecar(HEADER)).is_ok());
}

#[test]
fn an_unknown_importer_name_is_still_rejected() {
    let text = HEADER.replace("bvh", "fbx");
    assert!(SourceMetadata::load(&sidecar(&text)).is_err());
}

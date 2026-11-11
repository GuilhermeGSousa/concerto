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
fn a_sidecar_round_trips_its_settings_and_refuses_an_unknown_importer() {
    let path = sidecar(&format!(
        "{HEADER}\n[settings]\nscale = 0.5\nframe_range = [0, 10]\n"
    ));
    let metadata = SourceMetadata::load(&path).unwrap();
    assert_eq!(metadata.settings["scale"].as_float(), Some(0.5));
    let written = toml::to_string_pretty(&metadata).unwrap();
    let again = SourceMetadata::load(&sidecar(&written)).unwrap();
    assert_eq!(again.settings, metadata.settings);

    let bare = SourceMetadata::load(&sidecar(HEADER)).unwrap();
    assert!(bare.settings.is_empty());
    assert!(!toml::to_string_pretty(&bare).unwrap().contains("settings"));

    assert!(SourceMetadata::load(&sidecar(&HEADER.replace("bvh", "fbx"))).is_err());
}

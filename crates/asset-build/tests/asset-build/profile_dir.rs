use std::path::{Path, PathBuf};

use concerto_asset_build::profile_dir;

#[test]
fn the_profile_directory_is_found_above_out_dir_in_either_cargo_layout() {
    let cases: [(&str, Option<&str>); 4] = [
        (
            "/t/target/debug/build/game-0a1b/out",
            Some("/t/target/debug"),
        ),
        (
            "/t/target/debug/build/game/0a1b/out",
            Some("/t/target/debug"),
        ),
        (
            "/home/build/t/target/x86_64-unknown-linux-gnu/release/build/game/0a1b/out",
            Some("/home/build/t/target/x86_64-unknown-linux-gnu/release"),
        ),
        ("/t/out", None),
    ];
    for (out_dir, expected) in cases {
        assert_eq!(
            profile_dir(Path::new(out_dir)),
            expected.map(PathBuf::from),
            "{out_dir}"
        );
    }
}

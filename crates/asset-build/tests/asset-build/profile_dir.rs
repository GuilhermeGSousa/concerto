use std::path::Path;

use concerto_asset_build::profile_dir;

struct Layout {
    out_dir: &'static str,
    profile: &'static str,
}

#[test]
fn the_profile_directory_is_found_above_out_dir_in_either_cargo_layout() {
    let cases = [
        Layout {
            out_dir: "/t/target/debug/build/game-0a1b/out",
            profile: "/t/target/debug",
        },
        Layout {
            out_dir: "/t/target/debug/build/game/0a1b/out",
            profile: "/t/target/debug",
        },
        Layout {
            out_dir: "/home/build/t/target/x86_64-unknown-linux-gnu/release/build/game/0a1b/out",
            profile: "/home/build/t/target/x86_64-unknown-linux-gnu/release",
        },
    ];
    for case in &cases {
        assert_eq!(
            profile_dir(Path::new(case.out_dir)).as_deref(),
            Some(Path::new(case.profile)),
            "{}",
            case.out_dir
        );
    }
}

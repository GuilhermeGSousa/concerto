fn main() {
    concerto_editor::run_with(
        tech_demo::game,
        concat!(env!("CARGO_MANIFEST_DIR"), "/.."),
        &[concerto_motion_matching_editor::install],
    );
}

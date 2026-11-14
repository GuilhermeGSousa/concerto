fn main() {
    concerto_editor::run_with(
        tech_demo::sanatorium,
        concat!(env!("CARGO_MANIFEST_DIR"), "/.."),
        &[
            concerto_animation_editor::install,
            concerto_motion_matching_editor::install,
        ],
    );
}

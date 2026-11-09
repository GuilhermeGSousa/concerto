fn main() {
    concerto_editor::run_with(
        one_more_night::game,
        concat!(env!("CARGO_MANIFEST_DIR"), "/.."),
        &[concerto_animation_editor::install],
    );
}

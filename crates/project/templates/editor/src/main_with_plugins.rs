fn main() {
    concerto_editor::run_with(
        {{crate}}::game,
        concat!(env!("CARGO_MANIFEST_DIR"), "/.."),
        {{plugins}},
    );
}

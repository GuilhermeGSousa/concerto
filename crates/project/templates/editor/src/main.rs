fn main() {
    concerto_editor::run({{crate}}::game, concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
}

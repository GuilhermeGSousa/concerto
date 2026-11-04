fn main() {
    let recipe = if std::env::args().any(|arg| arg == "--sanatorium") {
        tech_demo::sanatorium
    } else {
        tech_demo::game
    };
    concerto::run(recipe, concerto::content_registry!());
}

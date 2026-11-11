fn main() {
    let manor = std::env::args().any(|arg| arg == "--manor");
    let game = if manor {
        one_more_night::manor
    } else {
        one_more_night::game
    };
    concerto::run(game, concerto::content_registry!());
}

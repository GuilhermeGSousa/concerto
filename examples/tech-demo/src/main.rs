fn main() {
    let has = |flag: &str| std::env::args().any(|arg| arg == flag);
    let recipe = if has("--sanatorium") && has("--retarget") {
        tech_demo::retargeted_sanatorium
    } else if has("--sanatorium") {
        tech_demo::sanatorium
    } else if has("--retarget") {
        tech_demo::retarget
    } else {
        tech_demo::grid
    };
    concerto::run(recipe, concerto::content_registry!());
}

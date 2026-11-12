use concerto::{DefaultPlugins, app::App, asset_id};

pub mod book;
pub mod book_ui;
pub mod casefile;
pub mod controls;
pub mod house;
pub mod level;
pub mod players;
pub mod rules;
pub mod save;
pub mod schedule;
#[cfg(not(target_arch = "wasm32"))]
pub mod seal;
pub mod session;
pub mod world;

use crate::clock::NightClockPlugin;

/// The case schema and its loader.
pub use one_more_night_case as case;

pub mod clock;

/// Builds the game; the standalone binary and the editor both call it.
pub fn game(app: &mut App) {
    players::install(app);
    app.register_plugin(DefaultPlugins);
    players::enable_joining(app);
    app.set_startup_scene(asset_id!("content/level/test_level.gasset"));
    app.register_plugin(NightClockPlugin::default());
    world::install(app, world::default_saves());
}

/// The manor, `-- --manor`: the sealed case when its bundle and key are there, saving as it goes, and the open walk otherwise.
pub fn manor(app: &mut App) {
    match sealed_case() {
        Some(data) => manor_under(
            app,
            Some(save::save_dir().join("manor")),
            world::Stage::manor(data),
        ),
        None => manor_walk(app),
    }
}

/// The manor's greybox with its open case that solves nothing, saving nowhere.
pub fn manor_walk(app: &mut App) {
    manor_under(app, None, world::Stage::manor_walk());
}

fn manor_under(app: &mut App, saves: Option<std::path::PathBuf>, stage: world::Stage) {
    players::install(app);
    app.register_plugin(DefaultPlugins);
    players::enable_joining(app);
    app.register_plugin(NightClockPlugin::default());
    world::install_stage(app, saves, stage);
}

#[cfg(not(target_arch = "wasm32"))]
fn sealed_case() -> Option<casefile::CaseData> {
    let bundle = seal::bundle_path();
    if !bundle.exists() {
        return None;
    }
    let files = match seal::open_files(&bundle, &seal::key_path()) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("The sealed case did not open: {error}. Playing the open walk.");
            return None;
        }
    };
    match casefile::CaseData::manor(&files) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("The sealed case opened and did not load. Playing the open walk.");
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn sealed_case() -> Option<casefile::CaseData> {
    None
}

/// The game as tests build it: the same, saving nowhere.
pub fn game_without_saves(app: &mut App) {
    players::install(app);
    app.register_plugin(DefaultPlugins);
    players::enable_joining(app);
    app.set_startup_scene(asset_id!("content/level/test_level.gasset"));
    app.register_plugin(NightClockPlugin::default());
    world::install(app, None);
}

//! AFTER HOURS — a first-person mannequin horror game.
use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{FixedUpdate, LateUpdate, Startup, Update},
    },
    ecs::{IntoSystemConfig, Res, system::NonSendMarker},
    foundation::transform::systems::{propagate_global_transforms, update_simple_entities},
    window::plugin::Window,
};

mod body;
mod content;
mod debug;
mod game;
mod level;
mod lighting;
mod mannequin;
mod meshes;
mod night;
mod palette;
mod platform;
mod player;
mod poses;
mod scare;
mod sfx;
mod store;
mod textures;
mod ui;

fn random_seed() -> u64 {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            (js_sys::Math::random() * u32::MAX as f64) as u64 ^ (js_sys::Date::now() as u64)
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x5eed)
        }
    }
}

fn set_title(_: NonSendMarker, window: Res<Window>) {
    window.window_handle.set_title("AFTER HOURS");
}

fn main() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            let level = if platform::debug_flag("trace") { log::Level::Info } else { log::Level::Warn };
            console_log::init_with_level(level).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }

    let seed = random_seed();
    let mut app = App::new();
    app.register_plugin(DefaultPlugins::default());

    app.insert_resource(game::Game::new(platform::load_best(), seed))
        .insert_resource(game::Rand::new(seed))
        .insert_resource(palette::PaletteSlot::default())
        .insert_resource(poses::PoseLibrary::default())
        .insert_resource(sfx::Sounds::default())
        .insert_resource(night::NightState::with_rebuild(night::Rebuild::Title))
        .insert_resource(mannequin::CurrentLevel::default())
        .insert_resource(mannequin::Flow::default())
        .insert_resource(player::Eye::default())
        .insert_resource(player::Settings::load())
        .insert_resource(scare::Caught::default())
        .insert_resource(scare::CameraOverride::default())
        .insert_resource(platform::CursorState::default());

    app.add_system(
        Startup,
        (
            set_title,
            palette::create_palette,
            poses::build_pose_library,
            sfx::load_sounds,
            ui::spawn_ui,
        ),
    );

    app.add_system(
        Update,
        (
            night::advance_phases,
            player::adjust_settings,
            night::rebuild_night,
            night::title_drift,
            player::control_player,
            mannequin::block_player,
            player::update_flashlight,
            night::objectives,
            night::night_clock,
            night::flicker_lights,
            mannequin::dress_mannequins,
            ui::update_ui,
        )
            .chain(),
    );
    app.add_system(FixedUpdate, body::apply_movers);

    app.add_system(
        LateUpdate,
        (
            scare::run_scare,
            player::place_camera,
            mannequin::hunt,
            mannequin::face_mannequins,
            mannequin::apply_poses,
        )
            .chain()
            .before(update_simple_entities)
            .before(propagate_global_transforms),
    );
    app.add_system(LateUpdate, platform::sync_cursor);
    app.insert_resource(debug::TraceFrame::default());
    app.add_system(LateUpdate, debug::trace.after(propagate_global_transforms));

    app.run();
}

//! The few things that differ between the browser and a desktop window:
//! pointer lock and saving progress.
//!
//! On the web, browsers only grant pointer lock inside a user gesture, so the
//! page's own click handler (in `index.html`) requests it whenever the game
//! has flagged that it wants it (`<body data-want-lock="1">`).
use concerto::{
    ecs::{Res, ResMut, Resource, system::NonSendMarker},
    window::plugin::Window,
};

use crate::game::{Game, Phase};

pub const IS_WEB: bool = cfg!(target_arch = "wasm32");

#[cfg(target_arch = "wasm32")]
mod imp {
    fn document() -> Option<web_sys::Document> {
        web_sys::window()?.document()
    }

    /// Whether the page URL carries `flag` in its query (`?nolock&noui`).
    /// Used for automated testing in headless browsers.
    pub fn debug_flag(flag: &str) -> bool {
        web_sys::window()
            .and_then(|w| w.location().search().ok())
            .is_some_and(|q| q.trim_start_matches('?').split('&').any(|f| f == flag))
    }

    /// The value of `name=value` in the page URL's query, if present.
    pub fn debug_value(name: &str) -> Option<String> {
        let query = web_sys::window()?.location().search().ok()?;
        query
            .trim_start_matches('?')
            .split('&')
            .find_map(|f| f.strip_prefix(name)?.strip_prefix('=').map(str::to_string))
    }

    pub fn pointer_locked() -> bool {
        debug_flag("nolock") || document().and_then(|d| d.pointer_lock_element()).is_some()
    }

    pub fn want_lock(want: bool) {
        if let Some(body) = document().and_then(|d| d.body()) {
            let _ = body.dataset().set("wantLock", if want { "1" } else { "0" });
        }
        if !want && pointer_locked() {
            if let Some(d) = document() {
                d.exit_pointer_lock();
            }
        }
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    pub fn load(key: &str) -> Option<String> {
        storage()?
            .get_item(&format!("after-hours.{key}"))
            .ok()
            .flatten()
    }

    pub fn save(key: &str, value: &str) {
        if let Some(s) = storage() {
            let _ = s.set_item(&format!("after-hours.{key}"), value);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    static LOCKED: AtomicBool = AtomicBool::new(false);
    // Natively, progress lasts for the session only for now.
    static STORE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

    pub fn debug_flag(flag: &str) -> bool {
        std::env::var("AFTER_HOURS_DEBUG").is_ok_and(|v| v.split(',').any(|f| f == flag))
    }

    pub fn debug_value(name: &str) -> Option<String> {
        let all = std::env::var("AFTER_HOURS_DEBUG").ok()?;
        all.split(',')
            .find_map(|f| f.strip_prefix(name)?.strip_prefix('=').map(str::to_string))
    }

    pub fn pointer_locked() -> bool {
        debug_flag("nolock") || LOCKED.load(Ordering::Relaxed)
    }

    pub fn set_locked(locked: bool) {
        LOCKED.store(locked, Ordering::Relaxed);
    }

    pub fn load(key: &str) -> Option<String> {
        STORE.lock().ok()?.as_ref()?.get(key).cloned()
    }

    pub fn save(key: &str, value: &str) {
        if let Ok(mut store) = STORE.lock() {
            store
                .get_or_insert_with(HashMap::new)
                .insert(key.to_string(), value.to_string());
        }
    }
}

pub use imp::{debug_flag, debug_value, load, pointer_locked, save};

/// Furthest night reached, across sessions (1 if none).
pub fn load_best() -> u32 {
    load("best-night").and_then(|v| v.parse().ok()).unwrap_or(1)
}

pub fn save_best(night: u32) {
    save("best-night", &night.to_string());
}

/// What the cursor should be doing, remembered so it is only changed on
/// transitions.
#[derive(Resource, Default)]
pub struct CursorState {
    wanted: Option<bool>,
}

/// Grabs the mouse while on the floor and frees it on menus.
pub fn sync_cursor(
    _: NonSendMarker,
    game: Res<Game>,
    mut state: ResMut<CursorState>,
    window: Res<Window>,
) {
    // The title screen wants the lock too, so the click that starts the
    // night also captures the mouse.
    let want = matches!(
        game.phase,
        Phase::Title | Phase::Intro | Phase::Playing | Phase::Dead | Phase::Escaped
    ) && !(game.phase == Phase::Playing && game.paused && !IS_WEB);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = &window;
        if state.wanted != Some(want) {
            imp::want_lock(want);
            state.wanted = Some(want);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Natively the lock follows the phase directly: only on the floor.
        let want = game.phase == Phase::Playing && !game.paused && want;
        if state.wanted != Some(want) {
            use winit::window::CursorGrabMode;
            let handle = &window.window_handle;
            if want {
                let _ = handle
                    .set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| handle.set_cursor_grab(CursorGrabMode::Confined));
            } else {
                let _ = handle.set_cursor_grab(CursorGrabMode::None);
            }
            handle.set_cursor_visible(!want);
            imp::set_locked(want);
            state.wanted = Some(want);
        }
    }
}

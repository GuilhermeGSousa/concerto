pub use time::Instant;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "wasm32")] {
        use web_time as time;
    } else {
        use std::time;
    }
}

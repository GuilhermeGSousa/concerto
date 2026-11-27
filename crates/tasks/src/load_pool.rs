use std::sync::OnceLock;

use crate::task_pool::TaskPool;

static LOAD_TASK_POOL: OnceLock<TaskPool> = OnceLock::new();

/// The process-wide [`TaskPool`] that loads assets, kept apart from the compute pool so a slow load
/// never stalls a frame.
pub struct LoadTaskPool;

impl LoadTaskPool {
    /// Returns the load pool, creating it with `f` on the first call; later calls ignore `f`.
    pub fn get_or_init<F>(f: F) -> &'static TaskPool
    where
        F: FnOnce() -> TaskPool,
    {
        LOAD_TASK_POOL.get_or_init(f)
    }
}

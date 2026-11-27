use std::sync::OnceLock;

use crate::task_pool::TaskPool;

static COMPUTE_TASK_POOL: OnceLock<TaskPool> = OnceLock::new();

/// The process-wide [`TaskPool`] that runs systems in parallel.
pub struct ComputeTaskPool;

impl ComputeTaskPool {
    /// Returns the compute pool, creating it with `f` on the first call; later calls ignore `f`.
    pub fn get_or_init<F>(f: F) -> &'static TaskPool
    where
        F: FnOnce() -> TaskPool,
    {
        COMPUTE_TASK_POOL.get_or_init(f)
    }
}

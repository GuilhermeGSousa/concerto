use crate::task::Task;
use std::future::Future;

/// A task pool that runs futures on the browser's event loop, since wasm has no worker threads.
pub struct TaskPool;

impl TaskPool {
    /// Creates the pool.
    pub fn new() -> Self {
        TaskPool
    }

    /// Name is ignored on wasm: there are no worker threads to label.
    pub fn with_name(_name: &str) -> Self {
        Self::new()
    }

    /// Spawns `future` onto the browser's event loop and returns its [`Task`].
    pub fn spawn<T>(&self, future: impl Future<Output = T> + 'static) -> Task<T>
    where
        T: Send + 'static,
    {
        Task::new(future)
    }
}

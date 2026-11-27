//! Thread pools and tasks for running futures off the main thread.
//!
//! [`task_pool::TaskPool`] owns worker threads; [`compute_pool::ComputeTaskPool`] and
//! [`load_pool::LoadTaskPool`] are the process-wide pools the ECS executor and the asset server
//! use. On wasm there are no worker threads and every future runs on the browser's event loop.

#[cfg_attr(target_arch = "wasm32", path = "wasm_task.rs")]
mod task;

pub use task::Task;

/// Executors that run their tasks only on the thread that created them.
#[cfg(not(target_arch = "wasm32"))]
pub mod thread_executor;

/// The process-wide pool for parallel system execution.
pub mod compute_pool;
/// The process-wide pool for asset loading.
pub mod load_pool;

/// A pool of worker threads that run spawned futures, and scopes that borrow from the caller.
#[cfg_attr(target_arch = "wasm32", path = "wasm_task_pool.rs")]
pub mod task_pool;

#[cfg_attr(target_arch = "wasm32", path = "wasm_task.rs")]
mod task;

pub use task::Task;

#[cfg(not(target_arch = "wasm32"))]
pub mod thread_executor;

pub mod compute_pool;
pub mod load_pool;

#[cfg_attr(target_arch = "wasm32", path = "wasm_task_pool.rs")]
pub mod task_pool;

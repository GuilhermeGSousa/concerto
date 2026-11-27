use crate::{World, system::schedule::CompiledScheduleData};

/// [`MultiThreadedExecutor`](multi_thread::MultiThreadedExecutor), which runs systems in parallel
/// on the compute pool.
#[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
pub mod multi_thread;
/// [`SingleThreadedExecutor`](single_thread::SingleThreadedExecutor), which runs systems one at a
/// time in schedule order.
pub mod single_thread;

/// Runs the systems of a compiled schedule on a world.
pub trait SystemExecutor: Send + Sync {
    /// Creates the executor for `compiled_data`, when the schedule is compiled.
    fn init(compiled_data: &CompiledScheduleData) -> Self
    where
        Self: Sized;

    /// Runs every system of `compiled_data` once on `world`, applying their commands as the
    /// schedule's sync points require.
    fn run(&mut self, compiled_data: &mut CompiledScheduleData, world: &mut World);
}

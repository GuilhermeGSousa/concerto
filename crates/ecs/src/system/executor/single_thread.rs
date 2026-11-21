use crate::{
    World,
    system::{executor::SystemExecutor, schedule::CompiledScheduleData},
};

/// Runs every system on the calling thread, one at a time, in dependency order.
pub struct SingleThreadedExecutor {}

impl SystemExecutor for SingleThreadedExecutor {
    fn init(_compiled_data: &CompiledScheduleData) -> Self
    where
        Self: Sized,
    {
        Self {}
    }

    fn run(&mut self, compiled_data: &mut CompiledScheduleData, world: &mut World) {
        for &idx in &compiled_data.sorted_systems {
            profiling::scope!(compiled_data.systems[idx].name());
            compiled_data.systems[idx].run_and_apply((), world);
        }
    }
}

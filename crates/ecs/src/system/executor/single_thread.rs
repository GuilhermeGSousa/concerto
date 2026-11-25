use fixedbitset::FixedBitSet;

use crate::{
    World,
    system::{BoxedSystem, executor::SystemExecutor, schedule::CompiledScheduleData},
};

/// Runs every system on the calling thread, one at a time, in dependency order.
pub struct SingleThreadedExecutor {
    unapplied_systems: FixedBitSet,
}

impl SystemExecutor for SingleThreadedExecutor {
    fn init(compiled_data: &CompiledScheduleData) -> Self
    where
        Self: Sized,
    {
        Self {
            unapplied_systems: FixedBitSet::with_capacity(compiled_data.systems.len()),
        }
    }

    fn run(&mut self, compiled_data: &mut CompiledScheduleData, world: &mut World) {
        for idx in 0..compiled_data.systems.len() {
            let access = &compiled_data.system_access[idx];
            if access.is_exclusive() {
                apply_deferred(
                    &mut compiled_data.systems,
                    &mut self.unapplied_systems,
                    world,
                );
            }
            profiling::scope!(compiled_data.systems[idx].name());
            compiled_data.systems[idx].run((), world);
            if access.needs_apply() {
                self.unapplied_systems.insert(idx);
            }
        }
        apply_deferred(
            &mut compiled_data.systems,
            &mut self.unapplied_systems,
            world,
        );
    }
}

fn apply_deferred(systems: &mut [BoxedSystem], unapplied: &mut FixedBitSet, world: &mut World) {
    profiling::scope!("apply_deferred");
    for index in unapplied.ones() {
        systems[index].apply(world);
    }
    unapplied.clear();
}

use crate::{
    component::SystemTicks,
    resource::Resource,
    system::{
        access::SystemAccess,
        input::{SystemInput, ValidationError},
        meta::SystemMetadata,
        schedule::ScheduleLabel,
    },
    world::{UnsafeWorldCell, World},
};

#[derive(Resource)]
pub(crate) struct GateOpen(pub bool);

pub(crate) struct Gate;

impl SystemInput for Gate {
    type State = ();
    type Data<'world, 'state> = Gate;

    fn init_state(_world: &mut World) -> Self::State {}

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: reads only `GateOpen`, which `fill_access` declares.
        let open = unsafe { world.world() }
            .get_resource::<GateOpen>()
            .is_some_and(|open| open.0);
        if open {
            Ok(Gate)
        } else {
            Err(ValidationError::skip::<Self>("the gate is closed"))
        }
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_resource::<GateOpen>();
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Outer;

impl ScheduleLabel for Outer {
    fn dyn_clone(&self) -> Box<dyn ScheduleLabel> {
        Box::new(self.clone())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Inner;

impl ScheduleLabel for Inner {
    fn dyn_clone(&self) -> Box<dyn ScheduleLabel> {
        Box::new(self.clone())
    }
}

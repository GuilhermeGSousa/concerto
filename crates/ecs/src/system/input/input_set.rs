use std::{
    any::{Any, TypeId},
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
};

use crate::{
    component::SystemTicks,
    resource::Resource,
    system::{access::SystemAccess, meta::SystemMetadata},
    world::{UnsafeWorldCell, World},
};

use super::{ReadOnlySystemInput, SystemInput, ValidationError};

type ErasedState = Box<dyn Any + Send + Sync>;

#[derive(Clone, Copy)]
struct Member {
    id: TypeId,
    init_state: fn(&mut World) -> ErasedState,
    fill_access: fn(&(dyn Any + Send + Sync), &mut SystemMetadata, &mut SystemAccess),
    fetch: unsafe fn(
        &mut (dyn Any + Send + Sync),
        UnsafeWorldCell<'_>,
        SystemTicks,
    ) -> Result<Box<dyn Any>, ValidationError>,
    ran: fn(&mut (dyn Any + Send + Sync)),
}

impl Member {
    fn of<P: ReadOnlySystemInput + 'static>() -> Self {
        Self {
            id: TypeId::of::<P>(),
            init_state: |world| Box::new(P::init_state(world)),
            fill_access: |state, meta, access| {
                P::fill_state_access(typed::<P>(state), meta, access)
            },
            fetch: fetch::<P>,
            ran: |state| P::ran(typed_mut::<P>(state)),
        }
    }
}

fn typed<P: SystemInput + 'static>(state: &(dyn Any + Send + Sync)) -> &P::State {
    state
        .downcast_ref::<P::State>()
        .expect("an input set member keeps the state of its own input")
}

fn typed_mut<P: SystemInput + 'static>(state: &mut (dyn Any + Send + Sync)) -> &mut P::State {
    state
        .downcast_mut::<P::State>()
        .expect("an input set member keeps the state of its own input")
}

/// # Safety
/// The caller must hold `P`'s declared access, and the value must not outlive the run that fetched it.
unsafe fn fetch<P: ReadOnlySystemInput + 'static>(
    state: &mut (dyn Any + Send + Sync),
    world: UnsafeWorldCell<'_>,
    ticks: SystemTicks,
) -> Result<Box<dyn Any>, ValidationError> {
    // SAFETY: the caller holds `P`'s access, which the input set declared for its system.
    let data = ManuallyDrop::new(unsafe { P::get_data(typed_mut::<P>(state), world, ticks) }?);
    // SAFETY: only lifetimes differ, and `Inputs::get` hands the value out with the run's own lifetimes.
    let data: P::Data<'static, 'static> =
        unsafe { std::ptr::read((&*data as *const P::Data<'_, '_>).cast()) };
    Ok(Box::new(data))
}

struct InputSetRegistry<K> {
    members: Vec<Member>,
    marker: PhantomData<fn() -> K>,
}

impl<K: 'static> Resource for InputSetRegistry<K> {
    fn name() -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Makes `P` a member of every `InputSet<K>` initialized afterwards; registering it again changes nothing.
pub fn register_input<K: 'static, P: ReadOnlySystemInput + 'static>(world: &mut World) {
    let registry = world.get_resource_or_insert_with(|| InputSetRegistry::<K> {
        members: Vec::new(),
        marker: PhantomData,
    });
    if registry
        .members
        .iter()
        .all(|member| member.id != TypeId::of::<P>())
    {
        registry.members.push(Member::of::<P>());
    }
}

/// The state of an `InputSet<K>`: every member registered under `K` when its system was initialized.
pub struct InputSetState<K> {
    members: Vec<(Member, ErasedState)>,
    marker: PhantomData<fn() -> K>,
}

/// A read-only input whose members were registered under `K` at app build time; it declares their union.
pub struct InputSet<'w, 's, K> {
    inputs: Inputs<'w, 's>,
    marker: PhantomData<fn() -> K>,
}

impl<'w, 's, K> Deref for InputSet<'w, 's, K> {
    type Target = Inputs<'w, 's>;

    fn deref(&self) -> &Self::Target {
        &self.inputs
    }
}

/// The members of an input set fetched for one run, handed out by type.
pub struct Inputs<'w, 's> {
    fetched: Vec<(TypeId, Result<Box<dyn Any>, ValidationError>)>,
    marker: PhantomData<(&'w (), &'s ())>,
}

impl<'w, 's> Inputs<'w, 's> {
    /// The data of member `P`; a member that is not registered declines with `Skip`, a missing resource with `Missing`.
    pub fn get<P: ReadOnlySystemInput + 'static>(
        &self,
    ) -> Result<&P::Data<'w, 's>, ValidationError> {
        let Some((_, fetched)) = self.fetched.iter().find(|(id, _)| *id == TypeId::of::<P>())
        else {
            return Err(ValidationError::skip::<P>(
                "the input is not a member of this input set",
            ));
        };
        let data = fetched
            .as_ref()
            .map_err(Clone::clone)?
            .downcast_ref::<P::Data<'static, 'static>>()
            .expect("an input set member keeps the data of its own input");
        // SAFETY: the value was fetched for this run with lifetimes 'w and 's, and stored with them erased.
        Ok(unsafe { &*(data as *const P::Data<'static, 'static>).cast::<P::Data<'w, 's>>() })
    }
}

impl<'a, 'b, K: 'static> SystemInput for InputSet<'a, 'b, K> {
    type State = InputSetState<K>;
    type Data<'world, 'state> = InputSet<'world, 'state, K>;

    fn init_state(world: &mut World) -> Self::State {
        let members = world
            .get_resource::<InputSetRegistry<K>>()
            .map(|registry| registry.members.clone())
            .unwrap_or_default();
        InputSetState {
            members: members
                .into_iter()
                .map(|member| (member, (member.init_state)(world)))
                .collect(),
            marker: PhantomData,
        }
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        let fetched = state
            .members
            .iter_mut()
            .map(|(member, state)| {
                // SAFETY: `fill_state_access` declared every member's access for this system.
                (member.id, unsafe {
                    (member.fetch)(&mut **state, world, ticks)
                })
            })
            .collect();
        Ok(InputSet {
            inputs: Inputs {
                fetched,
                marker: PhantomData,
            },
            marker: PhantomData,
        })
    }

    fn ran(state: &mut Self::State) {
        for (member, state) in &mut state.members {
            (member.ran)(&mut **state);
        }
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_world();
    }

    fn fill_state_access(
        state: &Self::State,
        meta: &mut SystemMetadata,
        access: &mut SystemAccess,
    ) {
        for (member, state) in &state.members {
            (member.fill_access)(&**state, meta, access);
        }
    }
}

impl<K: 'static> ReadOnlySystemInput for InputSet<'_, '_, K> {}

#[cfg(test)]
mod tests {
    use crate::{
        Res, ResMut, Resource, System, World,
        system::executor::single_thread::SingleThreadedExecutor,
        system::{
            IntoSystem, access::SystemAccess, input::ValidationKind, meta::SystemMetadata,
            schedule::Schedule,
        },
    };

    use super::{InputSet, register_input};
    use crate::system::input::InputState;

    struct Phase;

    #[derive(Resource)]
    struct First(u32);

    #[derive(Resource)]
    struct Second(u32);

    #[derive(Resource)]
    struct Other;

    #[derive(Resource, Default)]
    struct Seen(Vec<String>);

    fn reads(_: InputSet<Phase>) {}

    fn access_of<M>(world: &mut World, system: impl IntoSystem<(), M>) -> SystemAccess {
        let mut system = system.into_system();
        system.initialize(world);
        let mut access = SystemAccess::default();
        system.fill_access(&mut SystemMetadata::default(), &mut access);
        access
    }

    fn registered() -> World {
        let mut world = World::new();
        register_input::<Phase, Res<'static, First>>(&mut world);
        register_input::<Phase, Res<'static, Second>>(&mut world);
        world
    }

    #[test]
    fn an_input_set_declares_the_union_of_its_members() {
        let mut world = registered();
        let set = access_of(&mut world, reads);

        let first = access_of(&mut world, |_: ResMut<First>| {});
        let second = access_of(&mut world, |_: ResMut<Second>| {});
        let other = access_of(&mut world, |_: ResMut<Other>| {});
        assert!(!SystemAccess::are_disjoint(&set, &first));
        assert!(!SystemAccess::are_disjoint(&set, &second));
        assert!(SystemAccess::are_disjoint(&set, &other));
    }

    #[test]
    fn an_input_set_not_yet_initialised_declares_a_read_of_the_world() {
        let system = reads.into_system();
        let mut access = SystemAccess::default();
        system.fill_access(&mut SystemMetadata::default(), &mut access);

        assert!(access.reads_world());
        assert!(!access.is_exclusive());
    }

    fn record(set: InputSet<Phase>, mut seen: ResMut<Seen>) {
        let entry = match set.get::<Res<'static, First>>() {
            Ok(first) => format!("first {}", first.0),
            Err(error) => format!("{:?}", error.kind),
        };
        seen.0.push(entry);
        let entry = match set.get::<Res<'static, Other>>() {
            Ok(_) => "other".to_string(),
            Err(error) => format!("{:?}", error.kind),
        };
        seen.0.push(entry);
        if let (Ok(first), Ok(second)) = (
            set.get::<Res<'static, First>>(),
            set.get::<Res<'static, Second>>(),
        ) {
            seen.0.push(format!("both {}", first.0 + second.0));
        }
    }

    fn seen(world: &World) -> Vec<String> {
        world.get_resource::<Seen>().unwrap().0.clone()
    }

    #[test]
    fn a_member_is_fetched_by_its_type_and_an_unregistered_one_is_refused() {
        let mut world = registered();
        world.insert_resource(Seen::default());
        world.insert_resource(First(3));
        world.insert_resource(Second(4));
        let mut system = record.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);

        assert_eq!(
            seen(&world),
            [
                "first 3".to_string(),
                "Skip".to_string(),
                "both 7".to_string()
            ]
        );
    }

    #[test]
    fn a_member_whose_resource_is_missing_is_reported_missing() {
        let mut world = registered();
        world.insert_resource(Seen::default());
        let mut system = record.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);

        assert_eq!(seen(&world)[0], format!("{:?}", ValidationKind::Missing));
    }

    #[test]
    fn registering_the_same_input_twice_keeps_one_member() {
        let mut world = registered();
        register_input::<Phase, Res<'static, First>>(&mut world);
        world.insert_resource(Seen::default());
        world.insert_resource(First(1));
        world.insert_resource(Second(1));
        let mut system = record.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);

        assert_eq!(seen(&world)[0], "first 1");
    }

    #[test]
    fn a_schedule_runs_a_system_whose_input_set_was_registered_before_it_compiled() {
        let mut world = registered();
        world.insert_resource(Seen::default());
        world.insert_resource(First(5));
        world.insert_resource(Second(6));
        let mut schedule = Schedule::default();
        schedule.add_system(record);

        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(seen(&world)[2], "both 11");
    }

    #[test]
    fn an_input_set_of_another_marker_sees_none_of_these_members() {
        struct Elsewhere;
        let mut world = registered();
        let elsewhere = access_of(&mut world, |_: InputSet<Elsewhere>| {});
        let first = access_of(&mut world, |_: ResMut<First>| {});

        assert!(SystemAccess::are_disjoint(&elsewhere, &first));
    }

    #[test]
    fn an_input_state_fetches_an_input_set_outside_a_system() {
        let mut world = registered();
        world.insert_resource(First(2));
        world.insert_resource(Second(9));
        let mut state = InputState::<InputSet<Phase>>::new(&mut world);

        let set = state.get(&world).unwrap();
        let sum = set.get::<Res<'static, First>>().unwrap().0
            + set.get::<Res<'static, Second>>().unwrap().0;

        assert_eq!(sum, 11);
    }
}

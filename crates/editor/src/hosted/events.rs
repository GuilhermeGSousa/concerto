use std::collections::HashMap;

use concerto_app::{App, schedule_groups::First};
use concerto_ecs::{
    Entity, Event, IntoSystemConfig, Query, World,
    component::SystemTicks,
    events::{event_channel::EventChannel, event_cursor::EventCursor, event_writer::EventWriter},
    system::{
        access::SystemAccess,
        input::{ReadOnlySystemInput, SystemInput, SystemLocal, ValidationError},
        meta::SystemMetadata,
    },
    world::{FromWorld, UnsafeWorldCell},
    world_id::WorldId,
};

use super::{HostedApp, ParkedEditApp};

/// An event of a hosted `App`, re-written in the editor's world with the document it came from.
#[derive(Clone)]
pub struct Hosted<E> {
    pub document: Entity,
    pub event: E,
}

impl<E: Event> Event for Hosted<E> {}

/// One cursor per hosted world, so a cursor follows its `App` when it is parked or swapped.
pub struct HostedCursors<E: Event + 'static> {
    cursors: HashMap<WorldId, EventCursor<E>>,
}

impl<E: Event + 'static> FromWorld for HostedCursors<E> {
    fn from_world(_world: &World) -> Self {
        Self {
            cursors: HashMap::new(),
        }
    }
}

impl<E: Event + 'static> HostedCursors<E> {
    fn read<'w>(&mut self, world: &'w World) -> Vec<&'w E> {
        let Some(channel) = world.get_resource::<EventChannel<E>>() else {
            return Vec::new();
        };
        let cursor = self.cursors.entry(world.id()).or_default();
        channel.read_from(cursor).collect()
    }

    fn keep_only(&mut self, live: &[WorldId]) {
        self.cursors.retain(|world, _| live.contains(world));
    }
}

type Inputs<E> = (
    Query<'static, 'static, (Entity, &'static HostedApp)>,
    Query<'static, 'static, &'static ParkedEditApp>,
    SystemLocal<'static, HostedCursors<E>>,
);

/// Reads `E` from every hosted `App`, each event once, with the document it came from.
pub struct HostedAppEvents<'w, 's, E: Event + 'static> {
    shown: Query<'w, 's, (Entity, &'static HostedApp)>,
    parked: Query<'w, 's, &'static ParkedEditApp>,
    cursors: SystemLocal<'s, HostedCursors<E>>,
}

impl<'w, E: Event + 'static> HostedAppEvents<'w, '_, E> {
    /// Every `E` written since this system last read, in each `App`'s write order.
    pub fn read(&mut self) -> Vec<(Entity, &'w E)> {
        let mut read = Vec::new();
        let mut live = Vec::new();
        for (document, hosted) in self.shown.iter() {
            let world = hosted.world();
            live.push(world.id());
            read.extend(
                self.cursors
                    .read(world)
                    .into_iter()
                    .map(|event| (document, event)),
            );
        }
        live.extend(self.parked.iter().map(|parked| parked.0.world().id()));
        self.cursors.keep_only(&live);
        read
    }
}

impl<E: Event + 'static> SystemInput for HostedAppEvents<'_, '_, E> {
    type State = <Inputs<E> as SystemInput>::State;
    type Data<'world, 'state> = HostedAppEvents<'world, 'state, E>;

    fn init_state(world: &mut World) -> Self::State {
        <Inputs<E>>::init_state(world)
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares exactly the access of its parts.
        let (shown, parked, cursors) = unsafe { <Inputs<E>>::get_data(state, world, ticks) }?;
        Ok(HostedAppEvents {
            shown,
            parked,
            cursors,
        })
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        <Inputs<E>>::fill_access(meta, access);
    }
}

impl<E: Event + 'static> ReadOnlySystemInput for HostedAppEvents<'_, '_, E> {}

/// Re-writes every `E` the hosted `App`s wrote as a [`Hosted<E>`] in the editor's world.
pub(crate) fn forward_events<E: Event + Clone + 'static>(
    mut hosted: HostedAppEvents<E>,
    mut out: EventWriter<Hosted<E>>,
) {
    for (document, event) in hosted.read() {
        out.write(Hosted {
            document,
            event: event.clone(),
        });
    }
}

pub trait HostedEventsAppExt {
    /// Re-writes every `E` a hosted `App` writes as a [`Hosted<E>`] event in this `App`.
    fn forward_hosted_event<E: Event + Clone + 'static>(&mut self) -> &mut Self;
}

impl HostedEventsAppExt for App {
    fn forward_hosted_event<E: Event + Clone + 'static>(&mut self) -> &mut Self {
        if self.get_resource::<EventChannel<Hosted<E>>>().is_some() {
            return self;
        }
        self.register_event::<Hosted<E>>().add_system(
            First,
            forward_events::<E>.after(super::input::step_edit_target),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted::testing::*;
    use concerto_app::{main_schedule::MainSchedulePlugin, plugins::TimePlugin};
    use concerto_ecs::{IntoSystem, ResMut, Resource, System};

    #[derive(Resource, Default)]
    struct Heard(Vec<(Entity, u32)>);

    fn record(mut events: HostedAppEvents<Echo>, mut heard: ResMut<Heard>) {
        let mut read: Vec<(Entity, u32)> = events
            .read()
            .into_iter()
            .map(|(document, echo)| (document, echo.0))
            .collect();
        read.sort_by_key(|(document, said)| (document.index(), *said));
        heard.0.extend(read);
    }

    struct Reader {
        world: World,
        system: Box<dyn System<In = ()>>,
    }

    impl Reader {
        fn new() -> Self {
            let mut world = World::new();
            world.insert_resource(Heard::default());
            let mut system: Box<dyn System<In = ()>> = Box::new(record.into_system());
            system.initialize(&mut world);
            Self { world, system }
        }

        fn host(&mut self) -> Entity {
            self.world.spawn(hosted_now(plain_recipe, |_| {}))
        }

        fn say(&mut self, document: Entity, said: u32) {
            self.world
                .get_component_for_entity_mut::<HostedApp>(document)
                .unwrap()
                .trigger(Say(said));
        }

        fn read(&mut self) -> Vec<(Entity, u32)> {
            self.system.run_and_apply((), &mut self.world);
            std::mem::take(&mut self.world.get_resource_mut::<Heard>().unwrap().0)
        }
    }

    #[test]
    fn events_are_read_from_every_hosted_app_with_their_document() {
        let mut r = Reader::new();
        let first = r.host();
        let second = r.host();

        r.say(first, 1);
        r.say(second, 2);
        r.say(first, 3);

        assert_eq!(r.read(), vec![(first, 1), (first, 3), (second, 2)]);
    }

    #[test]
    fn each_event_is_read_once() {
        let mut r = Reader::new();
        let document = r.host();
        r.say(document, 1);
        assert_eq!(r.read(), vec![(document, 1)]);

        assert!(r.read().is_empty());

        r.say(document, 2);
        assert_eq!(r.read(), vec![(document, 2)]);
    }

    #[test]
    fn a_cursor_follows_its_app_through_parking_and_swapping() {
        let mut r = Reader::new();
        let document = r.host();
        r.say(document, 1);
        assert_eq!(r.read(), vec![(document, 1)]);

        let play = hosted_now(plain_recipe, |_| {});
        let edit = std::mem::replace(
            &mut *r
                .world
                .get_component_for_entity_mut::<HostedApp>(document)
                .unwrap(),
            play,
        );
        r.world.insert(ParkedEditApp(edit), document);
        r.say(document, 10);
        assert_eq!(r.read(), vec![(document, 10)]);

        for (mut shown, mut parked) in r
            .world
            .query::<(&mut HostedApp, &mut ParkedEditApp), ()>()
            .iter(&mut r.world)
        {
            std::mem::swap(&mut **shown, &mut parked.0);
        }
        r.world.remove_component::<ParkedEditApp>(document);
        r.say(document, 2);
        assert_eq!(r.read(), vec![(document, 2)]);
    }

    #[test]
    fn only_the_cursors_of_live_worlds_are_kept() {
        let mut cursors = HostedCursors::<Echo>::from_world(&World::new());
        let mut kept = World::new();
        kept.insert_resource(EventChannel::<Echo>::default());
        let mut gone = World::new();
        gone.insert_resource(EventChannel::<Echo>::default());
        cursors.read(&kept);
        cursors.read(&gone);

        cursors.keep_only(&[kept.id()]);

        assert_eq!(cursors.cursors.len(), 1);
        assert!(cursors.cursors.contains_key(&kept.id()));
    }

    fn hosted_echoes(app: &App) -> Vec<(Entity, u32)> {
        let mut cursor = EventCursor::default();
        app.get_resource::<EventChannel<Hosted<Echo>>>()
            .unwrap()
            .read_from(&mut cursor)
            .map(|hosted| (hosted.document, hosted.event.0))
            .collect()
    }

    #[test]
    fn a_forwarded_type_is_rewritten_once_even_when_registered_twice() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .forward_hosted_event::<Echo>()
            .forward_hosted_event::<Echo>();
        let document = app
            .main_mut()
            .world_mut()
            .spawn(hosted_now(plain_recipe, |_| {}));
        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<HostedApp>(document)
            .unwrap()
            .trigger(Say(4));
        app.finish_plugin_build();

        app.update();

        assert_eq!(hosted_echoes(&app), vec![(document, 4)]);
    }
}

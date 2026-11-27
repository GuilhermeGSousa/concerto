//! Events of hosted `App`s, read from the editor's world: [`HostedAppEvents`] reads them in place
//! and [`HostedEventsAppExt::forward_hosted_event`] re-writes them as [`Hosted`] events.
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

/// An event about a document: re-written in the editor's world from its hosted `App`, or written there directly for a value it holds.
#[derive(Clone)]
pub struct Hosted<E> {
    /// The document whose `App` wrote the event, or that holds the value it is about.
    pub document: Entity,
    /// The event itself.
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
        for (document, hosted) in self.shown.iter_inner() {
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

/// Forwards events from hosted `App`s into the editor's `App`.
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
            forward_events::<E>.after(super::input::step_viewport_target),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted::testing::*;
    use concerto_ecs::{IntoSystem, ResMut, Resource, System};

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Said {
        document: Entity,
        said: u32,
    }

    #[derive(Resource, Default)]
    struct Heard(Vec<Said>);

    fn record(mut events: HostedAppEvents<Echo>, mut heard: ResMut<Heard>) {
        let mut read: Vec<Said> = events
            .read()
            .into_iter()
            .map(|(document, echo)| Said {
                document,
                said: echo.0,
            })
            .collect();
        read.sort_by_key(|said| (said.document.index(), said.said));
        heard.0.extend(read);
    }

    #[test]
    fn events_are_read_once_from_every_hosted_app_with_their_document_through_play_and_stop() {
        let mut world = World::new();
        world.insert_resource(Heard::default());
        let mut system = record.into_system();
        system.initialize(&mut world);
        let mut read = |world: &mut World| {
            system.run_and_apply((), world);
            std::mem::take(&mut world.get_resource_mut::<Heard>().unwrap().0)
        };
        let say = |world: &mut World, document: Entity, said: u32| {
            world
                .get_component_for_entity_mut::<HostedApp>(document)
                .unwrap()
                .trigger(Say(said));
        };
        let first = world.spawn(hosted_now(plain_recipe, |_| {}));
        let second = world.spawn(hosted_now(plain_recipe, |_| {}));

        say(&mut world, first, 1);
        say(&mut world, second, 2);
        say(&mut world, first, 3);
        let said = |document, said| Said { document, said };
        assert_eq!(
            read(&mut world),
            [said(first, 1), said(first, 3), said(second, 2)]
        );
        assert!(read(&mut world).is_empty());

        let edit = std::mem::replace(
            &mut *world
                .get_component_for_entity_mut::<HostedApp>(first)
                .unwrap(),
            hosted_now(plain_recipe, |_| {}),
        );
        world.insert(ParkedEditApp(edit), first);
        say(&mut world, first, 10);
        assert_eq!(read(&mut world), [said(first, 10)]);

        let edit = std::mem::replace(
            &mut world
                .get_component_for_entity_mut::<ParkedEditApp>(first)
                .unwrap()
                .0,
            hosted_now(plain_recipe, |_| {}),
        );
        world.remove_component::<ParkedEditApp>(first);
        *world
            .get_component_for_entity_mut::<HostedApp>(first)
            .unwrap() = edit;
        say(&mut world, first, 4);
        assert_eq!(read(&mut world), [said(first, 4)]);
    }
}

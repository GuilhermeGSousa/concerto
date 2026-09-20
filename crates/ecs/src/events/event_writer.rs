use crate::{
    World,
    events::{Event, event_channel::EventChannel},
    resource::ResMut,
    system::{input::SystemInput, meta::SystemMetadata},
    world::UnsafeWorldCell,
};

/// System parameter for sending events of type `T`.
///
/// Obtain one as a system parameter and call [`write`](EventWriter::write) to enqueue an event.
/// The event will be available to all [`EventReader`](super::event_reader::EventReader)s until
/// the end of the current frame.
///
/// # Example
/// ```ignore
/// fn player_died(mut writer: EventWriter<PlayerDied>) {
///     writer.write(PlayerDied { score: 42 });
/// }
/// ```
pub struct EventWriter<'world, T: Event + 'static> {
    channel: ResMut<'world, EventChannel<T>>,
}

impl<'w, T: Event> EventWriter<'w, T> {
    pub fn new(world: UnsafeWorldCell<'w>) -> Self {
        Self {
            channel: ResMut::new(world),
        }
    }

    /// Enqueues `event` so that [`EventReader`](super::event_reader::EventReader)s can read it
    /// this frame.
    pub fn write(&mut self, event: T) {
        self.channel.push_event(event);
    }
}

impl<'w, T> SystemInput for EventWriter<'w, T>
where
    T: Event,
{
    type State = ();
    type Data<'world, 'state> = EventWriter<'world, T>;

    fn init_state(_world: &mut World) -> Self::State {}

    fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        EventWriter::new(world)
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.write_resource::<EventChannel<T>>();
    }
}

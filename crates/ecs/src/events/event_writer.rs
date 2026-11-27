use crate::{
    World,
    component::SystemTicks,
    events::{Event, event_channel::EventChannel},
    resource::ResMut,
    system::{
        input::{SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};

/// System parameter for sending events of type `T`.
///
/// Obtain one as a system parameter and call [`write`](EventWriter::write) to enqueue an event.
/// The event stays readable by every [`EventReader`](super::event_reader::EventReader) until the
/// second buffer swap after it was written, so a reader that runs before the writer in one
/// frame still sees it in the next.
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
    /// Borrows `T`'s channel from `world` for a run with `ticks`, or reports it missing.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] of kind `Missing` when `T` was never registered, so its
    /// [`EventChannel`] does not exist.
    ///
    /// # Safety
    /// The caller must hold write access to `T`'s [`EventChannel`].
    pub unsafe fn new(
        world: UnsafeWorldCell<'w>,
        ticks: SystemTicks,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            // SAFETY: the caller holds write access to the channel.
            channel: unsafe { ResMut::new(world, ticks) }?,
        })
    }

    /// Enqueues `event` for every [`EventReader`](super::event_reader::EventReader) of `T`.
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

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: crate::world::UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares write access to the channel.
        unsafe { EventWriter::new(world, ticks) }
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        access.write_resource::<EventChannel<T>>();
    }
}

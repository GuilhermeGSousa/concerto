use std::{iter::Chain, slice::Iter};

use crate::{
    World,
    events::{Event, event_channel::EventChannel, event_cursor::EventCursor},
    resource::Res,
    system::{
        input::{SystemInput, SystemLocal},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};

/// The system inputs an [`EventReader`] is built from; its state is theirs.
type ChannelParam<T> = Res<'static, EventChannel<T>>;
type CursorParam<T> = SystemLocal<'static, EventCursor<T>>;

/// System parameter for reading events of type `T`.
///
/// Each system reading `T` keeps its own cursor, so [`read`](EventReader::read) yields every
/// event exactly once regardless of where the system sits in the schedule. Events written
/// after this system has already run in a frame are picked up on the next frame, as long as
/// the system reads at least once every frame — going two frames without reading silently
/// skips whatever expired in the meantime.
///
/// # Example
/// ```ignore
/// fn on_player_died(mut reader: EventReader<PlayerDied>) {
///     for event in reader.read() {
///         println!("Player died with score {}", event.score);
///     }
/// }
/// ```
pub struct EventReader<'world, 'state, T: Event + 'static> {
    channel: Res<'world, EventChannel<T>>,
    cursor: SystemLocal<'state, EventCursor<T>>,
}

impl<'w, 's, T: Event> EventReader<'w, 's, T> {
    /// Returns an iterator over every event not yet seen by this reader, and marks them read.
    pub fn read(&mut self) -> EventIterator<'_, T> {
        let last_event_count =
            std::mem::replace(&mut self.cursor.last_event_count, self.channel.event_count);
        EventIterator::new(&self.channel, last_event_count)
    }

    /// Number of events this reader has not read yet.
    pub fn len(&self) -> usize {
        self.channel.event_count - self.cursor.last_event_count.max(oldest_id(&self.channel))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Marks all buffered events as read without returning them.
    pub fn clear(&mut self) {
        self.cursor.last_event_count = self.channel.event_count;
    }
}

fn oldest_id<T: Event>(channel: &EventChannel<T>) -> usize {
    channel.events_a.start_event_count
}

impl<T> SystemInput for EventReader<'_, '_, T>
where
    T: Event + 'static,
{
    type State = <CursorParam<T> as SystemInput>::State;
    type Data<'world, 'state> = EventReader<'world, 'state, T>;

    fn init_state(world: &mut World) -> Self::State {
        <CursorParam<T>>::init_state(world)
    }

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        EventReader {
            channel: <ChannelParam<T>>::get_data(&mut (), world),
            cursor: <CursorParam<T>>::get_data(state, world),
        }
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut crate::system::access::SystemAccess) {
        <ChannelParam<T>>::fill_access(meta, access);
        <CursorParam<T>>::fill_access(meta, access);
    }
}

pub struct EventIterator<'a, T: Event + 'static> {
    iter: Chain<Iter<'a, T>, Iter<'a, T>>,
}

impl<'a, T: Event + 'static> EventIterator<'a, T> {
    pub(crate) fn new(channel: &'a EventChannel<T>, last_event_count: usize) -> Self {
        Self {
            iter: channel
                .events_a
                .unread_from(last_event_count)
                .iter()
                .chain(channel.events_b.unread_from(last_event_count).iter()),
        }
    }
}

impl<'a, T: Event + 'static> Iterator for EventIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

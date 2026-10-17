use crate::{
    events::{Event, event_cursor::EventCursor, event_reader::EventIterator},
    resource::{ResMut, Resource},
};

/// One half of an [`EventChannel`]'s double buffer.
///
/// `start_event_count` is the id of the first event this buffer would hold, so a
/// reader can turn an absolute event id into an index without scanning.
pub(crate) struct EventSequence<T: Event + 'static> {
    pub(crate) events: Vec<T>,
    pub(crate) start_event_count: usize,
}

impl<T: Event + 'static> EventSequence<T> {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            start_event_count: 0,
        }
    }

    /// Returns the events with an id at or after `last_event_count`.
    ///
    /// A reader that has not run for several frames is clamped forward to the
    /// oldest event still buffered rather than panicking.
    pub(crate) fn unread_from(&self, last_event_count: usize) -> &[T] {
        let index = last_event_count.saturating_sub(self.start_event_count);
        self.events.get(index..).unwrap_or(&[])
    }
}

/// Internal storage for a single event type.
///
/// Events live in a double buffer: writes land in the newer buffer, and
/// [`update`](EventChannel::update) swaps the two and clears the older one. An event
/// therefore survives two `update` calls, so a reader that runs before the writer in
/// one frame still sees the event in the next. Readers track their own position with
/// an [`EventCursor`](super::event_cursor::EventCursor), so an event is never read twice.
///
/// Prefer the higher-level [`EventWriter`](super::event_writer::EventWriter) and
/// [`EventReader`](super::event_reader::EventReader) in system code.
#[derive(Resource)]
pub struct EventChannel<T: Event + 'static> {
    pub(crate) events_a: EventSequence<T>,
    pub(crate) events_b: EventSequence<T>,
    /// Number of events ever written; doubles as the id of the next event.
    pub(crate) event_count: usize,
}

impl<T: Event + 'static> EventChannel<T> {
    /// Creates an empty channel.
    pub fn new() -> Self {
        EventChannel {
            events_a: EventSequence::new(),
            events_b: EventSequence::new(),
            event_count: 0,
        }
    }

    /// Enqueues `event`, returning the id it was given.
    pub fn push_event(&mut self, event: T) -> usize {
        let id = self.event_count;
        self.events_b.events.push(event);
        self.event_count += 1;
        id
    }

    /// Returns the events written since `cursor` last read, and moves it past them.
    pub fn read_from<'a>(&'a self, cursor: &mut EventCursor<T>) -> EventIterator<'a, T> {
        let unread = EventIterator::new(self, cursor.last_event_count);
        cursor.last_event_count = self.event_count;
        unread
    }

    /// Swaps the buffers and drops the events that have now expired.
    ///
    /// Must be called once per frame per event type, or the buffers grow forever.
    pub fn update(&mut self) {
        std::mem::swap(&mut self.events_a, &mut self.events_b);
        self.events_b.events.clear();
        self.events_b.start_event_count = self.event_count;
    }

    /// Number of events currently buffered across both halves.
    pub fn len(&self) -> usize {
        self.events_a.events.len() + self.events_b.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Event + 'static> Default for EventChannel<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// System that advances an event channel's double buffer once per frame.
///
/// Registered automatically by [`App::register_event`].
pub fn update_event_channel<T: Event + 'static>(mut channel: ResMut<EventChannel<T>>) {
    channel.update();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Event, Debug, PartialEq, Clone)]
    struct Note(u32);

    fn read(channel: &EventChannel<Note>, cursor: &mut EventCursor<Note>) -> Vec<u32> {
        channel.read_from(cursor).map(|note| note.0).collect()
    }

    #[test]
    fn a_held_cursor_reads_each_event_once() {
        let mut channel = EventChannel::<Note>::new();
        let mut cursor = EventCursor::default();
        channel.push_event(Note(1));
        channel.push_event(Note(2));

        assert_eq!(read(&channel, &mut cursor), vec![1, 2]);
        assert!(read(&channel, &mut cursor).is_empty());

        channel.push_event(Note(3));
        assert_eq!(read(&channel, &mut cursor), vec![3]);
    }

    #[test]
    fn an_event_written_between_two_updates_is_still_readable_after_the_next() {
        let mut channel = EventChannel::<Note>::new();
        let mut cursor = EventCursor::default();
        channel.update();
        channel.push_event(Note(7));
        channel.update();

        assert_eq!(read(&channel, &mut cursor), vec![7]);
    }
}

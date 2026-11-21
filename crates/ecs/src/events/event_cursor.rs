use std::marker::PhantomData;

use crate::{
    events::Event,
    world::{FromWorld, World},
};

/// A single reader's position in an event stream.
///
/// Held as a [`SystemLocal`](crate::system::input::SystemLocal) by
/// [`EventReader`](super::event_reader::EventReader), which is what gives every system an
/// independent view of the stream.
pub struct EventCursor<T: Event + 'static> {
    pub(crate) last_event_count: usize,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Event + 'static> Default for EventCursor<T> {
    fn default() -> Self {
        Self {
            last_event_count: 0,
            _marker: PhantomData,
        }
    }
}

impl<T: Event + 'static> Clone for EventCursor<T> {
    fn clone(&self) -> Self {
        Self {
            last_event_count: self.last_event_count,
            _marker: PhantomData,
        }
    }
}

impl<T: Event + 'static> FromWorld for EventCursor<T> {
    fn from_world(_world: &World) -> Self {
        Self::default()
    }
}

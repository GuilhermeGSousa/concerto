/// [`EventChannel`](event_channel::EventChannel), the resource that holds one event type's double
/// buffer.
pub mod event_channel;
/// [`EventCursor`](event_cursor::EventCursor), one reader's position in an event stream.
pub mod event_cursor;
/// [`EventReader`](event_reader::EventReader), the system parameter that reads events.
pub mod event_reader;
/// [`EventWriter`](event_writer::EventWriter), the system parameter that sends events.
pub mod event_writer;

pub use concerto_ecs_macros::Event;

/// Marker trait for messages that can be sent between systems.
///
/// Derive with `#[derive(Event)]`.  Register a new event type with the app's
/// `register_event` method before using it.
///
/// Send events from systems with [`EventWriter`](event_writer::EventWriter) and
/// read them with [`EventReader`](event_reader::EventReader).
///
/// Events are double-buffered: each one stays readable until the second buffer swap
/// (the app's `EventUpdate`, which runs first in every frame) after it was written, and
/// every reader sees it exactly once.
///
/// # Example
/// ```ignore
/// use concerto_ecs::events::Event;
///
/// #[derive(Event)]
/// struct PlayerDied { score: u32 }
/// ```
pub trait Event: Send + Sync {}

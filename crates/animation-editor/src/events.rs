//! The events a clip's tab shows and edits, read from and written back to the clip's event keys.
use concerto_animation::{
    clip::{AnimationClip, ClipEventData, ClipEventKey},
    event::AnimationEvent,
};
use concerto_editor::timeline::{frame_at, time_of_frame};
use concerto_name::Name;
use concerto_reflect::{ReflectedValue, apply_value, to_value};

use super::ClipDocument;

const TYPED_IS_READ_ONLY: &str = "Typed events are read-only in the editor";
const NO_NAME: &str = "An event needs a name";

/// An event as a clip tab shows and edits it.
#[derive(Clone, Debug, PartialEq)]
pub struct TabEvent {
    /// The event's time in the clip, in seconds.
    pub time: f32,
    /// The event's name, or its stored type and value.
    pub kind: TabEventKind,
}

/// What a tab event is: an `AnimationEvent` it can edit, or a typed event it keeps as stored.
#[derive(Clone, Debug, PartialEq)]
pub enum TabEventKind {
    /// An [`AnimationEvent`] with this name, which the tab edits.
    Named(Name),
    /// An event of any other type, read-only in the tab and saved as read.
    Typed {
        /// The event's registered type name.
        type_name: String,
        /// The event's value, as stored in the clip.
        value: ReflectedValue,
    },
}

fn animation_event_type() -> &'static str {
    std::any::type_name::<AnimationEvent>()
}

fn decoded_name(type_name: &str, value: &ReflectedValue) -> Option<Name> {
    if type_name != animation_event_type() {
        return None;
    }
    let mut event = AnimationEvent::default();
    let warnings = apply_value(&mut event, value);
    (warnings.is_empty() && !event.name.is_empty()).then_some(event.name)
}

impl TabEvent {
    /// Reads the event `key` holds.
    ///
    /// An [`AnimationEvent`] that decodes to a name becomes [`TabEventKind::Named`]; anything
    /// else is kept as stored in [`TabEventKind::Typed`].
    ///
    /// # Errors
    ///
    /// Returns a message naming the type and time when the key's stored value cannot be read.
    pub fn read(key: &ClipEventKey) -> Result<Self, String> {
        let type_name = key.event.type_name();
        let value = key.event.value().map_err(|error| {
            format!(
                "`{type_name}` at {:.2} s could not be read: {error}",
                key.time
            )
        })?;
        let kind = match decoded_name(type_name, &value) {
            Some(name) => TabEventKind::Named(name),
            None => TabEventKind::Typed {
                type_name: type_name.to_owned(),
                value: value.into_owned(),
            },
        };
        Ok(Self {
            time: key.time,
            kind,
        })
    }

    /// The event key a save writes for this event, holding stored data.
    ///
    /// # Errors
    ///
    /// Returns a message naming the event and time when a named event cannot be encoded.
    pub fn write(&self) -> Result<ClipEventKey, String> {
        let event = match &self.kind {
            TabEventKind::Named(name) => ClipEventData::Stored {
                type_name: animation_event_type().to_owned(),
                value: to_value(&AnimationEvent { name: *name }).map_err(|error| {
                    format!(
                        "`{name}` at {:.2} s could not be written: {error}",
                        self.time
                    )
                })?,
            },
            TabEventKind::Typed { type_name, value } => ClipEventData::Stored {
                type_name: type_name.clone(),
                value: value.clone(),
            },
        };
        Ok(ClipEventKey {
            time: self.time,
            event,
        })
    }

    /// What its marker reads: the name, or the last segment of the type name.
    pub fn label(&self) -> &str {
        match &self.kind {
            TabEventKind::Named(name) => name.as_str(),
            TabEventKind::Typed { type_name, .. } => {
                type_name.rsplit("::").next().unwrap_or(type_name.as_str())
            }
        }
    }

    /// Whether the editor keeps the event as stored rather than editing it.
    pub fn is_typed(&self) -> bool {
        matches!(self.kind, TabEventKind::Typed { .. })
    }
}

/// The tab events of `clip`, in its order.
///
/// # Errors
///
/// Returns the first event that [`TabEvent::read`] cannot read.
pub fn read_events(clip: &AnimationClip) -> Result<Vec<TabEvent>, String> {
    clip.events().iter().map(TabEvent::read).collect()
}

/// The event keys a save writes for `events`, in their order.
///
/// # Errors
///
/// Returns the first event that [`TabEvent::write`] cannot write.
pub fn write_events(events: &[TabEvent]) -> Result<Vec<ClipEventKey>, String> {
    events.iter().map(TabEvent::write).collect()
}

fn checked_name(name: &str) -> Result<Name, String> {
    if name.is_empty() {
        Err(NO_NAME.to_owned())
    } else {
        Ok(Name::new(name))
    }
}

impl ClipDocument {
    /// Appends an [`AnimationEvent`] named `name` at `time` and selects it.
    ///
    /// `time` is in seconds and snaps to the nearest frame when the clip has a frame rate.
    ///
    /// # Errors
    ///
    /// Returns a message when `name` is empty, when `time` is outside the clip, or when an
    /// event of that name already sits at the same frame.
    pub fn add_event(&mut self, name: &str, time: f32) -> Result<(), String> {
        let name = checked_name(name)?;
        let time = self.event_time(time)?;
        self.check_free(None, name, time)?;
        self.events.push(TabEvent {
            time,
            kind: TabEventKind::Named(name),
        });
        self.selected_event = Some(self.events.len() - 1);
        Ok(())
    }

    /// Moves the event at `index` to `time`.
    ///
    /// `time` is in seconds and snaps to the nearest frame when the clip has a frame rate.
    ///
    /// # Errors
    ///
    /// Returns a message when there is no event at `index`, when it is a typed event, when
    /// `time` is outside the clip, or when an event of the same name already sits at that frame.
    pub fn move_event(&mut self, index: usize, time: f32) -> Result<(), String> {
        let name = self.editable(index)?;
        let time = self.event_time(time)?;
        self.check_free(Some(index), name, time)?;
        self.events[index].time = time;
        Ok(())
    }

    /// Renames the event at `index`.
    ///
    /// # Errors
    ///
    /// Returns a message when there is no event at `index`, when it is a typed event, when
    /// `name` is empty, or when an event named `name` already sits at the same frame.
    pub fn rename_event(&mut self, index: usize, name: &str) -> Result<(), String> {
        self.editable(index)?;
        let name = checked_name(name)?;
        self.check_free(Some(index), name, self.events[index].time)?;
        self.events[index].kind = TabEventKind::Named(name);
        Ok(())
    }

    /// Removes the event at `index` and clears the selection.
    ///
    /// # Errors
    ///
    /// Returns a message when there is no event at `index` or when it is a typed event.
    pub fn remove_event(&mut self, index: usize) -> Result<(), String> {
        self.editable(index)?;
        self.events.remove(index);
        self.selected_event = None;
        Ok(())
    }

    fn editable(&self, index: usize) -> Result<Name, String> {
        match self.events.get(index).map(|event| &event.kind) {
            Some(TabEventKind::Named(name)) => Ok(*name),
            Some(TabEventKind::Typed { .. }) => Err(TYPED_IS_READ_ONLY.to_owned()),
            None => Err(format!("The clip has no event {index}")),
        }
    }

    fn event_time(&self, time: f32) -> Result<f32, String> {
        let duration = self.facts.duration;
        let refused = || format!("{time:.2} s is outside the clip's 0.00 to {duration:.2} s");
        if !time.is_finite() || time < 0.0 {
            return Err(refused());
        }
        match self.facts.rate {
            Some(rate) => {
                let frame = frame_at(time, rate);
                if frame > frame_at(duration, rate) {
                    return Err(refused());
                }
                Ok(time_of_frame(frame, rate).min(duration))
            }
            None if time > duration => Err(refused()),
            None => Ok(time),
        }
    }

    fn check_free(&self, moving: Option<usize>, name: Name, time: f32) -> Result<(), String> {
        let rate = self.facts.rate;
        let same_moment = |other: f32| match rate {
            Some(rate) => frame_at(other, rate) == frame_at(time, rate),
            None => other == time,
        };
        let taken = self.events.iter().enumerate().any(|(index, event)| {
            Some(index) != moving
                && event.kind == TabEventKind::Named(name)
                && same_moment(event.time)
        });
        if !taken {
            return Ok(());
        }
        Err(match rate {
            Some(rate) => format!("`{name}` is already at frame {}", frame_at(time, rate)),
            None => format!("`{name}` is already at {time:.2} s"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ClipFacts,
        tests::{FOOTSTEP, clip_of, footstep_value, named_key, typed_key},
    };

    fn named(time: f32, name: &str) -> TabEvent {
        TabEvent {
            time,
            kind: TabEventKind::Named(Name::new(name)),
        }
    }

    fn typed(time: f32) -> TabEvent {
        TabEvent {
            time,
            kind: TabEventKind::Typed {
                type_name: FOOTSTEP.to_owned(),
                value: footstep_value(),
            },
        }
    }

    fn frame(frame: u32) -> f32 {
        time_of_frame(frame, 30.0)
    }

    #[test]
    fn event_keys_read_as_named_animation_events_and_stored_others_and_write_back_the_same() {
        let mut clip = clip_of(1);
        clip.replace_events(vec![named_key(0.2, "footstep"), typed_key(0.4)])
            .unwrap();

        let events = read_events(&clip).unwrap();

        assert_eq!(events, vec![named(0.2, "footstep"), typed(0.4)]);
        let keys = write_events(&events).unwrap();
        assert_eq!(keys[0].event.type_name(), animation_event_type());
        let ClipEventData::Stored { type_name, value } = &keys[1].event else {
            panic!("a written key holds stored data");
        };
        assert_eq!(type_name, FOOTSTEP);
        assert_eq!(value, &footstep_value());
        let read: Result<Vec<TabEvent>, String> = keys.iter().map(TabEvent::read).collect();
        assert_eq!(read.unwrap(), events);
    }

    enum Edit {
        Add { name: &'static str, time: f32 },
        Move { index: usize, time: f32 },
        Rename { index: usize, name: &'static str },
        Remove { index: usize },
    }

    struct Row {
        edit: Edit,
        accepted: Option<Vec<TabEvent>>,
    }

    #[test]
    fn an_edit_snaps_to_a_frame_and_is_refused_on_a_typed_event_an_empty_name_a_time_outside_or_a_taken_frame()
     {
        let opened = || {
            vec![
                named(frame(3), "land"),
                typed(0.5),
                named(frame(12), "footstep"),
            ]
        };
        let with = |change: &dyn Fn(&mut Vec<TabEvent>)| {
            let mut events = opened();
            change(&mut events);
            Some(events)
        };
        let rows = [
            Row {
                edit: Edit::Add {
                    name: "jump",
                    time: 0.41,
                },
                accepted: with(&|events| events.push(named(frame(12), "jump"))),
            },
            Row {
                edit: Edit::Move {
                    index: 0,
                    time: 0.01,
                },
                accepted: with(&|events| events[0].time = 0.0),
            },
            Row {
                edit: Edit::Rename {
                    index: 0,
                    name: "jump",
                },
                accepted: with(&|events| events[0] = named(frame(3), "jump")),
            },
            Row {
                edit: Edit::Remove { index: 0 },
                accepted: with(&|events| {
                    events.remove(0);
                }),
            },
            Row {
                edit: Edit::Move {
                    index: 1,
                    time: 0.2,
                },
                accepted: None,
            },
            Row {
                edit: Edit::Add {
                    name: "",
                    time: 0.5,
                },
                accepted: None,
            },
            Row {
                edit: Edit::Add {
                    name: "jump",
                    time: 1.1,
                },
                accepted: None,
            },
            Row {
                edit: Edit::Add {
                    name: "footstep",
                    time: 0.405,
                },
                accepted: None,
            },
        ];
        for row in rows {
            let mut document = ClipDocument {
                facts: ClipFacts::of(&clip_of(1)),
                events: opened(),
                ..ClipDocument::default()
            };
            let result = match row.edit {
                Edit::Add { name, time } => document.add_event(name, time),
                Edit::Move { index, time } => document.move_event(index, time),
                Edit::Rename { index, name } => document.rename_event(index, name),
                Edit::Remove { index } => document.remove_event(index),
            };
            assert_eq!(result.is_ok(), row.accepted.is_some(), "{result:?}");
            assert_eq!(document.events, row.accepted.unwrap_or_else(opened));
        }
    }
}

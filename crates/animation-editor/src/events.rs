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
    pub time: f32,
    pub kind: TabEventKind,
}

/// What a tab event is: an `AnimationEvent` it can edit, or a typed event it keeps as stored.
#[derive(Clone, Debug, PartialEq)]
pub enum TabEventKind {
    Named(Name),
    Typed {
        type_name: String,
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
    /// The event `key` holds: `Named` for an `AnimationEvent` that decodes to a name, `Typed` as stored otherwise.
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
pub fn read_events(clip: &AnimationClip) -> Result<Vec<TabEvent>, String> {
    clip.events().iter().map(TabEvent::read).collect()
}

/// The event keys a save writes for `events`, in their order.
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
    /// Appends an `AnimationEvent` named `name` at `time`, snapped to a frame and clamped to the clip, and selects it.
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

    /// Moves the event at `index` to `time`, snapped to a frame and clamped to the clip.
    pub fn move_event(&mut self, index: usize, time: f32) -> Result<(), String> {
        let name = self.editable(index)?;
        let time = self.event_time(time)?;
        self.check_free(Some(index), name, time)?;
        self.events[index].time = time;
        Ok(())
    }

    /// Renames the event at `index`.
    pub fn rename_event(&mut self, index: usize, name: &str) -> Result<(), String> {
        self.editable(index)?;
        let name = checked_name(name)?;
        self.check_free(Some(index), name, self.events[index].time)?;
        self.events[index].kind = TabEventKind::Named(name);
        Ok(())
    }

    /// Removes the event at `index` and clears the selection.
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
        tests::{FOOTSTEP, clip_of, footstep_value, named_key, stored_key, typed_key},
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

    fn clip_with(keys: Vec<ClipEventKey>) -> AnimationClip {
        let mut clip = clip_of(1);
        clip.replace_events(keys).unwrap();
        clip
    }

    fn at_30_fps(events: Vec<TabEvent>) -> ClipDocument {
        ClipDocument {
            facts: ClipFacts::of(&clip_of(1)),
            events,
            ..ClipDocument::default()
        }
    }

    fn without_rate(duration: f32, events: Vec<TabEvent>) -> ClipDocument {
        ClipDocument {
            facts: ClipFacts {
                duration,
                ..ClipFacts::default()
            },
            events,
            ..ClipDocument::default()
        }
    }

    fn frame(frame: u32) -> f32 {
        time_of_frame(frame, 30.0)
    }

    #[test]
    fn loading_builds_named_for_animation_events_and_typed_for_others() {
        let clip = clip_with(vec![named_key(0.2, "footstep"), typed_key(0.4)]);

        assert_eq!(
            read_events(&clip).unwrap(),
            vec![named(0.2, "footstep"), typed(0.4)]
        );
    }

    #[test]
    fn an_animation_event_that_does_not_decode_or_has_no_name_stays_typed() {
        let undecodable = ReflectedValue::Leaf(vec![1, 2, 3]);
        let unknown_field =
            ReflectedValue::Struct(vec![("strength".to_owned(), ReflectedValue::Leaf(vec![7]))]);
        let nameless = to_value(&AnimationEvent::default()).unwrap();
        let values = [undecodable, unknown_field, nameless];
        let keys = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                stored_key(index as f32 * 0.1, animation_event_type(), value.clone())
            })
            .collect();

        let events = read_events(&clip_with(keys)).unwrap();

        let expected: Vec<TabEvent> = values
            .into_iter()
            .enumerate()
            .map(|(index, value)| TabEvent {
                time: index as f32 * 0.1,
                kind: TabEventKind::Typed {
                    type_name: animation_event_type().to_owned(),
                    value,
                },
            })
            .collect();
        assert_eq!(events, expected);
    }

    #[test]
    fn tab_events_round_trip_to_event_keys_with_typed_ones_kept_as_stored() {
        let events = vec![named(0.5, "footstep"), typed(0.25)];

        let keys = write_events(&events).unwrap();

        assert_eq!(keys[0].event.type_name(), animation_event_type());
        assert_eq!(keys[1].time, 0.25);
        let ClipEventData::Stored { type_name, value } = &keys[1].event else {
            panic!("a written key holds stored data");
        };
        assert_eq!(type_name, FOOTSTEP);
        assert_eq!(value, &footstep_value());
        let read: Result<Vec<TabEvent>, String> = keys.iter().map(TabEvent::read).collect();
        assert_eq!(read.unwrap(), events);
    }

    #[test]
    fn a_typed_marker_reads_the_last_segment_of_its_type_name() {
        assert_eq!(typed(0.0).label(), "Footstep");
        assert_eq!(named(0.0, "land").label(), "land");
    }

    #[test]
    fn add_event_appends_a_named_event_snapped_to_a_frame_and_selects_it() {
        let mut document = at_30_fps(vec![typed(0.5)]);

        document.add_event("footstep", 0.41).unwrap();

        assert_eq!(
            document.events,
            vec![typed(0.5), named(frame(12), "footstep")]
        );
        assert_eq!(document.selected_event, Some(1));
    }

    #[test]
    fn move_event_snaps_the_time_and_keeps_the_events_order() {
        let mut document = at_30_fps(vec![named(frame(3), "land"), named(frame(6), "footstep")]);

        document.move_event(1, 0.01).unwrap();

        assert_eq!(
            document.events,
            vec![named(frame(3), "land"), named(0.0, "footstep")]
        );
    }

    #[test]
    fn rename_event_renames_in_place() {
        let mut document = at_30_fps(vec![named(frame(3), "land")]);

        document.rename_event(0, "jump").unwrap();

        assert_eq!(document.events, vec![named(frame(3), "jump")]);
    }

    #[test]
    fn remove_event_removes_it_and_clears_the_selection() {
        let mut document = at_30_fps(vec![named(frame(3), "land"), typed(0.5)]);
        document.selected_event = Some(0);

        document.remove_event(0).unwrap();

        assert_eq!(document.events, vec![typed(0.5)]);
        assert_eq!(document.selected_event, None);
    }

    #[test]
    fn edits_on_a_typed_event_are_refused_and_change_nothing() {
        let mut document = at_30_fps(vec![typed(0.5)]);
        document.selected_event = Some(0);

        for result in [
            document.move_event(0, 0.2),
            document.rename_event(0, "land"),
            document.remove_event(0),
        ] {
            assert_eq!(result, Err(TYPED_IS_READ_ONLY.to_owned()));
        }
        assert_eq!(document.events, vec![typed(0.5)]);
        assert_eq!(document.selected_event, Some(0));
    }

    #[test]
    fn an_empty_name_is_refused() {
        let mut document = at_30_fps(vec![named(frame(3), "land")]);

        assert_eq!(document.add_event("", 0.5), Err(NO_NAME.to_owned()));
        assert_eq!(document.rename_event(0, ""), Err(NO_NAME.to_owned()));
        assert_eq!(document.events, vec![named(frame(3), "land")]);
        assert_eq!(document.selected_event, None);
    }

    #[test]
    fn a_time_outside_the_clip_is_refused() {
        let mut document = at_30_fps(vec![named(frame(3), "land")]);

        for time in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            assert!(document.add_event("footstep", time).is_err());
            assert!(document.move_event(0, time).is_err());
        }
        assert_eq!(
            document.add_event("footstep", 1.5),
            Err("1.50 s is outside the clip's 0.00 to 1.00 s".to_owned())
        );
        assert_eq!(document.events, vec![named(frame(3), "land")]);

        let mut document = without_rate(2.0, Vec::new());
        assert!(document.add_event("footstep", 2.01).is_err());
        assert!(document.events.is_empty());
    }

    #[test]
    fn a_same_name_event_on_the_same_frame_is_refused() {
        let mut document = at_30_fps(vec![named(frame(12), "footstep"), named(frame(20), "land")]);
        let refusal = Err("`footstep` is already at frame 12".to_owned());

        assert_eq!(document.add_event("footstep", 0.405), refusal);
        assert_eq!(document.move_event(1, 0.4), Ok(()));
        assert_eq!(document.rename_event(1, "footstep"), refusal);
        assert_eq!(
            document.events,
            vec![named(frame(12), "footstep"), named(frame(12), "land")]
        );

        assert_eq!(document.add_event("footstep", frame(13)), Ok(()));
        assert_eq!(document.move_event(2, frame(12)), refusal);
        assert_eq!(document.events[2], named(frame(13), "footstep"));
    }

    #[test]
    fn a_same_name_event_at_the_same_time_is_refused_without_a_rate() {
        let mut document = without_rate(2.0, vec![named(0.5, "footstep")]);

        assert_eq!(
            document.add_event("footstep", 0.5),
            Err("`footstep` is already at 0.50 s".to_owned())
        );
        assert_eq!(document.add_event("footstep", 0.51), Ok(()));
        assert_eq!(
            document.events,
            vec![named(0.5, "footstep"), named(0.51, "footstep")]
        );
    }

    #[test]
    fn a_snapped_time_on_the_last_frame_of_a_clip_ending_between_frames_is_clamped_and_accepted() {
        let mut document = ClipDocument {
            facts: ClipFacts {
                duration: 1.02,
                rate: Some(30.0),
                ..ClipFacts::default()
            },
            ..ClipDocument::default()
        };

        assert_eq!(document.add_event("end", frame(31)), Ok(()));
        assert_eq!(document.events, vec![named(1.02, "end")]);
        assert!(document.add_event("late", frame(32)).is_err());
    }
}

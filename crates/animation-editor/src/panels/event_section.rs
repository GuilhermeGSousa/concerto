//! The Events section of a clip's inspector, the Delete key, and the fields that edit the selected event.
use concerto_ecs::{
    Entity, Query, Res, ResMut,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_editor::{ActiveEditor, EditorDocument},
    timeline::{frame_at, time_of_frame},
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    focus::FocusedWidget,
    interaction::{UIClick, UIDisabled},
    node::UINode,
    text::UIText,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::{
    KeyCode, MouseButton,
    actions::{ActionFired, ActionMap, Shortcut},
};

use super::{ClipPanels, title};
use crate::{
    ClipDocument, ClipEventContext, RemoveClipEvent,
    events::{TabEvent, TabEventKind},
    settle_edit,
};

const NEW_EVENT: &str = "event";
const FIELD_LABEL_WIDTH: f32 = 40.0;

pub(crate) struct EventsPanel {
    add: Entity,
    name_row: Entity,
    name: Entity,
    time_row: Entity,
    time: Entity,
    type_name: Entity,
    remove: Entity,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EventField {
    Name,
    Time,
}

impl EventsPanel {
    fn field(&self, entity: Entity) -> Option<EventField> {
        if entity == self.name {
            Some(EventField::Name)
        } else if entity == self.time {
            Some(EventField::Time)
        } else {
            None
        }
    }
}

fn add_event_at_head(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some((mut editor, mut state)) = documents.get_entity(document) else {
            return;
        };
        let time = state.time;
        let result = state.add_event(NEW_EVENT, time);
        settle_edit(result, document, &mut editor, &mut cmd);
    })
    .into_listener()
}

fn remove_selected(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some((mut editor, mut state)) = documents.get_entity(document) else {
            return;
        };
        let Some(index) = state.selected_event else {
            return;
        };
        let result = state.remove_event(index);
        settle_edit(result, document, &mut editor, &mut cmd);
    })
    .into_listener()
}

struct FieldRow {
    row: Entity,
    field: Entity,
}

fn field_row(panel: &mut EntityCommandQueue, label: &str, theme: &UITheme) -> FieldRow {
    let mut row = panel.spawn_child(theme.row().gap(theme.spacing_sm).fixed().hidden());
    row.add_child(
        theme
            .label(label)
            .muted()
            .width(UIValue::Px(FIELD_LABEL_WIDTH))
            .fixed(),
    );
    let field = row.spawn_child(theme.text_field("").grow()).entity();
    FieldRow {
        row: row.entity(),
        field,
    }
}

pub(crate) fn build_events(
    panel: &mut EntityCommandQueue,
    document: Entity,
    theme: &UITheme,
) -> EventsPanel {
    panel.add_child(title(theme, "Events"));
    let add = panel
        .spawn_child(
            theme
                .button("Add event")
                .ghost()
                .on_click(add_event_at_head(document)),
        )
        .entity();
    let name = field_row(panel, "Name", theme);
    let time = field_row(panel, "Time", theme);
    let type_name = panel
        .spawn_child(theme.label("").muted().hidden().fixed())
        .entity();
    let remove = panel
        .spawn_child(
            theme
                .button("Remove")
                .ghost()
                .hidden()
                .on_click(remove_selected(document)),
        )
        .entity();
    EventsPanel {
        add,
        name_row: name.row,
        name: name.field,
        time_row: time.row,
        time: time.field,
        type_name,
        remove,
    }
}

fn selected(state: &ClipDocument) -> Option<&TabEvent> {
    state
        .selected_event
        .and_then(|index| state.events.get(index))
}

fn selected_named(state: &ClipDocument) -> Option<&TabEvent> {
    selected(state).filter(|event| !event.is_typed())
}

fn time_text(time: f32, rate: Option<f32>) -> String {
    match rate {
        Some(rate) => frame_at(time, rate).to_string(),
        None => format!("{time:.3}"),
    }
}

fn field_text(state: &ClipDocument, field: EventField) -> Option<String> {
    let event = selected_named(state)?;
    Some(match field {
        EventField::Name => event.label().to_owned(),
        EventField::Time => time_text(event.time, state.facts.rate),
    })
}

fn parsed_time(value: &str, rate: Option<f32>) -> Result<f32, String> {
    let value = value.trim();
    match rate {
        Some(rate) => value
            .parse::<u32>()
            .map(|frame| time_of_frame(frame, rate))
            .map_err(|_| format!("`{value}` is not a whole frame")),
        None => value
            .parse::<f32>()
            .map_err(|_| format!("`{value}` is not a time in seconds")),
    }
}

fn same_time(current: f32, time: f32, rate: Option<f32>) -> bool {
    match rate {
        Some(rate) => frame_at(current, rate) == frame_at(time, rate),
        None => current == time,
    }
}

fn submitted_edit(
    state: &mut ClipDocument,
    field: EventField,
    value: &str,
) -> Option<Result<(), String>> {
    let index = state.selected_event?;
    let current = state.events.get(index)?.clone();
    match field {
        EventField::Name if current.label() == value => None,
        EventField::Name => Some(state.rename_event(index, value)),
        EventField::Time => {
            let rate = state.facts.rate;
            match parsed_time(value, rate) {
                Err(message) => Some(Err(message)),
                Ok(time) if same_time(current.time, time, rate) => None,
                Ok(time) => Some(state.move_event(index, time)),
            }
        }
    }
}

fn write_value(fields: &Query<&mut UITextInput>, entity: Entity, value: &str) {
    if let Some(mut input) = fields.get_entity(entity)
        && input.value != value
    {
        input.value = value.to_owned();
        input.cursor = input.value.len();
        input.selection_anchor = None;
    }
}

pub(crate) fn apply_clip_event_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut cancelled: EventReader<UITextInputCancelled>,
    documents: Query<(Entity, &mut EditorDocument, &mut ClipDocument, &ClipPanels)>,
    fields: Query<&mut UITextInput>,
    mut cmd: CommandQueue,
) {
    for event in submitted.read() {
        for (document, mut editor, mut state, panels) in documents.iter() {
            let Some(field) = panels.inspector.events.field(event.entity) else {
                continue;
            };
            if let Some(result) = submitted_edit(&mut state, field, &event.value) {
                settle_edit(result, document, &mut editor, &mut cmd);
            }
            if let Some(value) = field_text(&state, field) {
                write_value(&fields, event.entity, &value);
            }
        }
    }
    for event in cancelled.read() {
        for (_, _, state, panels) in documents.iter() {
            let Some(field) = panels.inspector.events.field(event.entity) else {
                continue;
            };
            if let Some(value) = field_text(&state, field) {
                write_value(&fields, event.entity, &value);
            }
        }
    }
}

pub(crate) fn sync_clip_events(
    documents: Query<(&ClipDocument, &ClipPanels)>,
    nodes: Query<&mut UINode>,
    texts: Query<&mut UIText>,
    fields: Query<&mut UITextInput>,
    disabled: Query<&UIDisabled>,
    focus: Res<FocusedWidget>,
    mut cmd: CommandQueue,
) {
    let show = |entity: Entity, visible: bool| {
        if let Some(mut node) = nodes.get_entity(entity)
            && node.visible != visible
        {
            node.visible = visible;
        }
    };
    let mut disable = |entity: Entity, off: bool| match (off, disabled.get_entity(entity).is_some())
    {
        (true, false) => cmd.insert(UIDisabled, entity),
        (false, true) => cmd.remove::<UIDisabled>(entity),
        _ => {}
    };
    for (state, panels) in documents.iter() {
        let section = &panels.inspector.events;
        let event = selected(state);
        let type_name = event.and_then(|event| match &event.kind {
            TabEventKind::Typed { type_name, .. } => Some(type_name.as_str()),
            TabEventKind::Named(_) => None,
        });
        let named = event.is_some() && type_name.is_none();

        show(section.name_row, named);
        show(section.time_row, named);
        show(section.type_name, type_name.is_some());
        if let Some(mut text) = texts.get_entity(section.type_name) {
            let value = type_name.unwrap_or("");
            if text.text != value {
                text.text = value.to_owned();
            }
        }
        show(section.remove, event.is_some());
        disable(section.remove, type_name.is_some());
        disable(section.add, state.facts.duration <= 0.0);

        for (entity, field) in [
            (section.name, EventField::Name),
            (section.time, EventField::Time),
        ] {
            if **focus == Some(entity) {
                continue;
            }
            if let Some(value) = field_text(state, field) {
                write_value(&fields, entity, &value);
            }
        }
    }
}

pub(crate) fn remove_clip_event(
    mut fired: EventReader<ActionFired>,
    active: Res<ActiveEditor>,
    documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
    mut cmd: CommandQueue,
) {
    let remove = fired
        .read()
        .fold(false, |remove, action| remove | action.is(RemoveClipEvent));
    let Some(document) = active.0.filter(|_| remove) else {
        return;
    };
    let Some((mut editor, mut state)) = documents.get_entity(document) else {
        return;
    };
    let Some(index) = selected_named(&state).and(state.selected_event) else {
        return;
    };
    let result = state.remove_event(index);
    settle_edit(result, document, &mut editor, &mut cmd);
}

pub(crate) fn sync_clip_event_context(
    active: Res<ActiveEditor>,
    documents: Query<&ClipDocument>,
    focus: Res<FocusedWidget>,
    inputs: Query<&UITextInput>,
    mut actions: ResMut<ActionMap>,
) {
    let named = active
        .0
        .and_then(|document| documents.get_entity(document))
        .and_then(selected_named)
        .is_some();
    let typing = (**focus).is_some_and(|entity| inputs.get_entity(entity).is_some());
    if named && !typing {
        actions.push_context(ClipEventContext);
    } else if actions.is_active(ClipEventContext) {
        actions.pop_context(ClipEventContext);
    }
}

pub(crate) fn bind_clip_keys(mut actions: ResMut<ActionMap>) {
    actions.bind(
        RemoveClipEvent,
        Shortcut::key(KeyCode::Delete),
        ClipEventContext,
    );
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Tab, eventful_clip, single_key_clip, tab_of_clip};
    use super::*;
    use crate::tests::FOOTSTEP;
    use concerto_animation::clip::AnimationClip;
    use concerto_ecs::events::event_channel::EventChannel;
    use concerto_editor::actions::{DeleteEntity, TreeContext, ViewportContext};
    use concerto_name::Name;
    use concerto_window::input::actions::{ActionLabel, InternedActionLabel};

    fn named(time: f32, name: &str) -> TabEvent {
        TabEvent {
            time,
            kind: TabEventKind::Named(Name::new(name)),
        }
    }

    impl Tab {
        fn events(&self) -> &EventsPanel {
            &self.panels().inspector.events
        }

        fn value(&self, field: Entity) -> String {
            self.world()
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value
                .clone()
        }

        fn focused(&self) -> Option<Entity> {
            **self.world().get_resource::<FocusedWidget>().unwrap()
        }

        fn focus(&mut self, field: Option<Entity>) {
            **self.f.world.get_resource_mut::<FocusedWidget>().unwrap() = field;
        }

        fn type_into(&mut self, field: Entity, value: &str) {
            self.focus(Some(field));
            self.f
                .world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
                .value = value.to_owned();
        }

        fn enter(&mut self, field: Entity, value: &str) {
            self.type_into(field, value);
            self.f
                .world
                .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
                .unwrap()
                .push_event(UITextInputSubmitted {
                    entity: field,
                    value: value.to_owned(),
                });
            self.f.frame();
        }

        fn escape(&mut self, field: Entity) {
            self.f
                .world
                .get_resource_mut::<EventChannel<UITextInputCancelled>>()
                .unwrap()
                .push_event(UITextInputCancelled { entity: field });
            self.focus(None);
            self.f.frame();
        }

        fn select(&mut self, index: usize) {
            self.select_marker(Some(index));
            self.f.frame();
        }

        fn delete_means(&self) -> Option<InternedActionLabel> {
            self.world()
                .get_resource::<ActionMap>()
                .unwrap()
                .resolve(Shortcut::key(KeyCode::Delete))
        }

        fn press_delete(&mut self) {
            self.f
                .world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap()
                .push_event(ActionFired {
                    action: RemoveClipEvent.intern(),
                });
            self.f.frame();
        }

        fn focused_is(&self, field: Entity) -> bool {
            self.focused() == Some(field)
        }
    }

    fn event_tab() -> Tab {
        let mut tab = tab_of_clip(&eventful_clip());
        tab.f.frame();
        tab
    }

    #[test]
    fn with_nothing_selected_the_section_shows_only_add_event() {
        let tab = event_tab();
        let section = tab.events();

        assert!(tab.shown(section.add));
        for hidden in [
            section.name_row,
            section.time_row,
            section.type_name,
            section.remove,
        ] {
            assert!(!tab.shown(hidden));
        }
    }

    #[test]
    fn add_event_adds_event_at_the_heads_frame_and_selects_it() {
        let mut tab = event_tab();
        tab.scrub(2.01);
        tab.until("the scrub never came back", |state| {
            !state.playing && frame_at(state.time, 30.0) == 60
        });
        let add = tab.events().add;

        tab.click(add);
        tab.f.frame();

        assert_eq!(tab.state().events.len(), 4);
        assert_eq!(tab.state().events[3], named(2.0, "event"));
        assert_eq!(tab.state().selected_event, Some(3));
        assert!(!tab.clean());
        assert_eq!(tab.status(), "");
        let section = tab.events();
        assert!(tab.shown(section.name_row));
        assert_eq!(tab.value(section.name), "event");
        assert_eq!(tab.value(section.time), "60");
    }

    #[test]
    fn add_event_is_disabled_while_the_clip_has_no_duration() {
        let mut tab = tab_of_clip(&AnimationClip::default());
        tab.f.frame();
        assert!(tab.disabled(tab.events().add));

        let tab = event_tab();
        assert!(!tab.disabled(tab.events().add));
    }

    #[test]
    fn the_name_field_applies_on_enter() {
        let mut tab = event_tab();
        tab.select(1);
        let name = tab.events().name;
        assert_eq!(tab.value(name), "footstep");

        tab.enter(name, "land");

        assert_eq!(tab.state().events[1], named(0.5, "land"));
        assert_eq!(tab.value(name), "land");
        assert_eq!(tab.timeline().markers[1].label, "land");
        assert_eq!(tab.status(), "");
        assert!(!tab.clean());
    }

    #[test]
    fn the_time_field_reads_frames_with_a_rate_and_writes_its_own_format_after_enter() {
        let mut tab = event_tab();
        tab.select(1);
        let time = tab.events().time;
        assert_eq!(tab.value(time), "15");

        tab.enter(time, " 45 ");

        assert_eq!(tab.event_times(), vec![0.25, 1.5, 1.0]);
        assert_eq!(tab.value(time), "45");
        assert!(tab.focused_is(time));
        assert!(!tab.clean());
    }

    #[test]
    fn the_time_field_reads_seconds_with_three_decimals_without_a_rate() {
        let mut tab = tab_of_clip(&single_key_clip());
        tab.paused_at(0.5);
        let add = tab.events().add;
        tab.click(add);
        tab.f.frame();
        let time = tab.events().time;
        assert_eq!(tab.value(time), "0.500");

        tab.enter(time, "1.25");

        assert_eq!(tab.event_times(), vec![1.25]);
        assert_eq!(tab.value(time), "1.250");
    }

    #[test]
    fn a_refused_enter_puts_back_the_events_value_while_the_field_keeps_focus() {
        let mut tab = event_tab();
        tab.select(2);
        let section = tab.events();
        let (name, time) = (section.name, section.time);

        tab.enter(time, "15");

        assert_eq!(tab.status(), "`footstep` is already at frame 15");
        assert_eq!(tab.value(time), "30");
        assert!(tab.focused_is(time));
        assert_eq!(tab.event_times(), vec![0.25, 0.5, 1.0]);

        tab.enter(time, "abc");

        assert_eq!(tab.status(), "`abc` is not a whole frame");
        assert_eq!(tab.value(time), "30");

        tab.enter(name, "");

        assert_eq!(tab.status(), "An event needs a name");
        assert_eq!(tab.value(name), "footstep");
        assert!(tab.focused_is(name));
        assert!(tab.clean());
    }

    #[test]
    fn a_focused_field_is_not_overwritten_and_escape_or_losing_focus_puts_back_its_value() {
        let mut tab = event_tab();
        tab.select(1);
        let name = tab.events().name;

        tab.type_into(name, "typing");
        tab.f.frame();
        assert_eq!(tab.value(name), "typing");

        tab.escape(name);
        assert_eq!(tab.value(name), "footstep");

        tab.type_into(name, "half");
        tab.f.frame();
        tab.focus(None);
        tab.f.frame();

        assert_eq!(tab.value(name), "footstep");
        assert_eq!(tab.state().events[1], named(0.5, "footstep"));
        assert!(tab.clean());
    }

    #[test]
    fn selecting_a_typed_event_shows_its_type_name_and_disables_remove() {
        let mut tab = event_tab();

        tab.select(0);

        let section = tab.events();
        assert!(tab.shown(section.type_name));
        assert_eq!(tab.text(section.type_name).text, FOOTSTEP);
        assert!(!tab.shown(section.name_row));
        assert!(!tab.shown(section.time_row));
        assert!(tab.shown(section.remove));
        assert!(tab.disabled(section.remove));

        tab.select(1);

        let section = tab.events();
        assert!(!tab.shown(section.type_name));
        assert!(tab.shown(section.name_row));
        assert!(!tab.disabled(section.remove));
    }

    #[test]
    fn remove_removes_the_selected_event() {
        let mut tab = event_tab();
        tab.select(1);
        let remove = tab.events().remove;

        tab.click(remove);
        tab.f.frame();

        assert_eq!(tab.event_times(), vec![0.25, 1.0]);
        assert_eq!(tab.state().selected_event, None);
        assert!(!tab.shown(remove));
        assert!(!tab.clean());
    }

    #[test]
    fn the_delete_action_removes_the_selected_named_event_while_its_context_is_active() {
        let mut tab = event_tab();
        assert_eq!(tab.delete_means(), None);
        tab.select(1);
        assert_eq!(tab.delete_means(), Some(RemoveClipEvent.intern()));

        tab.press_delete();

        assert_eq!(tab.event_times(), vec![0.25, 1.0]);
        assert_eq!(tab.state().selected_event, None);
        assert!(!tab.clean());
        assert_eq!(tab.delete_means(), None);
    }

    #[test]
    fn a_typed_event_or_a_focused_text_input_keeps_the_context_off() {
        let mut tab = event_tab();
        tab.select(0);
        assert_eq!(tab.delete_means(), None);

        tab.select(1);
        let name = tab.events().name;
        tab.focus(Some(name));
        tab.f.frame();
        assert_eq!(tab.delete_means(), None);

        tab.focus(None);
        tab.f.frame();
        assert_eq!(tab.delete_means(), Some(RemoveClipEvent.intern()));
    }

    #[test]
    fn the_delete_action_removes_the_selected_event_over_the_viewport_and_the_tree() {
        let mut tab = event_tab();
        tab.select_marker(Some(1));
        let actions = tab.f.world.get_resource_mut::<ActionMap>().unwrap();
        let delete = Shortcut::key(KeyCode::Delete);
        actions.bind(DeleteEntity, delete, ViewportContext);
        actions.bind(DeleteEntity, delete, TreeContext);
        actions.push_context(TreeContext);
        actions.push_context(ViewportContext);
        tab.f.frame();

        assert_eq!(tab.delete_means(), Some(RemoveClipEvent.intern()));

        tab.press_delete();

        assert_eq!(tab.event_times(), vec![0.25, 1.0]);
    }
}

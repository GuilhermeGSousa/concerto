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
    actions::PromptContext,
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
    transport::ClipTransport,
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
           mut documents: Query<(&mut EditorDocument, &mut ClipDocument, &ClipTransport)>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some((mut editor, mut state, transport)) = documents.get_entity_mut(document) else {
            return;
        };
        let time = transport.time;
        let result = state.add_event(NEW_EVENT, time);
        settle_edit(result, document, &mut editor, &mut cmd);
    })
    .into_listener()
}

fn remove_selected(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           mut documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some((mut editor, mut state)) = documents.get_entity_mut(document) else {
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

impl FieldRow {
    fn new(panel: &mut EntityCommandQueue, label: &str, theme: &UITheme) -> Self {
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
}

impl EventsPanel {
    pub(crate) fn build_events(
        panel: &mut EntityCommandQueue,
        document: Entity,
        theme: &UITheme,
    ) -> Self {
        panel.add_child(title(theme, "Events"));
        let add = panel
            .spawn_child(
                theme
                    .button("Add event")
                    .ghost()
                    .on_click(add_event_at_head(document)),
            )
            .entity();
        let name = FieldRow::new(panel, "Name", theme);
        let time = FieldRow::new(panel, "Time", theme);
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

impl ClipDocument {
    fn field_text(&self, field: EventField) -> Option<String> {
        let event = selected_named(self)?;
        Some(match field {
            EventField::Name => event.label().to_owned(),
            EventField::Time => time_text(event.time, self.facts.rate),
        })
    }

    fn submitted_edit(&mut self, field: EventField, value: &str) -> Option<Result<(), String>> {
        let index = self.selected_event?;
        let current = self.events.get(index)?.clone();
        match field {
            EventField::Name if current.label() == value => None,
            EventField::Name => Some(self.rename_event(index, value)),
            EventField::Time => {
                let rate = self.facts.rate;
                match parsed_time(value, rate) {
                    Err(message) => Some(Err(message)),
                    Ok(time) if same_time(current.time, time, rate) => None,
                    Ok(time) => Some(self.move_event(index, time)),
                }
            }
        }
    }
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

fn write_value(fields: &mut Query<&mut UITextInput>, entity: Entity, value: &str) {
    if let Some(mut input) = fields.get_entity_mut(entity)
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
    mut documents: Query<(Entity, &mut EditorDocument, &mut ClipDocument, &ClipPanels)>,
    mut fields: Query<&mut UITextInput>,
    mut cmd: CommandQueue,
) {
    for event in submitted.read() {
        for (document, mut editor, mut state, panels) in documents.iter_mut() {
            let Some(field) = panels.inspector.events.field(event.entity) else {
                continue;
            };
            if let Some(result) = state.submitted_edit(field, &event.value) {
                settle_edit(result, document, &mut editor, &mut cmd);
            }
            if let Some(value) = state.field_text(field) {
                write_value(&mut fields, event.entity, &value);
            }
        }
    }
    for event in cancelled.read() {
        for (_, _, state, panels) in documents.iter_mut() {
            let Some(field) = panels.inspector.events.field(event.entity) else {
                continue;
            };
            if let Some(value) = state.field_text(field) {
                write_value(&mut fields, event.entity, &value);
            }
        }
    }
}

pub(crate) fn sync_clip_events(
    documents: Query<(&ClipDocument, &ClipPanels)>,
    mut nodes: Query<&mut UINode>,
    mut texts: Query<&mut UIText>,
    mut fields: Query<&mut UITextInput>,
    disabled: Query<&UIDisabled>,
    focus: Res<FocusedWidget>,
    mut cmd: CommandQueue,
) {
    let mut show = |entity: Entity, visible: bool| {
        if let Some(mut node) = nodes.get_entity_mut(entity)
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
        if let Some(mut text) = texts.get_entity_mut(section.type_name) {
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
            if let Some(value) = state.field_text(field) {
                write_value(&mut fields, entity, &value);
            }
        }
    }
}

pub(crate) fn remove_clip_event(
    mut fired: EventReader<ActionFired>,
    active: Res<ActiveEditor>,
    mut documents: Query<(&mut EditorDocument, &mut ClipDocument)>,
    mut cmd: CommandQueue,
) {
    let remove = fired
        .read()
        .fold(false, |remove, action| remove | action.is(RemoveClipEvent));
    let Some(document) = active.0.filter(|_| remove) else {
        return;
    };
    let Some((mut editor, mut state)) = documents.get_entity_mut(document) else {
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
    let prompted = actions.is_active(PromptContext);
    if named && !typing && !prompted {
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
    use super::super::tests::{Tab, eventful_clip, tab_of_clip};
    use super::*;
    use concerto_ecs::events::event_channel::EventChannel;
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

        fn focus(&mut self, field: Option<Entity>) {
            **self.f.world.get_resource_mut::<FocusedWidget>().unwrap() = field;
        }

        fn enter(&mut self, field: Entity, value: &str) {
            self.focus(Some(field));
            self.f
                .world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
                .value = value.to_owned();
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

        fn delete_means(&self) -> Option<InternedActionLabel> {
            self.world()
                .get_resource::<ActionMap>()
                .unwrap()
                .resolve(Shortcut::key(KeyCode::Delete))
        }
    }

    #[test]
    fn an_event_added_at_the_head_is_renamed_and_retimed_by_enter_and_removed_by_delete() {
        let mut tab = tab_of_clip(&eventful_clip());
        tab.f.frame();
        tab.scrub(2.01);
        tab.until("the scrub never came back", |transport| {
            !transport.playing && frame_at(transport.time, 30.0) == 60
        });
        let add = tab.events().add;

        tab.click(add);
        tab.f.frame();

        assert_eq!(tab.state().events[3], named(2.0, "event"));
        assert_eq!(tab.state().selected_event, Some(3));
        let section = tab.events();
        let (name, time) = (section.name, section.time);
        assert_eq!(tab.value(time), "60");

        tab.enter(name, "land");
        tab.enter(time, " 45 ");

        assert_eq!(tab.state().events[3], named(1.5, "land"));
        assert_eq!(tab.value(time), "45");
        assert_eq!(tab.status(), "");

        tab.focus(None);
        tab.f.frame();
        assert_eq!(tab.delete_means(), Some(RemoveClipEvent.intern()));
        tab.f
            .world
            .get_resource_mut::<EventChannel<ActionFired>>()
            .unwrap()
            .push_event(ActionFired::new(RemoveClipEvent));
        tab.f.frame();

        assert_eq!(tab.event_times(), vec![0.25, 0.5, 1.0]);
        assert_eq!(tab.state().selected_event, None);
        assert!(!tab.clean());
    }
}

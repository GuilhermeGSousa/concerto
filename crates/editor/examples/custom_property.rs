//! A downstream composite editor. Run with `cargo run -p editor --example custom_property`
//! for a headless commit demonstration. Call `install` after InspectorPlugin in
//! an editor application with the UI plugin to enable its clickable widget.
use std::any::TypeId;

use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, ResMut, World,
    command::CommandQueue,
    signal::{On, listener::IntoListener},
};
use concerto_editable::Editable;
use concerto_editor::inspector::{
    EditError, EditableApp, InspectorRegistry, PropertyCommit, PropertyCommits, PropertyEditor,
    PropertyRow, PropertyRowValue, apply_property_commit,
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    interaction::{Interactable, UIClick},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

// Deliberately not Clone or PartialEq: the editor chooses what to snapshot.
#[derive(Component, Editable)]
pub struct Setting {
    pub title: String,
    pub enabled: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SettingSnapshot {
    pub title: String,
    pub enabled: bool,
}

pub enum SettingEdit {
    Toggle,
    Rename(String),
}

pub struct SettingEditor;

#[derive(Component)]
pub struct SettingButton(pub Entity);

impl PropertyEditor<Setting> for SettingEditor {
    type Snapshot = SettingSnapshot;
    type Edit = SettingEdit;

    fn snapshot(&self, value: &Setting) -> SettingSnapshot {
        SettingSnapshot {
            title: value.title.clone(),
            enabled: value.enabled,
        }
    }

    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &SettingSnapshot,
        theme: &UITheme,
    ) {
        let button = cmd
            .spawn((
                theme
                    .label(label(snapshot))
                    .height(UIValue::Px(theme.control_height))
                    .grow(),
                Interactable,
                SettingButton(row),
                toggle_setting.into_listener(),
            ))
            .entity();
        cmd.add_child(row, button);
    }

    fn apply(&self, value: &mut Setting, edit: &SettingEdit) -> Result<(), EditError> {
        match edit {
            SettingEdit::Toggle => value.enabled = !value.enabled,
            SettingEdit::Rename(title) => {
                if title.trim().is_empty() {
                    return Err(EditError::Rejected);
                }
                value.title = title.clone();
            }
        }
        Ok(())
    }
}

fn label(snapshot: &SettingSnapshot) -> String {
    format!(
        "{}: {} (click to toggle)",
        snapshot.title,
        if snapshot.enabled { "On" } else { "Off" }
    )
}

pub fn toggle_setting(
    on: On<UIClick>,
    buttons: Query<&SettingButton>,
    rows: Query<&PropertyRow>,
    mut commits: ResMut<PropertyCommits>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(button) = buttons.get_entity(on.entity()) else {
        return;
    };
    let Some(row) = rows.get_entity(button.0) else {
        return;
    };
    if let Err(error) = commits.push::<Setting, SettingEditor>(row, SettingEdit::Toggle) {
        log::warn!("Setting edit dropped: {error}");
    }
}

pub fn refresh_settings(
    buttons: Query<(&SettingButton, &mut UIText)>,
    rows: Query<&PropertyRowValue>,
) {
    for (button, mut text) in buttons.iter() {
        let Some(value) = rows.get_entity(button.0) else {
            continue;
        };
        let Some(snapshot) = value.snapshot::<Setting, SettingEditor>() else {
            continue;
        };
        let next = label(snapshot);
        if text.text != next {
            text.text = next;
        }
    }
}

pub fn install(app: &mut App) {
    app.register_editable::<Setting>()
        .register_property_editor::<Setting, SettingEditor>(SettingEditor)
        .add_system(LateUpdate, refresh_settings);
}

#[allow(dead_code)]
fn main() {
    let mut registry = InspectorRegistry::default();
    registry.register_component::<Setting>();
    registry.register_property_editor::<Setting, SettingEditor>(SettingEditor);
    let mut world = World::default();
    let entity = world.spawn(Setting {
        title: "Shadows".into(),
        enabled: false,
    });
    let property = registry
        .collect_component(&world, entity, TypeId::of::<Setting>())
        .unwrap()
        .remove(0);
    let row = property.row(entity, TypeId::of::<Setting>());
    world.insert_resource(registry);
    apply_property_commit(
        &mut world,
        PropertyCommit::new::<Setting, SettingEditor>(&row, SettingEdit::Toggle).unwrap(),
    )
    .unwrap();
    println!(
        "Shadows enabled: {}",
        world
            .get_component_for_entity::<Setting>(entity)
            .unwrap()
            .enabled
    );
}

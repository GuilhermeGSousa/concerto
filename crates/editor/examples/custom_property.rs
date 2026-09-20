//! A downstream composite editor. Run with `cargo run -p editor --example custom_property`
//! for a headless commit demonstration. Call `install` after InspectorPlugin in
//! an editor application with the UI plugin to enable its clickable widget.
use std::any::TypeId;

use app::{schedule_groups::LateUpdate, App};
use ecs::{
    command::CommandQueue, events::event_reader::EventReader, Component, Entity, Query, ResMut,
    World,
};
use editable::Editable;
use editor::inspector::{
    apply_property_commit, EditError, EditableApp, InspectorRegistry, PropertyCommit,
    PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};
use ui::{
    interaction::{Interactable, UIClick},
    node::UINode,
    text::TextComponent,
    theme::UITheme,
    transform::UIValue,
};

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
                UINode {
                    height: UIValue::Px(theme.control_height),
                    flex_grow: 1.0,
                    ..Default::default()
                },
                TextComponent {
                    text: label(snapshot),
                    color: theme.text,
                    font_size: theme.font_size_md,
                    line_height: theme.line_height(theme.font_size_md),
                    ..Default::default()
                },
                Interactable,
                SettingButton(row),
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

pub fn click_settings(
    mut clicks: EventReader<UIClick>,
    buttons: Query<&SettingButton>,
    rows: Query<&PropertyRow>,
    mut commits: ResMut<PropertyCommits>,
) {
    for click in clicks.read() {
        let Some(button) = buttons.get_entity(click.entity) else {
            continue;
        };
        let Some(row) = rows.get_entity(button.0) else {
            continue;
        };
        if let Err(error) = commits.push::<Setting, SettingEditor>(row, SettingEdit::Toggle) {
            log::warn!("Setting edit dropped: {error}");
        }
    }
}

pub fn refresh_settings(
    buttons: Query<(&SettingButton, &mut TextComponent)>,
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
        .add_system(LateUpdate, click_settings)
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

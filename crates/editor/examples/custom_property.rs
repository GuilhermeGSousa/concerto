//! A downstream composite editor. Run with `cargo run -p concerto-editor --example custom_property`
//! for a headless commit demonstration. Call `register` in the recipe the editor builds each
//! scene's `App` from, and `install` on the editor's own `App` after InspectorPlugin.
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::TimePlugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    signal::{On, listener::IntoListener},
};
use concerto_editable::Editable;
use concerto_editor::{
    asset_editor::ActiveEditor,
    dock::PanelRegistry,
    hosted::{HostRequest, HostRole, HostedPlugin, begin, finish_blocking, trigger_on},
    inspector::{
        EditError, EditableApp, InspectorPlugin, PropertyCommits, PropertyEditor, PropertyRow,
        PropertyRowValue, spawn_panel,
    },
    tools::{EditToolsPlugin, selection::Select},
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost},
    interaction::{Interactable, UIClick},
    node::UINode,
    text::UIText,
    text_input::{UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::{input::MouseButton, plugin::WindowPlugin};

// Deliberately not Clone or PartialEq: the editor chooses what to snapshot.
#[derive(Component, Editable, Default)]
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

/// Makes `Setting` editable in a scene's `App`; call it from the recipe.
pub fn register(app: &mut App) {
    app.register_editable::<Setting>();
}

/// Gives the editor's own `App` the widget that edits a `Setting`.
pub fn install(app: &mut App) {
    app.register_property_editor::<Setting, SettingEditor>(SettingEditor)
        .add_system(LateUpdate, refresh_settings);
}

fn headless_recipe(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin);
    register(app);
}

fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let parent = cmd.spawn(UINode::default()).entity();
    spawn_panel(&mut cmd, parent, &theme);
}

/// An editor `App` with the inspector and one scene `App`, `arrange`d before it starts, with no window.
pub fn headless_editor(arrange: impl FnOnce(&mut App)) -> (App, Entity) {
    let mut editor = App::new();
    editor
        .register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .insert_resource(PanelRegistry::default())
        .insert_resource(UITheme::default())
        .insert_resource(FocusedWidget::default())
        .insert_resource(ActiveEditor::default())
        .register_event::<UIFocusGained>()
        .register_event::<UIFocusLost>()
        .register_event::<UITextInputSubmitted>()
        .register_event::<UITextInputCancelled>()
        .register_plugin(HostedPlugin)
        .register_plugin(InspectorPlugin)
        .add_system(Startup, build_inspector);
    install(&mut editor);
    let world = editor.main_mut().world_mut();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    let scene = finish_blocking(begin(
        HostRequest {
            gpu: None,
            recipe: headless_recipe,
            size: (0, 0),
            scale_factor: 1.0,
            role: HostRole::Edit,
        },
        |app| {
            app.register_plugin(EditToolsPlugin);
            arrange(app);
        },
        |_| {},
    ));
    let document = world.spawn(scene);
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
    editor.finish_plugin_build();
    (editor, document)
}

#[allow(dead_code)]
fn main() {
    let mut shadows = None;
    let (mut editor, document) = headless_editor(|scene| {
        shadows = Some(scene.main_mut().world_mut().spawn(Setting {
            title: "Shadows".into(),
            enabled: false,
        }));
    });
    trigger_on(editor.main_mut().world_mut(), document, Select(shadows));
    editor.update();

    let world = editor.main_mut().world_mut();
    let button = world
        .query::<(Entity, &SettingButton), ()>()
        .iter(world)
        .map(|(button, _)| button)
        .next()
        .expect("the inspector built the widget");
    world.trigger_on(
        button,
        UIClick {
            position: Default::default(),
            button: MouseButton::Left,
        },
    );
    editor.update();

    let world = editor.main_mut().world_mut();
    let enabled = world
        .query::<&PropertyRowValue, ()>()
        .iter(world)
        .find_map(|value| value.snapshot::<Setting, SettingEditor>())
        .expect("the inspector shows the setting")
        .enabled;
    println!("Shadows enabled: {enabled}");
}

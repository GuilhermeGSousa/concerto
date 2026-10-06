//! Holds actions that would discard unsaved work until the user decides.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource, command::CommandQueue,
    events::event_reader::EventReader, signal::On,
};
use concerto_ui::{
    elements::prelude::*,
    interaction::{Interactable, UIClick},
    node::UINode,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::{
    input::{
        MouseButton,
        actions::{ActionFired, ActionMap},
    },
    plugin::{CloseRequest, InterceptClose},
    winit_events::WindowEvent,
};
use winit::event::WindowEvent as WinitWindowEvent;

use crate::{
    actions::{DismissPrompt, PromptContext},
    asset_editor::{AssetEditorCommand, AssetEditorCommands, EditorDocument, SaveRequested},
    project::{EditorCommand, EditorCommands},
};

/// An action that destroys a document's live state.
pub enum GuardedIntent {
    Editor(AssetEditorCommand),
    Project(EditorCommand),
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuardChoice {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Asking,
    Saving,
}

struct Held {
    intent: GuardedIntent,
    documents: Vec<Entity>,
    stage: Stage,
}

#[derive(Resource, Default)]
pub struct UnsavedGuard {
    held: Option<Held>,
    choice: Option<GuardChoice>,
    quit_requested: bool,
}

impl UnsavedGuard {
    /// Holds `intent` until the user decides what happens to the dirty `documents`; ignored while a prompt is open.
    pub fn hold(&mut self, intent: GuardedIntent, documents: Vec<Entity>) {
        if !self.asking() {
            self.held = Some(Held {
                intent,
                documents,
                stage: Stage::Asking,
            });
            self.choice = None;
        }
    }

    /// Whether the prompt is waiting for an answer.
    pub fn asking(&self) -> bool {
        self.held
            .as_ref()
            .is_some_and(|held| held.stage == Stage::Asking)
    }

    /// The documents the open prompt is asking about.
    pub fn documents(&self) -> &[Entity] {
        match &self.held {
            Some(held) if held.stage == Stage::Asking => &held.documents,
            _ => &[],
        }
    }

    pub fn choose(&mut self, choice: GuardChoice) {
        if self.asking() {
            self.choice = Some(choice);
        }
    }

    /// Asks to close the window, subject to the guard.
    pub fn request_quit(&mut self) {
        self.quit_requested = true;
    }
}

/// The dirty documents among `documents`.
pub fn dirty_documents(documents: &Query<(Entity, &EditorDocument)>) -> Vec<Entity> {
    documents
        .iter()
        .filter(|(_, document)| document.is_dirty())
        .map(|(entity, _)| entity)
        .collect()
}

pub struct GuardPlugin;

impl Plugin for GuardPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, intercept_close)
            .add_system(Startup, build_prompt)
            .add_system(Update, guard_quit)
            .add_system(Update, resolve_guard)
            .add_system(LateUpdate, dismiss_prompt)
            .add_system(LateUpdate, sync_prompt);
    }
}

fn intercept_close(mut intercept: ResMut<InterceptClose>) {
    intercept.0 = true;
}

fn guard_quit(
    mut events: EventReader<WindowEvent>,
    mut guard: ResMut<UnsavedGuard>,
    documents: Query<(Entity, &EditorDocument)>,
    mut close: ResMut<CloseRequest>,
) {
    let mut quit = events.read().fold(false, |quit, event| {
        quit | matches!(&**event, WinitWindowEvent::CloseRequested)
    });
    if guard.quit_requested {
        guard.quit_requested = false;
        quit = true;
    }
    if !quit {
        return;
    }
    let dirty = dirty_documents(&documents);
    if dirty.is_empty() {
        close.0 = true;
    } else {
        guard.hold(GuardedIntent::Quit, dirty);
    }
}

fn resolve_guard(
    mut guard: ResMut<UnsavedGuard>,
    documents: Query<(&mut EditorDocument, Option<&SaveRequested>)>,
    mut editor: ResMut<AssetEditorCommands>,
    mut project: ResMut<EditorCommands>,
    mut close: ResMut<CloseRequest>,
    mut commands: CommandQueue,
) {
    let Some(stage) = guard.held.as_ref().map(|held| held.stage) else {
        return;
    };
    let run = match (stage, guard.choice) {
        (Stage::Asking, None) => return,
        (Stage::Asking, Some(GuardChoice::Cancel)) => false,
        (Stage::Asking, Some(GuardChoice::Discard)) => {
            for &entity in guard.documents() {
                if let Some((mut document, _)) = documents.get_entity(entity) {
                    let revision = document.revision;
                    document.mark_saved(revision);
                }
            }
            true
        }
        (Stage::Asking, Some(GuardChoice::Save)) => {
            for &entity in guard.documents() {
                if documents
                    .get_entity(entity)
                    .is_some_and(|(document, _)| document.is_dirty())
                {
                    commands.insert(SaveRequested, entity);
                }
            }
            guard.choice = None;
            if let Some(held) = guard.held.as_mut() {
                held.stage = Stage::Saving;
            }
            return;
        }
        (Stage::Saving, _) => {
            let mut dirty = false;
            for &entity in guard.held.iter().flat_map(|held| &held.documents) {
                if let Some((document, requested)) = documents.get_entity(entity) {
                    if requested.is_some() {
                        return;
                    }
                    dirty |= document.is_dirty();
                }
            }
            !dirty
        }
    };
    guard.choice = None;
    let Some(held) = guard.held.take().filter(|_| run) else {
        return;
    };
    match held.intent {
        GuardedIntent::Editor(command) => editor.0.push_back(command),
        GuardedIntent::Project(command) => project.0.push_back(command),
        GuardedIntent::Quit => close.0 = true,
    }
}

#[derive(Component)]
struct Prompt;

#[derive(Component)]
struct PromptMessage;

#[derive(Component)]
struct PromptButton(GuardChoice);

const PROMPT_LAYER: i32 = 900;

fn build_prompt(mut cmd: CommandQueue, theme: Res<UITheme>) {
    cmd.spawn((
        theme
            .canvas()
            .fill(Color::srgba(0.0, 0.0, 0.0, 0.55))
            .size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .column()
            .align_items(taffy::AlignItems::Center)
            .justify(taffy::AlignContent::Center)
            .z_index(PROMPT_LAYER)
            .hidden(),
        Interactable,
        Prompt,
    ))
    .add_child_with(
        (
            theme
                .popup()
                .width(UIValue::Px(380.0))
                .padding(theme.spacing_md)
                .gap(theme.spacing_md),
            Interactable,
        ),
        |dialog| {
            dialog
                .add_child((
                    theme
                        .label("")
                        .single_line()
                        .height(UIValue::Px(theme.control_height)),
                    PromptMessage,
                ))
                .add_child_with(theme.row().justify(taffy::AlignContent::End), |mut row| {
                    for (label, choice) in [
                        ("Cancel", GuardChoice::Cancel),
                        ("Discard", GuardChoice::Discard),
                        ("Save", GuardChoice::Save),
                    ] {
                        row = row.add_child((
                            theme
                                .button(label)
                                .width(UIValue::Px(96.0))
                                .on_click(press_prompt_button),
                            PromptButton(choice),
                        ));
                    }
                });
        },
    );
}

fn press_prompt_button(
    on: On<UIClick>,
    buttons: Query<&PromptButton>,
    mut guard: ResMut<UnsavedGuard>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    if let Some(button) = buttons.get_entity(on.entity()) {
        guard.choose(button.0);
    }
}

fn dismiss_prompt(mut fired: EventReader<ActionFired>, mut guard: ResMut<UnsavedGuard>) {
    let dismiss = fired
        .read()
        .fold(false, |dismiss, action| dismiss | action.is(DismissPrompt));
    if dismiss && guard.asking() {
        guard.choose(GuardChoice::Cancel);
    }
}

fn sync_prompt(
    guard: Res<UnsavedGuard>,
    documents: Query<&EditorDocument>,
    prompts: Query<(&Prompt, &mut UINode)>,
    messages: Query<(&PromptMessage, &mut UIText)>,
    mut actions: ResMut<ActionMap>,
) {
    let asking = guard.asking();
    for (_, mut node) in prompts.iter() {
        if node.visible != asking {
            node.visible = asking;
        }
    }
    if actions.is_active(PromptContext) != asking {
        if asking {
            actions.push_context(PromptContext);
        } else {
            actions.pop_context(PromptContext);
        }
    }
    if !asking {
        return;
    }
    let message = prompt_message(
        guard
            .documents()
            .iter()
            .filter_map(|&entity| documents.get_entity(entity))
            .map(|document| document.title.as_str()),
    );
    for (_, mut text) in messages.iter() {
        if text.text != message {
            text.text = message.clone();
        }
    }
}

fn prompt_message<'a>(titles: impl Iterator<Item = &'a str>) -> String {
    let titles: Vec<&str> = titles.collect();
    match titles.as_slice() {
        [title] => format!("Save changes to {title}?"),
        titles => format!("Save changes to {} assets?", titles.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{IntoSystem, System, World};
    use std::path::PathBuf;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn world() -> World {
        let mut world = World::new();
        world.insert_resource(UnsavedGuard::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(EditorCommands::default());
        world.insert_resource(CloseRequest::default());
        world.insert_resource(concerto_ecs::events::event_channel::EventChannel::<
            WindowEvent,
        >::default());
        world
    }

    fn document(world: &mut World, dirty: bool) -> Entity {
        world.spawn(EditorDocument {
            asset_type: "Scene",
            title: "level".into(),
            current: None,
            pending: None,
            project_generation: 0,
            request_generation: 0,
            order: 0,
            status: String::new(),
            revision: u64::from(dirty),
            saved_revision: 0,
        })
    }

    fn guard(world: &mut World) -> &mut UnsavedGuard {
        world.get_resource_mut::<UnsavedGuard>().unwrap()
    }

    fn closed(world: &World) -> bool {
        world.get_resource::<CloseRequest>().unwrap().0
    }

    fn is_dirty(world: &World, entity: Entity) -> bool {
        world
            .get_component_for_entity::<EditorDocument>(entity)
            .unwrap()
            .is_dirty()
    }

    #[test]
    fn quitting_with_nothing_unsaved_closes_immediately() {
        let mut world = world();
        document(&mut world, false);
        guard(&mut world).request_quit();
        run(&mut world, guard_quit);
        assert!(closed(&world));
        assert!(!guard(&mut world).asking());
    }

    #[test]
    fn quitting_with_unsaved_work_asks_first_and_cancel_keeps_the_window() {
        let mut world = world();
        let doc = document(&mut world, true);
        guard(&mut world).request_quit();
        run(&mut world, guard_quit);
        assert!(!closed(&world));
        assert_eq!(guard(&mut world).documents(), [doc]);

        guard(&mut world).choose(GuardChoice::Cancel);
        run(&mut world, resolve_guard);
        assert!(!closed(&world));
        assert!(!guard(&mut world).asking());
        assert!(is_dirty(&world, doc));
    }

    #[test]
    fn discard_runs_the_held_intent_and_clears_the_documents() {
        let mut world = world();
        let doc = document(&mut world, true);
        guard(&mut world).hold(
            GuardedIntent::Editor(AssetEditorCommand::Close(doc)),
            vec![doc],
        );
        guard(&mut world).choose(GuardChoice::Discard);
        run(&mut world, resolve_guard);
        assert!(!is_dirty(&world, doc));
        assert!(matches!(
            world.get_resource::<AssetEditorCommands>().unwrap().0.front(),
            Some(AssetEditorCommand::Close(entity)) if *entity == doc
        ));
        assert!(!guard(&mut world).asking());
    }

    #[test]
    fn save_runs_the_intent_only_once_the_documents_are_clean() {
        let mut world = world();
        let doc = document(&mut world, true);
        guard(&mut world).hold(
            GuardedIntent::Project(EditorCommand::OpenProject(PathBuf::from("next"))),
            vec![doc],
        );
        guard(&mut world).choose(GuardChoice::Save);
        run(&mut world, resolve_guard);
        assert!(
            world
                .get_component_for_entity::<SaveRequested>(doc)
                .is_some()
        );
        run(&mut world, resolve_guard);
        assert!(world.get_resource::<EditorCommands>().unwrap().0.is_empty());

        world.remove_component::<SaveRequested>(doc);
        world
            .get_component_for_entity_mut::<EditorDocument>(doc)
            .unwrap()
            .mark_saved(1);
        run(&mut world, resolve_guard);
        assert!(matches!(
            world.get_resource::<EditorCommands>().unwrap().0.front(),
            Some(EditorCommand::OpenProject(path)) if path == &PathBuf::from("next")
        ));
    }

    #[test]
    fn a_failed_save_drops_the_intent() {
        let mut world = world();
        let doc = document(&mut world, true);
        guard(&mut world).hold(GuardedIntent::Quit, vec![doc]);
        guard(&mut world).choose(GuardChoice::Save);
        run(&mut world, resolve_guard);
        world.remove_component::<SaveRequested>(doc);
        run(&mut world, resolve_guard);
        assert!(!closed(&world));
        assert!(is_dirty(&world, doc));
        guard(&mut world).request_quit();
        run(&mut world, guard_quit);
        assert!(guard(&mut world).asking(), "the guard is free to ask again");
    }

    #[test]
    fn a_second_guarded_action_is_ignored_while_the_prompt_is_open() {
        let mut world = world();
        let doc = document(&mut world, true);
        guard(&mut world).hold(GuardedIntent::Quit, vec![doc]);
        guard(&mut world).hold(
            GuardedIntent::Editor(AssetEditorCommand::Close(doc)),
            vec![doc],
        );
        guard(&mut world).choose(GuardChoice::Discard);
        run(&mut world, resolve_guard);
        assert!(closed(&world));
        assert!(
            world
                .get_resource::<AssetEditorCommands>()
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[test]
    fn the_prompt_names_one_asset_and_counts_several() {
        assert_eq!(
            prompt_message(["level"].into_iter()),
            "Save changes to level?"
        );
        assert_eq!(
            prompt_message(["a", "b"].into_iter()),
            "Save changes to 2 assets?"
        );
    }
}

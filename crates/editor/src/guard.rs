//! Holds actions that would discard unsaved work until the user decides.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource,
    command::CommandQueue,
    events::event_reader::EventReader,
    query::filter::With,
    signal::{
        On, Signal,
        listener::{IntoListener, Listener},
    },
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
    events::WindowEvent,
    input::{
        MouseButton,
        actions::{ActionFired, ActionMap},
    },
    plugin::{Window, WindowCommand},
};

use crate::{
    actions::{DismissPrompt, PromptContext},
    asset_editor::{
        AssetEditorCommand, AssetEditorCommands, EditorDocument, SaveRequested, Unsaved,
    },
    hosted::{PlayState, StopPlay},
    project::{EditorCommand, EditorCommands},
};

/// An action that destroys a document's live state.
pub enum GuardedIntent {
    Editor(AssetEditorCommand),
    Project(EditorCommand),
    Quit,
}

/// The user discarded a document's unsaved changes; its editor forgets that they were made.
pub struct DiscardEdits(pub Entity);
impl Signal for DiscardEdits {}

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

/// The documents with unsaved changes.
pub fn dirty_documents(
    documents: &Query<Entity, (With<EditorDocument>, With<Unsaved>)>,
) -> Vec<Entity> {
    documents.iter().collect()
}

pub(crate) fn stop_every_game(states: &Query<(Entity, &PlayState)>, cmd: &mut CommandQueue) {
    for (document, state) in states.iter() {
        if state.is_on() {
            cmd.trigger(StopPlay(document));
        }
    }
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

fn intercept_close(mut window: ResMut<Window>) {
    window.settings.intercept_close = true;
}

fn guard_quit(
    mut events: EventReader<WindowEvent>,
    mut guard: ResMut<UnsavedGuard>,
    documents: Query<Entity, (With<EditorDocument>, With<Unsaved>)>,
    states: Query<(Entity, &PlayState)>,
    mut window: ResMut<Window>,
    mut cmd: CommandQueue,
) {
    let mut quit = events.read().fold(false, |quit, event| {
        quit | matches!(event, WindowEvent::CloseRequested)
    });
    if guard.quit_requested {
        guard.quit_requested = false;
        quit = true;
    }
    if !quit {
        return;
    }
    stop_every_game(&states, &mut cmd);
    let dirty = dirty_documents(&documents);
    if dirty.is_empty() {
        window.send(WindowCommand::Close);
    } else {
        guard.hold(GuardedIntent::Quit, dirty);
    }
}

fn resolve_guard(
    mut guard: ResMut<UnsavedGuard>,
    documents: Query<(&EditorDocument, Option<&Unsaved>, Option<&SaveRequested>)>,
    mut editor: ResMut<AssetEditorCommands>,
    mut project: ResMut<EditorCommands>,
    mut window: ResMut<Window>,
    states: Query<(Entity, &PlayState)>,
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
                if documents
                    .get_entity(entity)
                    .is_some_and(|(_, unsaved, _)| unsaved.is_some())
                {
                    commands.remove::<Unsaved>(entity);
                    commands.trigger(DiscardEdits(entity));
                }
            }
            true
        }
        (Stage::Asking, Some(GuardChoice::Save)) => {
            for &entity in guard.documents() {
                if documents
                    .get_entity(entity)
                    .is_some_and(|(_, unsaved, _)| unsaved.is_some())
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
                if let Some((_, unsaved, requested)) = documents.get_entity(entity) {
                    if requested.is_some() {
                        return;
                    }
                    dirty |= unsaved.is_some();
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
        GuardedIntent::Project(command) => {
            stop_every_game(&states, &mut commands);
            project.0.push_back(command);
        }
        GuardedIntent::Quit => {
            stop_every_game(&states, &mut commands);
            window.send(WindowCommand::Close);
        }
    }
}

#[derive(Component)]
struct Prompt;

#[derive(Component)]
struct PromptMessage;

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
        |mut dialog| {
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
                        row.add_child(
                            theme
                                .button(label)
                                .width(UIValue::Px(96.0))
                                .on_click(press_prompt_button(choice)),
                        );
                    }
                });
        },
    );
}

fn press_prompt_button(choice: GuardChoice) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut guard: ResMut<UnsavedGuard>| {
        if on.signal().button == MouseButton::Left {
            guard.choose(choice);
        }
    })
    .into_listener()
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
        world.insert_resource(Window::new((1280, 720), 1.0));
        world.insert_resource(concerto_ecs::events::event_channel::EventChannel::<
            WindowEvent,
        >::default());
        world
    }

    fn document(world: &mut World, dirty: bool) -> Entity {
        let document = world.spawn(EditorDocument {
            asset_type: "Scene",
            title: "level".into(),
            current: None,
            pending: None,
            order: 0,
            status: String::new(),
            failed: None,
        });
        if dirty {
            world.insert(Unsaved, document);
        }
        document
    }

    fn guard(world: &mut World) -> &mut UnsavedGuard {
        world.get_resource_mut::<UnsavedGuard>().unwrap()
    }

    fn closed(world: &mut World) -> bool {
        world
            .get_resource_mut::<Window>()
            .unwrap()
            .drain_commands()
            .contains(&WindowCommand::Close)
    }

    fn is_dirty(world: &World, entity: Entity) -> bool {
        world.get_component_for_entity::<Unsaved>(entity).is_some()
    }

    #[test]
    fn quitting_with_nothing_unsaved_closes_immediately() {
        let mut world = world();
        document(&mut world, false);
        guard(&mut world).request_quit();
        run(&mut world, guard_quit);
        assert!(closed(&mut world));
        assert!(!guard(&mut world).asking());
    }

    fn push_close_request(world: &mut World) {
        world
            .get_resource_mut::<concerto_ecs::events::event_channel::EventChannel<WindowEvent>>()
            .unwrap()
            .push_event(WindowEvent::CloseRequested);
    }

    #[test]
    fn a_close_request_with_nothing_unsaved_queues_close() {
        let mut world = world();
        document(&mut world, false);
        push_close_request(&mut world);
        run(&mut world, guard_quit);
        assert!(closed(&mut world));
        assert!(!guard(&mut world).asking());
    }

    #[test]
    fn a_close_request_with_unsaved_work_holds_the_quit() {
        let mut world = world();
        let doc = document(&mut world, true);
        push_close_request(&mut world);
        run(&mut world, guard_quit);
        assert!(!closed(&mut world));
        assert_eq!(guard(&mut world).documents(), [doc]);
    }

    #[test]
    fn quitting_with_unsaved_work_asks_first_and_cancel_keeps_the_window() {
        let mut world = world();
        let doc = document(&mut world, true);
        guard(&mut world).request_quit();
        run(&mut world, guard_quit);
        assert!(!closed(&mut world));
        assert_eq!(guard(&mut world).documents(), [doc]);

        guard(&mut world).choose(GuardChoice::Cancel);
        run(&mut world, resolve_guard);
        assert!(!closed(&mut world));
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
    fn a_discarded_document_is_clean_when_its_held_replacement_loads() {
        use crate::asset_editor::{
            ActiveEditor, AssetEditorRegistry, finish_asset_request, process_editor_commands,
        };
        use concerto_foundation::assets::{Asset, AssetId};
        use concerto_scene::scene::Scene;

        let mut world = world();
        world.insert_resource(ActiveEditor::default());
        let mut project = crate::project::ProjectState::default();
        project.project = Some(crate::project::Project {
            root: std::path::PathBuf::from("project"),
            content_root: "content".into(),
        });
        world.insert_resource(project);
        let mut registry = AssetEditorRegistry::default();
        registry
            .register::<Scene, crate::scene::SceneEditor>()
            .unwrap();
        world.insert_resource(registry);
        let doc = document(&mut world, true);
        let next = concerto_foundation::assets::registry::AssetEntry {
            id: AssetId::new(),
            address: "next.gasset".into(),
            kind: Scene::name().into(),
            provenance: None,
        };
        let mut assets = concerto_foundation::assets::registry::AssetRegistry::default();
        assets.insert(next.clone());
        world.insert_resource(assets);
        let open = |world: &mut World| {
            world
                .get_resource_mut::<AssetEditorCommands>()
                .unwrap()
                .0
                .push_back(AssetEditorCommand::Open(next.clone()));
        };

        open(&mut world);
        run(&mut world, process_editor_commands);
        assert_eq!(guard(&mut world).documents(), [doc]);
        let pending = |world: &World| {
            world
                .get_component_for_entity::<EditorDocument>(doc)
                .unwrap()
                .pending
                .clone()
        };
        assert!(pending(&world).is_none());

        guard(&mut world).choose(GuardChoice::Discard);
        run(&mut world, resolve_guard);
        run(&mut world, process_editor_commands);
        assert!(!guard(&mut world).asking());
        assert_eq!(pending(&world), Some(next.clone()));

        let opened = world
            .get_component_for_entity_mut::<EditorDocument>(doc)
            .unwrap();
        assert!(finish_asset_request(opened, &next, Ok(())));
        assert_eq!(opened.current, Some(next));
        assert!(!is_dirty(&world, doc));
    }

    #[test]
    fn discarding_an_edited_scene_lets_its_held_replacement_load() {
        use crate::hosted::testing::SceneFixture;
        let mut f = SceneFixture::new();
        f.world.insert_resource(EditorCommands::default());
        let document = f.open();
        f.rename(document, "first", "renamed");
        assert!(f.unsaved(document));
        f.request_other_scene(document);
        assert_eq!(guard(&mut f.world).documents(), [document]);

        guard(&mut f.world).choose(GuardChoice::Discard);
        run(&mut f.world, resolve_guard);
        f.frames_until_idle();

        assert!(f.open_failures().is_empty(), "{}", f.status(document));
        assert_eq!(f.document(document).current, Some(f.other.clone()));
        assert!(!f.unsaved(document));
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
        world.remove_component::<Unsaved>(doc);
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
        assert!(!closed(&mut world));
        assert!(is_dirty(&world, doc));
        guard(&mut world).request_quit();
        run(&mut world, guard_quit);
        assert!(guard(&mut world).asking(), "the guard is free to ask again");
    }

    fn prompt_button(world: &mut World, label: &str) -> Entity {
        world
            .query::<(Entity, &UIText), ()>()
            .iter(world)
            .find(|(_, text)| text.text == label)
            .map(|(entity, _)| entity)
            .expect("the prompt has the button")
    }

    #[test]
    fn each_prompt_button_chooses_its_own_answer() {
        for (label, choice) in [
            ("Cancel", GuardChoice::Cancel),
            ("Discard", GuardChoice::Discard),
            ("Save", GuardChoice::Save),
        ] {
            let mut world = world();
            world.insert_resource(UITheme::default());
            world.register_component::<concerto_ecs::entity::hierarchy::ChildOf>();
            world.register_component::<concerto_ecs::entity::hierarchy::Children>();
            run(&mut world, build_prompt);
            document(&mut world, true);
            guard(&mut world).request_quit();
            run(&mut world, guard_quit);

            let button = prompt_button(&mut world, label);
            world.trigger_on(
                button,
                UIClick {
                    position: Default::default(),
                    button: MouseButton::Left,
                },
            );

            assert_eq!(guard(&mut world).choice, Some(choice), "{label}");
        }
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
        assert!(closed(&mut world));
        assert!(
            world
                .get_resource::<AssetEditorCommands>()
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[test]
    fn quitting_while_playing_stops_the_game_and_closes_without_asking() {
        use crate::hosted::{PlayState, testing::SceneFixture};
        let mut f = SceneFixture::new();
        let document = f.open();
        f.world.trigger(crate::scene::Play(document));
        f.frames_until_playing(document);

        guard(&mut f.world).request_quit();
        run(&mut f.world, guard_quit);

        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.hosted_is_editing(document));
        assert!(closed(&mut f.world));
        assert!(!guard(&mut f.world).asking());
    }

    #[test]
    fn a_held_quit_or_project_change_stops_a_game_started_while_it_was_held() {
        use crate::hosted::{PlayState, testing::SceneFixture};
        for intent in [
            GuardedIntent::Quit,
            GuardedIntent::Project(EditorCommand::OpenProject(PathBuf::from("next"))),
        ] {
            let mut f = SceneFixture::new();
            f.world.insert_resource(EditorCommands::default());
            let document = f.open();
            f.world.trigger(crate::scene::Play(document));
            f.frames_until_playing(document);
            guard(&mut f.world).hold(intent, vec![document]);
            guard(&mut f.world).choose(GuardChoice::Discard);

            run(&mut f.world, resolve_guard);

            assert_eq!(f.play_state(document), Some(PlayState::Editing));
            assert!(f.hosted_is_editing(document));
        }
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

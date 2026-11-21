//! Lifecycle and registration for asset editors.

use anyhow::{Result, bail};
use concerto_app::{App, InspectedAsset};
use concerto_ecs::events::{event_reader::EventReader, event_writer::EventWriter};
use concerto_ecs::{
    Component, Entity, Query, Resource,
    command::{CommandQueue, EntityCommandQueue},
    resource::{Res, ResMut},
};
use concerto_foundation::assets::content::save_content_asset_into;
use concerto_foundation::assets::{Asset, AssetId, CreatableAsset};
use concerto_window::input::actions::ActionFired;
use derive_more::Deref;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};

use crate::inspected_asset::InspectedAssetEditor;
use crate::project::AssetsWritten;
use crate::scene::SceneSaved;

/// Installs an editor's components when its document is created.
pub trait AssetEditor: Send + Sync + 'static {
    fn build(&self, _editor: &mut EntityCommandQueue) {}
    /// Whether this editor's tabs show a close button.
    fn closable(&self) -> bool {
        true
    }
    /// The preview `App` this editor's documents host, if any.
    fn preview(&self) -> Option<PreviewSpec> {
        None
    }
    /// Whether this editor's documents get a host in the bottom strip, beside the Asset Browser.
    fn bottom_host(&self) -> bool {
        false
    }
    /// Whether this editor's tabs show their inspected values as the tab's main content.
    fn form(&self) -> bool {
        false
    }
}

/// What an asset editor's preview `App` adds to the game's recipe.
#[derive(Clone, Copy)]
pub struct PreviewSpec {
    pub build: fn(&mut App),
}

/// Writes a new asset of one creatable type.
pub struct Creator(Box<dyn Fn(&Path, &str, &mut AssetRegistry) -> Result<AssetId> + Send + Sync>);

impl Creator {
    /// Writes a new asset at `project_root/address`, records it in `registry`, and returns its id.
    pub fn create(
        &self,
        project_root: &Path,
        address: &str,
        registry: &mut AssetRegistry,
    ) -> Result<AssetId> {
        (self.0)(project_root, address, registry)
    }
}

/// Registered asset editor implementations, keyed by asset type.
#[derive(Resource, Default)]
pub struct AssetEditorRegistry {
    editors: HashMap<&'static str, Box<dyn AssetEditor>>,
    creators: BTreeMap<&'static str, Creator>,
}

pub(crate) const MAX_NEW_NUMBER: usize = 999;

pub(crate) fn new_asset_name(kind: &str, number: usize) -> String {
    match number {
        0 => format!("New {kind}"),
        number => format!("New {kind} {number}"),
    }
}

/// Checks `name` against the rules for an asset name typed in the editor.
pub fn valid_asset_name(name: &str) -> Result<(), String> {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if name.is_empty() {
        return Err("A name cannot be empty".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
    {
        return Err("A name holds only letters, digits, spaces, - and _".into());
    }
    if name.len() > 64 {
        return Err("A name holds at most 64 characters".into());
    }
    if name.starts_with(' ') || name.ends_with(' ') {
        return Err("A name cannot start or end with a space".into());
    }
    if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(name))
    {
        return Err(format!("{name} is a reserved name on Windows"));
    }
    Ok(())
}

impl AssetEditorRegistry {
    pub fn register<T: Asset, E: AssetEditor + Default>(&mut self) -> Result<()> {
        if self.editors.contains_key(T::name()) {
            bail!("an editor is already registered for {}", T::name());
        }
        self.editors.insert(T::name(), Box::new(E::default()));
        Ok(())
    }

    /// Registers `E` for `T`, and lets the editor create a new `T` with `T::new_asset`.
    pub fn register_creatable<T: CreatableAsset, E: AssetEditor + Default>(
        &mut self,
    ) -> Result<()> {
        let longest = new_asset_name(T::name(), MAX_NEW_NUMBER);
        if let Err(error) = valid_asset_name(&longest) {
            bail!("{longest} is not a valid asset name: {error}");
        }
        self.register::<T, E>()?;
        self.creators.insert(
            T::name(),
            Creator(Box::new(|root, address, registry| {
                save_content_asset_into(&T::new_asset(), root, address, registry)
            })),
        );
        Ok(())
    }

    /// Gives `record`'s kind the generic inspector editor and makes it creatable, unless the kind has an editor already.
    pub fn register_inspected(&mut self, record: InspectedAsset) {
        if self.editors.contains_key(record.kind) {
            return;
        }
        let longest = new_asset_name(record.kind, MAX_NEW_NUMBER);
        if let Err(error) = valid_asset_name(&longest) {
            log::warn!(
                "{} is not offered for editing: {longest} is not a valid asset name: {error}",
                record.kind
            );
            return;
        }
        self.editors
            .insert(record.kind, Box::new(InspectedAssetEditor { record }));
        self.creators.insert(
            record.kind,
            Creator(Box::new(move |root, address, registry| {
                (record.create)(root, address, registry)
            })),
        );
    }

    /// The creatable asset kinds and their creators, ordered by kind.
    pub fn creatable(&self) -> impl Iterator<Item = (&'static str, &Creator)> {
        self.creators.iter().map(|(kind, creator)| (*kind, creator))
    }

    /// Returns the registered asset kind matching `kind`, if it has an editor.
    pub fn editor_kind(&self, kind: &str) -> Option<&'static str> {
        self.editors.get_key_value(kind).map(|(kind, _)| *kind)
    }

    /// Whether a tab of `kind` shows a close button; a kind with no editor does.
    pub fn closable(&self, kind: &str) -> bool {
        self.editors
            .get(kind)
            .is_none_or(|editor| editor.closable())
    }

    /// The preview `App` a document of `kind` hosts; a kind with no editor has none.
    pub fn preview(&self, kind: &str) -> Option<PreviewSpec> {
        self.editors.get(kind).and_then(|editor| editor.preview())
    }

    /// Whether a document of `kind` gets a `bottom` host; a kind with no editor does not.
    pub fn bottom_host(&self, kind: &str) -> bool {
        self.editors
            .get(kind)
            .is_some_and(|editor| editor.bottom_host())
    }

    /// Whether a document of `kind` shows its inspected values in place of the centre: its editor says `form` and hosts no preview.
    pub fn form(&self, kind: &str) -> bool {
        self.editors
            .get(kind)
            .is_some_and(|editor| editor.form() && editor.preview().is_none())
    }
}

/// Application convenience API for asset editor registration.
pub trait AssetEditorAppExt {
    fn register_asset_editor<T: Asset, E: AssetEditor + Default>(&mut self) -> Result<&mut Self>;
    /// Registers `E` for `T`, and lets the content browser create a new `T` with `T::new_asset`.
    fn register_creatable_asset_editor<T: CreatableAsset, E: AssetEditor + Default>(
        &mut self,
    ) -> Result<&mut Self>;
}
impl AssetEditorAppExt for App {
    fn register_asset_editor<T: Asset, E: AssetEditor + Default>(&mut self) -> Result<&mut Self> {
        self.get_resource_mut::<AssetEditorRegistry>()
            .ok_or_else(|| anyhow::anyhow!("AssetEditorRegistry is not installed"))?
            .register::<T, E>()?;
        Ok(self)
    }

    fn register_creatable_asset_editor<T: CreatableAsset, E: AssetEditor + Default>(
        &mut self,
    ) -> Result<&mut Self> {
        self.get_resource_mut::<AssetEditorRegistry>()
            .ok_or_else(|| anyhow::anyhow!("AssetEditorRegistry is not installed"))?
            .register_creatable::<T, E>()?;
        Ok(self)
    }
}

#[derive(Component, Clone)]
pub struct EditorDocument {
    pub asset_type: &'static str,
    /// Tab title retained even if the initial load fails.
    pub title: String,
    pub current: Option<AssetEntry>,
    pub pending: Option<AssetEntry>,
    pub order: u64,
    pub status: String,
    /// The last request that failed to load, until a request replaces it.
    pub failed: Option<AssetEntry>,
}

/// On a document whose content differs from what is on disk.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Unsaved;

/// On a document while one of its workers runs, so MCP `status` reads busy and frames keep running.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct BackgroundWork;

/// Asks the editor owning this document to write it to disk; that editor removes the marker.
#[derive(Component)]
pub struct SaveRequested;

pub(crate) fn request_save(
    mut fired: EventReader<ActionFired>,
    active: Res<ActiveEditor>,
    documents: Query<&EditorDocument>,
    unsaved: Query<&Unsaved>,
    mut commands: CommandQueue,
) {
    let save = fired
        .read()
        .fold(false, |save, action| save | action.is(crate::actions::Save));
    let Some(entity) = active.0.filter(|_| save) else {
        return;
    };
    if unsaved.get_entity(entity).is_some()
        && documents
            .get_entity(entity)
            .is_some_and(|document| document.pending.is_none())
    {
        commands.insert(SaveRequested, entity);
    }
}

pub(crate) fn report_saves_as_written(
    mut saved: EventReader<SceneSaved>,
    mut written: EventWriter<AssetsWritten>,
) {
    for saved in saved.read() {
        if !saved.written.is_empty() {
            written.write(AssetsWritten {
                ids: saved.written.iter().map(|asset| asset.id).collect(),
            });
        }
    }
}

#[derive(Resource, Default, Deref)]
pub struct ActiveEditor(pub Option<Entity>);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorOwned(pub Entity);

#[derive(Component, Default, Clone, Copy)]
pub struct EditorHosts {
    pub canvas: Option<Entity>,
    pub left: Option<Entity>,
    pub inspector: Option<Entity>,
    pub bottom: Option<Entity>,
}

pub enum AssetEditorCommand {
    /// Opens an asset of the current project; one that is not in it is ignored.
    Open(AssetEntry),
    Activate(Entity),
    Close(Entity),
    CloseAll,
}
#[derive(Resource, Default)]
pub struct AssetEditorCommands(pub VecDeque<AssetEditorCommand>);

/// Applies the result of work started for `asset`, if the document still asks for it.
pub fn finish_asset_request(
    doc: &mut EditorDocument,
    asset: &AssetEntry,
    result: Result<()>,
) -> bool {
    if !asset_request_is_current(doc, asset) {
        return false;
    }
    match result {
        Ok(()) => {
            doc.current = doc.pending.take();
            doc.failed = None;
            if let Some(asset) = &doc.current {
                doc.title = asset.name().into_owned();
            }
            doc.status.clear();
        }
        Err(error) => {
            doc.status = format!("{error:#}");
            doc.failed = doc.pending.take();
        }
    }
    true
}

/// Whether work started for `asset` still matches the document's request.
pub fn asset_request_is_current(doc: &EditorDocument, asset: &AssetEntry) -> bool {
    doc.pending.as_ref() == Some(asset)
}

/// Process tab commands using declared ECS access.
pub fn process_editor_commands(
    mut requests: ResMut<AssetEditorCommands>,
    registry: Res<AssetEditorRegistry>,
    assets: Res<AssetRegistry>,
    mut active: ResMut<ActiveEditor>,
    mut guard: ResMut<crate::guard::UnsavedGuard>,
    documents: Query<(Entity, &EditorDocument)>,
    unsaved: Query<&Unsaved>,
    owned: Query<(Entity, &EditorOwned)>,
    states: Query<&crate::hosted::PlayState>,
    mut commands: CommandQueue,
) {
    if requests.0.is_empty() {
        return;
    }
    let stop_game = |entity: Entity, commands: &mut CommandQueue| {
        if states.get_entity(entity).is_some_and(|state| state.is_on()) {
            commands.trigger(crate::hosted::StopPlay(entity));
        }
    };
    let mut next_active = active.0;
    let mut staged: Vec<_> = documents.iter().map(|(e, d)| (e, d.clone())).collect();
    let mut created = HashSet::new();
    let mut changed = HashSet::new();
    let mut closed = HashSet::new();
    for request in requests.0.drain(..) {
        match request {
            AssetEditorCommand::Open(asset) => {
                if assets.get(asset.id).is_none() {
                    continue;
                }
                let Some(asset_type) = registry.editor_kind(&asset.kind) else {
                    continue;
                };
                if let Some((entity, doc)) =
                    staged.iter_mut().find(|(_, d)| d.asset_type == asset_type)
                {
                    let is_unsaved =
                        !created.contains(entity) && unsaved.get_entity(*entity).is_some();
                    if is_unsaved && doc.current.as_ref().is_none_or(|a| a.id != asset.id) {
                        stop_game(*entity, &mut commands);
                        guard.hold(
                            crate::guard::GuardedIntent::Editor(AssetEditorCommand::Open(asset)),
                            vec![*entity],
                        );
                        continue;
                    }
                    if doc.pending.as_ref().is_none_or(|a| a.id != asset.id) {
                        if doc.current.as_ref().is_some_and(|a| a.id == asset.id) {
                            doc.pending = None;
                            doc.status.clear();
                        } else {
                            doc.status = format!("Opening {}…", asset.address);
                            doc.failed = None;
                            doc.pending = Some(asset);
                        }
                        changed.insert(*entity);
                    }
                    next_active = Some(*entity);
                } else {
                    let order = staged.iter().map(|(_, d)| d.order).max().unwrap_or(0) + 1;
                    let entity = commands.spawn(()).entity();
                    staged.push((
                        entity,
                        EditorDocument {
                            asset_type,
                            title: asset.name().into_owned(),
                            status: format!("Opening {}…", asset.address),
                            failed: None,
                            current: None,
                            pending: Some(asset),
                            order,
                        },
                    ));
                    created.insert(entity);
                    changed.insert(entity);
                    next_active = Some(entity);
                }
            }
            AssetEditorCommand::Activate(entity) => {
                if staged.iter().any(|(e, _)| *e == entity) {
                    next_active = Some(entity);
                }
            }
            AssetEditorCommand::Close(entity) => {
                if let Some(index) = staged.iter().position(|(e, _)| *e == entity) {
                    if !created.contains(&entity) && unsaved.get_entity(entity).is_some() {
                        stop_game(entity, &mut commands);
                        guard.hold(
                            crate::guard::GuardedIntent::Editor(AssetEditorCommand::Close(entity)),
                            vec![entity],
                        );
                        continue;
                    }
                    let (_, doc) = staged.remove(index);
                    closed.insert(entity);
                    if next_active == Some(entity) {
                        next_active = staged
                            .iter()
                            .filter(|(_, d)| d.order < doc.order)
                            .max_by_key(|(_, d)| d.order)
                            .or_else(|| staged.iter().min_by_key(|(_, d)| d.order))
                            .map(|(e, _)| *e);
                    }
                }
            }
            AssetEditorCommand::CloseAll => {
                closed.extend(staged.drain(..).map(|(e, _)| e));
                next_active = None;
            }
        }
    }
    if active.0 != next_active {
        active.0 = next_active;
        commands.trigger(crate::hosted::SetEditTarget(next_active));
    }
    for (entity, doc) in staged {
        if created.contains(&entity)
            && let Some(editor) = registry.editors.get(doc.asset_type)
        {
            editor.build(&mut commands.entity(entity));
        }
        if changed.contains(&entity) {
            commands.insert(doc, entity);
        }
    }
    for (entity, owner) in owned.iter() {
        if closed.contains(&owner.0) {
            commands.despawn(entity);
        }
    }
    for entity in closed {
        stop_game(entity, &mut commands);
        commands.despawn(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{
        IntoSystem, System, World,
        events::{event_channel::EventChannel, event_cursor::EventCursor},
    };
    use concerto_foundation::assets::AssetId;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct TestAsset;
    impl Asset for TestAsset {
        fn name() -> &'static str {
            "TestAsset"
        }
    }
    #[derive(Default)]
    struct Fake;
    impl AssetEditor for Fake {}

    fn asset(path: &str) -> AssetEntry {
        AssetEntry {
            embedded_in: None,
            id: AssetId::from_path(path),
            address: path.into(),
            kind: "TestAsset".into(),
            kind_version: None,
            provenance: Some(concerto_foundation::assets::content::ImportProvenance {
                source: path.into(),
                sub_asset: String::new(),
            }),
        }
    }

    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(AssetEditorCommands::default());
        w.insert_resource(ActiveEditor::default());
        w.insert_resource(crate::guard::UnsavedGuard::default());
        let mut registry = AssetRegistry::default();
        for path in ["a", "b", "c"] {
            registry.insert(asset(path));
        }
        w.insert_resource(registry);
        let mut r = AssetEditorRegistry::default();
        r.register::<TestAsset, Fake>().unwrap();
        w.insert_resource(r);
        w
    }

    fn push(w: &mut World, command: AssetEditorCommand) {
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(command);
        let mut system = process_editor_commands.into_system();
        system.initialize(w);
        system.run_and_apply((), w);
    }

    fn open(w: &mut World, path: &str) -> Entity {
        push(w, AssetEditorCommand::Open(asset(path)));
        w.get_resource::<ActiveEditor>().unwrap().0.unwrap()
    }

    fn finish(w: &mut World, editor: Entity, asset: &AssetEntry, result: Result<()>) -> bool {
        w.get_component_for_entity_mut::<EditorDocument>(editor)
            .is_some_and(|doc| finish_asset_request(doc, asset, result))
    }

    fn current(w: &World, editor: Entity) -> Option<AssetId> {
        w.get_component_for_entity::<EditorDocument>(editor)
            .unwrap()
            .current
            .as_ref()
            .map(|asset| asset.id)
    }

    #[test]
    fn a_document_shows_the_asset_last_opened_and_keeps_it_through_a_stale_or_failed_result() {
        let mut w = world();
        let e = open(&mut w, "a");
        assert!(finish(&mut w, e, &asset("a"), Ok(())));
        assert_eq!(current(&w, e), Some(asset("a").id));

        push(&mut w, AssetEditorCommand::Open(asset("b")));
        push(&mut w, AssetEditorCommand::Open(asset("c")));
        assert!(!finish(&mut w, e, &asset("b"), Ok(())));
        assert_eq!(current(&w, e), Some(asset("a").id));

        assert!(finish(&mut w, e, &asset("c"), Err(anyhow::anyhow!("bad"))));
        assert_eq!(current(&w, e), Some(asset("a").id));

        push(&mut w, AssetEditorCommand::Open(asset("b")));
        assert!(finish(&mut w, e, &asset("b"), Ok(())));
        assert_eq!(current(&w, e), Some(asset("b").id));
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(e));
    }

    #[test]
    fn replacing_or_closing_a_dirty_document_is_held_for_the_guard() {
        let asking_about = |w: &World| {
            w.get_resource::<crate::guard::UnsavedGuard>()
                .unwrap()
                .documents()
                .to_vec()
        };
        let mut w = world();
        let e = open(&mut w, "a");
        assert!(finish(&mut w, e, &asset("a"), Ok(())));
        w.insert(Unsaved, e);

        open(&mut w, "b");
        let doc = w.get_component_for_entity::<EditorDocument>(e).unwrap();
        assert!(doc.pending.is_none());
        assert_eq!(asking_about(&w), [e]);

        w.insert_resource(crate::guard::UnsavedGuard::default());
        push(&mut w, AssetEditorCommand::Close(e));
        assert!(w.entity_is_valid(e));
        assert_eq!(asking_about(&w), [e]);

        w.insert_resource(crate::guard::UnsavedGuard::default());
        w.remove_component::<Unsaved>(e);
        push(&mut w, AssetEditorCommand::Close(e));
        assert!(!w.entity_is_valid(e));
        assert!(asking_about(&w).is_empty());
    }

    #[test]
    fn the_save_action_asks_an_unsaved_document_to_save() {
        let mut w = world();
        let e = open(&mut w, "a");
        assert!(finish(&mut w, e, &asset("a"), Ok(())));
        w.insert_resource(EventChannel::<ActionFired>::default());
        let fire_save = |w: &mut World| {
            let channel = w.get_resource_mut::<EventChannel<ActionFired>>().unwrap();
            channel.update();
            channel.update();
            channel.push_event(ActionFired::new(crate::actions::Save));
            let mut system = request_save.into_system();
            system.initialize(w);
            system.run_and_apply((), w);
            w.get_component_for_entity::<SaveRequested>(e).is_some()
        };

        assert!(!fire_save(&mut w));
        w.insert(Unsaved, e);
        assert!(fire_save(&mut w));
    }

    #[test]
    fn a_save_sends_the_files_it_wrote_as_written() {
        let mut w = World::new();
        w.insert_resource(EventChannel::<SceneSaved>::new());
        w.insert_resource(EventChannel::<AssetsWritten>::new());
        let mut system = report_saves_as_written.into_system();
        system.initialize(&mut w);
        let saved = asset("b");
        let document = w.spawn((Unsaved,));
        w.get_resource_mut::<EventChannel<SceneSaved>>()
            .unwrap()
            .push_event(SceneSaved {
                document,
                asset: asset("a"),
                written: vec![saved.clone()],
            });

        system.run_and_apply((), &mut w);

        let written: Vec<AssetsWritten> = w
            .get_resource::<EventChannel<AssetsWritten>>()
            .unwrap()
            .read_from(&mut EventCursor::default())
            .cloned()
            .collect();
        assert_eq!(
            written,
            [AssetsWritten {
                ids: vec![saved.id]
            }]
        );
    }

    #[test]
    fn asset_names_follow_the_name_rules_on_every_platform() {
        let longest = "a".repeat(64);
        for name in ["New Scene 1", "Forest_Path-2", longest.as_str(), "COM10"] {
            assert_eq!(valid_asset_name(name), Ok(()), "{name:?}");
        }
        let too_long = "a".repeat(65);
        for name in [
            "",
            too_long.as_str(),
            " scene",
            "scene ",
            "CON",
            "nul",
            "Com1",
            "lpt9",
            "a/b",
            "a\\b",
            "a.b",
            "a*b",
            "\u{c9}COLE",
        ] {
            assert!(valid_asset_name(name).is_err(), "{name:?}");
        }
    }
}

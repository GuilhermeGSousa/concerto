use std::{any::Any, path::Path, sync::Arc};

use anyhow::Context;
use concerto_app::{App, InspectedAsset, InspectedAssets, Plugin, schedule_groups::Update};
use concerto_ecs::{
    Changed, Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, With,
    command::{CommandQueue, EntityCommandQueue},
    events::{event_reader::EventReader, event_writer::EventWriter},
    reflect::AppTypeRegistry,
    signal::{On, lifecycle::Add},
};
use concerto_foundation::assets::{
    AssetId,
    content::decode_content_asset,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mesh::{ReflectSkeletonSource, Skeleton};
use concerto_name::Name;
use concerto_reflect::Reflect;

use crate::{
    asset_editor::{
        AssetEditor, AssetEditorRegistry, EditorDocument, SaveRequested, Unsaved,
        finish_asset_request,
    },
    guard::DiscardEdits,
    hosted::{HostRole, Hosted, HostedApp},
    inspector::{DocumentTypes, DocumentValue, InspectedValues, RefreshInspector},
    joint_picker::DocumentBones,
    project::ProjectState,
    scene::{SaveFailed, SceneOpened, SceneSaved},
    tools::edits::PropertyCommitted,
};

/// The generic editor of an asset kind the game marked with `register_inspected_asset`.
pub struct InspectedAssetEditor {
    pub record: InspectedAsset,
}

impl AssetEditor for InspectedAssetEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let document = editor.entity();
        let value = DocumentValue::<InspectedDocument> {
            name: self.record.kind.to_owned(),
            document,
            read: Box::new(|held| held.value.as_deref()),
            edit: Box::new(|held| held.value.as_deref_mut()),
        };
        editor.insert((
            InspectedDocument {
                record: self.record,
                value: None,
                loaded: Vec::new(),
                skeleton: None,
            },
            InspectedValues(vec![Arc::new(value)]),
            DocumentBones(None),
        ));
    }
}

/// What a generic tab holds: its kind's record, the value being edited, and the file as last read or saved.
#[derive(Component)]
pub struct InspectedDocument {
    pub record: InspectedAsset,
    pub value: Option<Box<dyn Reflect>>,
    pub loaded: Vec<u8>,
    pub skeleton: Option<AssetId>,
}

/// The game's inspected asset kinds and type registry, from the last scene's Edit App.
#[derive(Resource, Default)]
pub struct ProjectTypes {
    pub registry: Option<AppTypeRegistry>,
}

/// Learns the game's inspected asset kinds and types from each scene's Edit `App`, and opens, saves and discards them.
pub struct InspectedAssetPlugin;

impl Plugin for InspectedAssetPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ProjectTypes::default())
            .add_system(Update, register_inspected_kinds)
            .add_system(
                Update,
                (
                    follow_inspected_edits,
                    save_inspected_assets,
                    load_inspected_assets,
                    follow_inspected_skeleton,
                )
                    .chain(),
            )
            .add_listener(restore_discarded_value)
            .add_listener(clear_status_on_edit);
    }
}

pub(crate) fn register_inspected_kinds(
    mut opened: EventReader<SceneOpened>,
    hosted: Query<&HostedApp>,
    mut types: ResMut<ProjectTypes>,
    mut editors: ResMut<AssetEditorRegistry>,
) {
    for event in opened.read() {
        let Some(app) = hosted
            .get_entity(event.document)
            .filter(|app| app.role() == HostRole::Edit)
        else {
            continue;
        };
        types.registry = app.type_registry();
        let Some(records) = app.world().get_resource::<InspectedAssets>() else {
            continue;
        };
        for record in records.iter() {
            if editors.editor_kind(record.kind).is_none() {
                editors.register_inspected(*record);
            }
        }
    }
}

struct ReadAsset {
    bytes: Vec<u8>,
    value: Box<dyn Reflect>,
}

fn read_inspected(
    root: &Path,
    asset: &AssetEntry,
    record: &InspectedAsset,
) -> anyhow::Result<ReadAsset> {
    let bytes = std::fs::read(root.join(&asset.address))
        .with_context(|| format!("Cannot read {}", asset.address))?;
    let decoded = (record.decode)(&bytes)?;
    anyhow::ensure!(
        decoded.id == asset.id,
        "{} holds another asset's id",
        asset.address
    );
    Ok(ReadAsset {
        bytes,
        value: decoded.value,
    })
}

pub(crate) fn load_inspected_assets(
    documents: Query<(
        Entity,
        &mut EditorDocument,
        &mut InspectedDocument,
        Option<&DocumentTypes>,
    )>,
    project: Res<ProjectState>,
    types: Res<ProjectTypes>,
    mut cmd: CommandQueue,
) {
    let root = project
        .project
        .as_ref()
        .map(|project| project.root.as_path());
    for (entity, mut document, mut state, held_types) in documents.iter() {
        if held_types.is_none()
            && let Some(registry) = &types.registry
        {
            cmd.insert(DocumentTypes(registry.clone()), entity);
        }
        let Some(root) = root else {
            continue;
        };
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        match read_inspected(root, &asset, &state.record) {
            Ok(read) => {
                state.value = Some(read.value);
                state.loaded = read.bytes;
                cmd.remove::<Unsaved>(entity);
                cmd.trigger(RefreshInspector);
                finish_asset_request(&mut document, &asset, Ok(()));
            }
            Err(error) => {
                finish_asset_request(&mut document, &asset, Err(error));
            }
        }
    }
}

pub(crate) fn follow_inspected_edits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    documents: Query<Entity, With<InspectedDocument>>,
    mut cmd: CommandQueue,
) {
    for hosted in committed.read() {
        if hosted.event.result.is_ok() && documents.get_entity(hosted.document).is_some() {
            cmd.insert(Unsaved, hosted.document);
        }
    }
}

fn save_document(
    document: &mut EditorDocument,
    state: &mut InspectedDocument,
    root: &Path,
    assets: &mut AssetRegistry,
) -> anyhow::Result<AssetEntry> {
    let address = document
        .current
        .as_ref()
        .context("No asset is open")?
        .address
        .clone();
    let value = state.value.as_deref().context("No asset is open")?;
    let id = (state.record.save)(value, root, &address, assets)?;
    let saved = assets
        .get(id)
        .cloned()
        .context("The saved asset was not registered")?;
    state.loaded = std::fs::read(root.join(&saved.address))
        .with_context(|| format!("Cannot read {} back", saved.address))?;
    document.title = saved.name().to_owned();
    document.current = Some(saved.clone());
    Ok(saved)
}

pub(crate) fn save_inspected_assets(
    requested: Query<(Entity, &mut EditorDocument, &mut InspectedDocument), With<SaveRequested>>,
    project: Res<ProjectState>,
    mut assets: ResMut<AssetRegistry>,
    mut saved: EventWriter<SceneSaved>,
    mut failed: EventWriter<SaveFailed>,
    mut cmd: CommandQueue,
) {
    let root = project
        .project
        .as_ref()
        .map(|project| project.root.as_path());
    for (entity, mut document, mut state) in requested.iter() {
        cmd.remove::<SaveRequested>(entity);
        let written = root
            .context("Open a project before saving")
            .and_then(|root| save_document(&mut document, &mut state, root, &mut assets));
        match written {
            Ok(asset) => {
                document.status = format!("Saved {}", asset.address);
                saved.write(SceneSaved {
                    document: entity,
                    asset,
                });
                cmd.remove::<Unsaved>(entity);
            }
            Err(error) => {
                let message = format!("Save failed: {error:#}");
                document.status = message.clone();
                failed.write(SaveFailed {
                    document: entity,
                    message,
                });
            }
        }
    }
}

pub(crate) fn clear_status_on_edit(
    on: On<Add<Unsaved>>,
    documents: Query<&mut EditorDocument, With<InspectedDocument>>,
) {
    if let Some(mut document) = documents.get_entity(on.signal().entity)
        && !document.status.is_empty()
    {
        document.status.clear();
    }
}

pub(crate) fn restore_discarded_value(
    on: On<DiscardEdits>,
    documents: Query<&mut InspectedDocument>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let Some(mut state) = documents.get_entity(document) else {
        return;
    };
    if state.loaded.is_empty() {
        return;
    }
    match (state.record.decode)(&state.loaded) {
        Ok(decoded) => {
            state.value = Some(decoded.value);
            cmd.trigger(RefreshInspector);
        }
        Err(error) => {
            log::warn!("Discarded edits stay: {error:#}");
            cmd.insert(Unsaved, document);
        }
    }
}

fn skeleton_named(value: &dyn Reflect, types: &AppTypeRegistry) -> Option<AssetId> {
    let types = types.read();
    let source = types.get_type_data::<ReflectSkeletonSource>((value as &dyn Any).type_id())?;
    (source.skeleton)(value).filter(|id| !id.is_nil())
}

fn read_bones(root: &Path, id: AssetId, assets: &AssetRegistry) -> anyhow::Result<Vec<Name>> {
    let asset = assets
        .get(id)
        .context("The skeleton is not in the project")?;
    let bytes = std::fs::read(root.join(&asset.address))
        .with_context(|| format!("Cannot read {}", asset.address))?;
    let (header, skeleton) = decode_content_asset::<Skeleton>(&bytes)?;
    anyhow::ensure!(
        header.asset_id == id,
        "{} holds another asset's id",
        asset.address
    );
    Ok(skeleton.names.to_vec())
}

pub(crate) fn follow_inspected_skeleton(
    documents: Query<
        (
            &mut InspectedDocument,
            &mut DocumentBones,
            Option<&DocumentTypes>,
        ),
        Changed<InspectedDocument>,
    >,
    project: Res<ProjectState>,
    types: Res<ProjectTypes>,
    assets: Res<AssetRegistry>,
) {
    let Some(project) = project.project.as_ref() else {
        return;
    };
    for (mut state, mut bones, held_types) in documents.iter() {
        let registry = held_types.map(|types| &types.0).or(types.registry.as_ref());
        let named = registry.and_then(|registry| skeleton_named(state.value.as_deref()?, registry));
        if named == state.skeleton {
            continue;
        }
        state.skeleton = named;
        let read = named.map(|id| read_bones(&project.root, id, &assets));
        bones.0 = match read {
            Some(Ok(names)) => Some(names),
            Some(Err(error)) => {
                log::warn!("The skeleton's bones cannot be read: {error:#}");
                None
            }
            None => None,
        };
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        asset_editor::{AssetEditorRegistry, Creator, SaveRequested},
        guard::DiscardEdits,
        hosted::testing::{SceneFixture, hosted_now, play_request, scene_entry, scene_recipe},
        inspector::{
            InspectedSnapshots, InspectedValues, InspectorRegistry, PropertyCommits, PropertyRow,
            commit_number, spawn_panel, value_systems,
        },
        scene::{SaveFailed, SceneSaved},
    };
    use concerto_ecs::{
        Entity, IntoSystem, Res, System, World, command::CommandQueue,
        events::event_channel::EventChannel, system::BoxedSystem,
    };
    use concerto_foundation::assets::{
        Asset, AssetId, CreatableAsset, LoadableAsset,
        content::{decode_content_asset, save_content_asset_into},
        handle::AssetHandle,
        registry::{AssetEntry, AssetRegistry},
    };
    use concerto_mesh::{JointName, JointPose, ReflectSkeletonSource, Skeleton, SkeletonSource};
    use concerto_name::Name;
    use concerto_reflect::Reflect;
    use concerto_ui::{node::UINode, theme::UITheme};
    use concerto_window::input::actions::{ActionFired, ActionLabel};
    use std::any::TypeId;

    #[derive(serde::Serialize, serde::Deserialize, Reflect, Clone, PartialEq, Debug)]
    struct Note {
        text: String,
        weight: f32,
    }
    impl Asset for Note {
        fn name() -> &'static str {
            "TestNote"
        }
    }
    impl CreatableAsset for Note {
        fn new_asset() -> Self {
            Note {
                text: String::new(),
                weight: 1.0,
            }
        }
    }
    impl LoadableAsset for Note {}

    #[derive(serde::Serialize, serde::Deserialize, Reflect)]
    struct Tag {
        label: String,
    }
    impl Asset for Tag {
        fn name() -> &'static str {
            "TestTag"
        }
    }
    impl CreatableAsset for Tag {
        fn new_asset() -> Self {
            Tag {
                label: String::new(),
            }
        }
    }
    impl LoadableAsset for Tag {}

    #[derive(serde::Serialize, serde::Deserialize, Reflect, Clone)]
    #[reflect(SkeletonSource)]
    pub(crate) struct Rig {
        pub(crate) skeleton: AssetHandle<Skeleton>,
        pub(crate) joint: JointName,
        pub(crate) joints: Vec<JointName>,
    }
    impl Asset for Rig {
        fn name() -> &'static str {
            "TestRig"
        }
    }
    impl CreatableAsset for Rig {
        fn new_asset() -> Self {
            Rig {
                skeleton: AssetHandle::default(),
                joint: JointName::default(),
                joints: Vec::new(),
            }
        }
    }
    impl LoadableAsset for Rig {}
    impl SkeletonSource for Rig {
        fn skeleton(&self) -> Option<&AssetHandle<Skeleton>> {
            Some(&self.skeleton)
        }
    }

    #[derive(Default)]
    struct PinnedNoteEditor;
    impl AssetEditor for PinnedNoteEditor {
        fn closable(&self) -> bool {
            false
        }
    }

    fn inspected_recipe(app: &mut App) {
        scene_recipe(app);
        app.register_inspected_asset::<Note>()
            .register_inspected_asset::<Tag>();
    }

    fn rig_recipe(app: &mut App) {
        inspected_recipe(app);
        app.register_inspected_asset::<Rig>();
    }

    struct Records {
        world: World,
        system: BoxedSystem,
    }

    impl Records {
        fn new(editors: AssetEditorRegistry) -> Self {
            let mut world = World::new();
            world.insert_resource(editors);
            world.insert_resource(ProjectTypes::default());
            world.insert_resource(EventChannel::<SceneOpened>::new());
            let mut system = register_inspected_kinds.into_boxed_system();
            system.initialize(&mut world);
            Self { world, system }
        }

        fn open(&mut self, hosted: HostedApp) -> Entity {
            let document = self.world.spawn(hosted);
            self.world
                .get_resource_mut::<EventChannel<SceneOpened>>()
                .unwrap()
                .push_event(SceneOpened {
                    document,
                    asset: scene_entry("level"),
                });
            self.system.run_and_apply((), &mut self.world);
            document
        }

        fn editors(&self) -> &AssetEditorRegistry {
            self.world.get_resource::<AssetEditorRegistry>().unwrap()
        }

        fn creatable(&self) -> Vec<&'static str> {
            self.editors().creatable().map(|(kind, _)| kind).collect()
        }

        fn knows_note_type(&self) -> bool {
            self.world
                .get_resource::<ProjectTypes>()
                .unwrap()
                .registry
                .as_ref()
                .is_some_and(|registry| registry.read().get(TypeId::of::<Note>()).is_some())
        }
    }

    #[test]
    fn a_scene_opened_on_an_edit_app_registers_its_inspected_kinds_and_types() {
        let mut records = Records::new(AssetEditorRegistry::default());

        records.open(hosted_now(inspected_recipe, |_| {}));

        assert_eq!(records.editors().editor_kind("TestNote"), Some("TestNote"));
        assert_eq!(records.editors().editor_kind("TestTag"), Some("TestTag"));
        assert_eq!(records.creatable(), ["TestNote", "TestTag"]);
        assert!(records.knows_note_type());
    }

    #[test]
    fn a_kind_with_its_own_editor_keeps_it() {
        let mut editors = AssetEditorRegistry::default();
        editors
            .register_creatable::<Note, PinnedNoteEditor>()
            .unwrap();
        let mut records = Records::new(editors);

        records.open(hosted_now(inspected_recipe, |_| {}));

        assert!(!records.editors().closable("TestNote"));
        assert!(records.editors().closable("TestTag"));
        assert_eq!(records.creatable(), ["TestNote", "TestTag"]);
    }

    #[test]
    fn a_game_app_is_ignored() {
        let mut records = Records::new(AssetEditorRegistry::default());
        let game = crate::hosted::begin(play_request(inspected_recipe), |_| {}, |_| {})
            .finish_now()
            .unwrap();

        records.open(game);

        assert_eq!(records.editors().editor_kind("TestNote"), None);
        assert!(records.creatable().is_empty());
        assert!(!records.knows_note_type());
    }

    #[test]
    fn a_kind_whose_new_name_breaks_the_name_rules_is_skipped() {
        let mut editors = AssetEditorRegistry::default();
        let record = InspectedAsset {
            kind: "Bad/Kind",
            create: |_, _, _| anyhow::bail!("unused"),
            decode: |_| anyhow::bail!("unused"),
            save: |_, _, _, _| anyhow::bail!("unused"),
        };

        editors.register_inspected(record);

        assert_eq!(editors.editor_kind("Bad/Kind"), None);
        assert_eq!(editors.creatable().count(), 0);
    }

    #[test]
    fn a_registered_kind_creates_through_its_record() {
        let mut editors = AssetEditorRegistry::default();
        let record = InspectedAsset {
            kind: "Made",
            create: |_, address, _| anyhow::bail!("made {address}"),
            decode: |_| anyhow::bail!("unused"),
            save: |_, _, _, _| anyhow::bail!("unused"),
        };

        editors.register_inspected(record);

        let creators: Vec<&Creator> = editors.creatable().map(|(_, creator)| creator).collect();
        let error = creators[0]
            .create(
                std::path::Path::new("."),
                "content/x.gasset",
                &mut concerto_foundation::assets::registry::AssetRegistry::default(),
            )
            .unwrap_err();
        assert_eq!(format!("{error}"), "made content/x.gasset");
    }

    fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
        let parent = cmd.spawn(UINode::default()).entity();
        spawn_panel(&mut cmd, parent, &theme);
    }

    fn tab_fixture() -> SceneFixture {
        tab_fixture_with(inspected_recipe)
    }

    pub(crate) fn rig_fixture() -> SceneFixture {
        tab_fixture_with(rig_recipe)
    }

    fn tab_fixture_with(recipe: fn(&mut App)) -> SceneFixture {
        let mut f = SceneFixture::new();
        let game = hosted_now(recipe, |_| {});
        let records: Vec<InspectedAsset> = game
            .world()
            .get_resource::<InspectedAssets>()
            .unwrap()
            .iter()
            .copied()
            .collect();
        let editors = f.world.get_resource_mut::<AssetEditorRegistry>().unwrap();
        for record in records {
            editors.register_inspected(record);
        }
        f.world.insert_resource(ProjectTypes {
            registry: game.type_registry(),
        });
        f.world.insert_resource(InspectorRegistry::default());
        f.world.insert_resource(InspectedSnapshots::default());
        let mut panel = build_inspector.into_system();
        panel.initialize(&mut f.world);
        panel.run_and_apply((), &mut f.world);
        f.world.add_listener(restore_discarded_value);
        f.world.add_listener(clear_status_on_edit);
        let mut systems = value_systems();
        let cards = systems.split_off(1);
        for system in systems {
            f.add_delivery(system);
        }
        f.add_delivery(follow_inspected_edits.into_boxed_system());
        f.add_delivery(save_inspected_assets.into_boxed_system());
        f.add_delivery(load_inspected_assets.into_boxed_system());
        f.add_delivery(follow_inspected_skeleton.into_boxed_system());
        for system in cards {
            f.add_delivery(system);
        }
        f
    }

    fn note(text: &str, weight: f32) -> Note {
        Note {
            text: text.into(),
            weight,
        }
    }

    const ADDRESS: &str = "content/notes/first.gasset";

    pub(crate) fn write_asset<A: Asset>(
        f: &mut SceneFixture,
        address: &str,
        asset: &A,
    ) -> AssetEntry {
        let root = f.project_root.clone();
        let assets = f.world.get_resource_mut::<AssetRegistry>().unwrap();
        let id = save_content_asset_into(asset, &root, address, assets).unwrap();
        assets.get(id).unwrap().clone()
    }

    fn write_note(f: &mut SceneFixture, note: &Note) -> AssetEntry {
        write_asset(f, ADDRESS, note)
    }

    pub(crate) fn skeleton_of(names: &[&str]) -> Skeleton {
        let bones = names.len();
        Skeleton::new(
            vec![glam::Mat4::IDENTITY; bones],
            (0..bones)
                .map(|bone| bone.checked_sub(1).map(|parent| parent as u16))
                .collect(),
            names.iter().map(|name| Name::new(name)).collect(),
            vec![JointPose::IDENTITY; bones],
            (0..bones).map(|_| Default::default()).collect(),
        )
        .unwrap()
    }

    pub(crate) const RIG_ADDRESS: &str = "content/rigs/hero.gasset";

    pub(crate) fn rig_on(skeleton: &AssetEntry) -> Rig {
        Rig {
            skeleton: AssetHandle::weak(skeleton.id),
            joint: "Hips".into(),
            joints: vec!["LeftFoot".into(), "Spine".into()],
        }
    }

    pub(crate) fn bones_of(f: &SceneFixture, document: Entity) -> Option<Vec<String>> {
        f.world
            .get_component_for_entity::<DocumentBones>(document)
            .expect("the document carries its bones")
            .0
            .as_ref()
            .map(|bones| bones.iter().map(|bone| bone.as_str().to_owned()).collect())
    }

    fn rig_mut(f: &mut SceneFixture, document: Entity) -> &mut Rig {
        f.world
            .get_component_for_entity_mut::<InspectedDocument>(document)
            .unwrap()
            .value
            .as_deref_mut()
            .and_then(|value| value.downcast_mut::<Rig>())
            .expect("the document holds a rig")
    }

    pub(crate) struct OpenRig {
        pub(crate) f: SceneFixture,
        pub(crate) document: Entity,
        pub(crate) skeleton: AssetEntry,
    }

    pub(crate) fn open_rig() -> OpenRig {
        let mut f = rig_fixture();
        let skeleton = write_asset(
            &mut f,
            "content/rigs/skeleton.gasset",
            &skeleton_of(&["Hips", "Spine", "Head"]),
        );
        let asset = write_asset(&mut f, RIG_ADDRESS, &rig_on(&skeleton));
        let document = request_asset(&mut f, &asset);
        f.frame();
        OpenRig {
            f,
            document,
            skeleton,
        }
    }

    #[test]
    fn an_inspected_asset_naming_a_skeleton_gives_its_document_the_bones() {
        let rig = open_rig();

        assert_eq!(
            bones_of(&rig.f, rig.document).unwrap(),
            ["Hips", "Spine", "Head"]
        );
        assert_eq!(
            held_document(&rig.f, rig.document).skeleton,
            Some(rig.skeleton.id)
        );
    }

    #[test]
    fn an_inspected_asset_that_names_no_skeleton_has_no_bones() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));

        let document = request_asset(&mut f, &asset);
        f.frame();

        assert_eq!(bones_of(&f, document), None);
        assert_eq!(held_document(&f, document).skeleton, None);
    }

    #[test]
    fn an_edit_to_the_skeleton_gives_the_document_the_new_bones() {
        let mut rig = open_rig();
        let other = write_asset(
            &mut rig.f,
            "content/rigs/other.gasset",
            &skeleton_of(&["Root", "Tail"]),
        );

        rig_mut(&mut rig.f, rig.document).skeleton = AssetHandle::weak(other.id);
        rig.f.frame();

        assert_eq!(bones_of(&rig.f, rig.document).unwrap(), ["Root", "Tail"]);
    }

    #[test]
    fn discarding_an_edit_to_the_skeleton_brings_back_its_bones() {
        let mut rig = open_rig();
        let other = write_asset(
            &mut rig.f,
            "content/rigs/other.gasset",
            &skeleton_of(&["Root", "Tail"]),
        );
        rig_mut(&mut rig.f, rig.document).skeleton = AssetHandle::weak(other.id);
        rig.f.frame();

        rig.f.world.trigger(DiscardEdits(rig.document));
        rig.f.frame();

        assert_eq!(
            bones_of(&rig.f, rig.document).unwrap(),
            ["Hips", "Spine", "Head"]
        );
    }

    #[test]
    fn a_skeleton_that_cannot_be_read_leaves_the_document_with_no_bones() {
        let mut rig = open_rig();
        let broken = write_asset(
            &mut rig.f,
            "content/rigs/broken.gasset",
            &skeleton_of(&["Root"]),
        );
        std::fs::write(rig.f.project_root.join(&broken.address), b"not a skeleton").unwrap();

        rig_mut(&mut rig.f, rig.document).skeleton = AssetHandle::weak(broken.id);
        rig.f.frame();

        assert_eq!(bones_of(&rig.f, rig.document), None);
        assert_eq!(
            held_document(&rig.f, rig.document).skeleton,
            Some(broken.id)
        );
    }

    fn file_bytes(f: &SceneFixture) -> Vec<u8> {
        std::fs::read(f.project_root.join(ADDRESS)).unwrap()
    }

    pub(crate) fn request_asset(f: &mut SceneFixture, asset: &AssetEntry) -> Entity {
        f.request(asset);
        let document = f
            .world
            .get_resource::<crate::asset_editor::ActiveEditor>()
            .unwrap()
            .0
            .unwrap();
        f.frames_until("the asset never finished opening", |f| {
            f.document(document).pending.is_none()
        });
        document
    }

    fn held_document(f: &SceneFixture, document: Entity) -> &InspectedDocument {
        f.world
            .get_component_for_entity::<InspectedDocument>(document)
            .expect("the document is an inspected asset's")
    }

    fn held(f: &SceneFixture, document: Entity) -> Option<&Note> {
        held_document(f, document)
            .value
            .as_deref()
            .and_then(|value| value.downcast_ref::<Note>())
    }

    fn weight_row(f: &mut SceneFixture) -> Option<PropertyRow> {
        let world = &mut f.world;
        world
            .query::<&PropertyRow, ()>()
            .iter(world)
            .find(|row| {
                row.address.target.name() == "TestNote" && row.address.path.to_string() == "weight"
            })
            .cloned()
    }

    fn edit_weight(f: &mut SceneFixture, weight: f64) {
        f.frames_until("the card never showed the weight", |f| {
            weight_row(f).is_some()
        });
        let row = weight_row(f).unwrap();
        let commits = f.world.get_resource_mut::<PropertyCommits>().unwrap();
        commit_number(commits, &row, weight).unwrap();
        f.frame();
    }

    pub(crate) fn press_save(f: &mut SceneFixture) {
        let mut fired = EventChannel::<ActionFired>::default();
        fired.push_event(ActionFired {
            action: crate::actions::Save.intern(),
        });
        f.world.insert_resource(fired);
        let mut system = crate::asset_editor::request_save.into_system();
        system.initialize(&mut f.world);
        system.run_and_apply((), &mut f.world);
        f.frame();
    }

    #[test]
    fn an_inspected_asset_opens_in_a_tab_that_holds_its_value_and_its_file() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));

        let document = request_asset(&mut f, &asset);

        assert_eq!(held(&f, document), Some(&note("hello", 1.5)));
        assert_eq!(held_document(&f, document).loaded, file_bytes(&f));
        assert_eq!(f.document(document).current, Some(asset.clone()));
        assert_eq!(f.document(document).title, asset.name());
        assert_eq!(f.status(document), "");
        assert!(!f.unsaved(document));
        let names: Vec<String> = f
            .world
            .get_component_for_entity::<InspectedValues>(document)
            .unwrap()
            .0
            .iter()
            .map(|value| value.name())
            .collect();
        assert_eq!(names, ["TestNote"]);
    }

    #[test]
    fn an_edit_through_the_inspector_marks_the_document_unsaved() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));
        let document = request_asset(&mut f, &asset);

        edit_weight(&mut f, 4.0);

        assert_eq!(held(&f, document), Some(&note("hello", 4.0)));
        assert!(f.unsaved(document));
    }

    #[test]
    fn the_save_action_writes_the_value_and_reports_the_save() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));
        let document = request_asset(&mut f, &asset);
        edit_weight(&mut f, 4.0);

        press_save(&mut f);

        let bytes = file_bytes(&f);
        let (header, saved) = decode_content_asset::<Note>(&bytes).unwrap();
        assert_eq!(header.asset_id, asset.id);
        assert_eq!(saved, note("hello", 4.0));
        assert_eq!(held_document(&f, document).loaded, bytes);
        assert!(!f.unsaved(document));
        assert!(
            f.world
                .get_component_for_entity::<SaveRequested>(document)
                .is_none()
        );
        assert_eq!(
            f.written::<SceneSaved>(),
            [SceneSaved {
                document,
                asset: asset.clone(),
            }]
        );
        assert!(f.written::<SaveFailed>().is_empty());
        assert_eq!(f.document(document).current, Some(asset));
        assert_eq!(f.status(document), format!("Saved {ADDRESS}"));
    }

    #[test]
    fn an_edit_after_a_save_clears_the_saved_status() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));
        let document = request_asset(&mut f, &asset);
        edit_weight(&mut f, 4.0);
        press_save(&mut f);

        edit_weight(&mut f, 9.0);

        assert!(f.unsaved(document));
        assert_eq!(f.status(document), "");
    }

    #[test]
    fn discarding_goes_back_to_the_value_as_last_saved() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));
        let document = request_asset(&mut f, &asset);
        edit_weight(&mut f, 4.0);
        press_save(&mut f);
        edit_weight(&mut f, 9.0);

        f.world.trigger(DiscardEdits(document));
        f.frame();

        assert_eq!(held(&f, document), Some(&note("hello", 4.0)));
    }

    #[test]
    fn a_save_that_cannot_write_reports_it_and_stays_unsaved() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));
        let document = request_asset(&mut f, &asset);
        edit_weight(&mut f, 4.0);
        f.make_project_unwritable();

        press_save(&mut f);

        let failed = f.written::<SaveFailed>();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].document, document);
        assert!(failed[0].message.starts_with("Save failed"));
        assert_eq!(f.status(document), failed[0].message);
        assert!(f.written::<SceneSaved>().is_empty());
        assert!(f.unsaved(document));
        assert_eq!(
            decode_content_asset::<Note>(&file_bytes(&f)).unwrap().1,
            note("hello", 1.5)
        );
    }

    #[test]
    fn a_corrupt_file_fails_the_open_with_a_status() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));
        std::fs::write(f.project_root.join(ADDRESS), b"not a note").unwrap();

        let document = request_asset(&mut f, &asset);

        assert!(
            f.status(document).contains("content asset"),
            "{}",
            f.status(document)
        );
        assert_eq!(f.document(document).current, None);
        assert_eq!(f.document(document).failed, Some(asset));
        assert!(held(&f, document).is_none());
    }

    #[test]
    fn a_file_holding_another_assets_id_fails_the_open() {
        let mut f = tab_fixture();
        let written = write_note(&mut f, &note("hello", 1.5));
        let mistaken = AssetEntry {
            id: AssetId::new(),
            ..written
        };
        f.add_to_project(&mistaken);

        let document = request_asset(&mut f, &mistaken);

        assert!(
            f.status(document).contains("another asset"),
            "{}",
            f.status(document)
        );
        assert!(held(&f, document).is_none());
    }

    #[test]
    fn the_inspector_reads_an_inspected_asset_with_the_projects_types() {
        let mut f = tab_fixture();
        let asset = write_note(&mut f, &note("hello", 1.5));

        request_asset(&mut f, &asset);
        f.frame();

        let knows_note = |types: &AppTypeRegistry| types.read().get(TypeId::of::<Note>()).is_some();
        assert!(
            f.world
                .get_resource::<AppTypeRegistry>()
                .is_none_or(|types| !knows_note(types))
        );
        let snapshots = f.world.get_resource::<InspectedSnapshots>().unwrap();
        assert_eq!(snapshots.values.len(), 1);
        assert!(knows_note(&snapshots.values[0].types));
        assert!(!snapshots.values[0].properties.is_empty());
    }
}

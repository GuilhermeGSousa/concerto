use concerto_app::{App, InspectedAsset, InspectedAssets, Plugin, schedule_groups::Update};
use concerto_ecs::{
    Query, ResMut, Resource, events::event_reader::EventReader, reflect::AppTypeRegistry,
};

use crate::{
    asset_editor::{AssetEditor, AssetEditorRegistry},
    hosted::{HostRole, HostedApp},
    scene::SceneOpened,
};

/// The generic editor of an asset kind the game marked with `register_inspected_asset`.
pub struct InspectedAssetEditor {
    pub record: InspectedAsset,
}

impl AssetEditor for InspectedAssetEditor {}

/// The game's inspected asset kinds and type registry, from the last scene's Edit App.
#[derive(Resource, Default)]
pub struct ProjectTypes {
    pub registry: Option<AppTypeRegistry>,
}

/// Learns the game's inspected asset kinds and types from each scene's Edit `App`.
pub struct InspectedAssetPlugin;

impl Plugin for InspectedAssetPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ProjectTypes::default())
            .add_system(Update, register_inspected_kinds);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset_editor::{AssetEditorRegistry, Creator},
        hosted::testing::{hosted_now, play_request, scene_entry, scene_recipe},
    };
    use concerto_ecs::{
        Entity, IntoSystem, System, World, events::event_channel::EventChannel, system::BoxedSystem,
    };
    use concerto_foundation::assets::{Asset, CreatableAsset, LoadableAsset};
    use concerto_reflect::Reflect;
    use std::any::TypeId;

    #[derive(serde::Serialize, serde::Deserialize, Reflect)]
    struct Note {
        text: String,
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
}

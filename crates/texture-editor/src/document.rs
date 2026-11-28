use std::{path::Path, sync::Arc};

use anyhow::Context;
use concerto_ecs::{
    Component, Entity, Query, Res, With, World,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    reflect::AppTypeRegistry,
    signal::{On, lifecycle::Add},
};
use concerto_editor::{
    asset_editor::{AssetEditor, EditorDocument, Unsaved},
    guard::DiscardEdits,
    hosted::Hosted,
    inspector::{InspectedValues, PropertyTarget, RefreshInspector, TargetWorld},
    project::ProjectState,
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{
    content::{decode_content_asset, read_asset_record_without_uncooked, read_uncooked},
    handle::AssetHandle,
    registry::AssetEntry,
};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_render::assets::texture::{Texture, TextureRole, TextureSettings, TextureSource};

use crate::derive::DeriveJob;

const VALUE: &str = "Texture";

/// What a texture tab edits: the settings, and the role its importer set.
///
/// An edit cannot change the role.
#[derive(Reflect, Clone, Copy, PartialEq, Debug, Default)]
#[reflect(Default)]
pub struct TextureDocument {
    /// What the image is for, set by the importer; an edit that changes it is refused.
    pub role: TextureRole,
    /// The compression, colour space, size and mip settings the levels are derived with.
    pub settings: TextureSettings,
}

impl TextureDocument {
    fn of(texture: &Texture) -> Self {
        Self {
            role: texture.role,
            settings: texture.settings,
        }
    }
}

/// A texture tab's state.
///
/// It holds the value being edited, the file's value as last read or saved, its
/// source, and the derived preview or why deriving it failed.
#[derive(Component, Default)]
pub struct TextureTab {
    /// The value as edited so far, or `None` before the texture has loaded.
    pub value: Option<TextureDocument>,
    /// The file's value as last read or saved, which a discard restores.
    pub loaded: Option<TextureDocument>,
    pub(crate) preview: Option<AssetHandle<Texture>>,
    pub(crate) source: Option<TextureSource>,
    pub(crate) derive_wanted: bool,
    pub(crate) job: Option<DeriveJob>,
    pub(crate) derive_error: Option<String>,
}

/// Opens a `Texture` in a tab that previews its derived levels and edits its settings.
#[derive(Default)]
pub struct TextureEditor;

impl AssetEditor for TextureEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let document = editor.entity();
        editor.insert((
            TextureTab::default(),
            InspectedValues(vec![Arc::new(TextureTarget { document })]),
        ));
    }
}

struct TextureTarget {
    document: Entity,
}

impl PropertyTarget for TextureTarget {
    fn name(&self) -> String {
        VALUE.to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let value = world
            .get_component_for_entity::<TextureTab>(self.document)
            .and_then(|tab| tab.value.as_ref());
        let Some(value) = value else {
            return false;
        };
        read(value);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        self.try_edit(world, edit).unwrap_or(false)
    }

    fn try_edit(
        &self,
        world: &mut World,
        edit: &mut dyn FnMut(&mut dyn Reflect),
    ) -> Result<bool, String> {
        let before = world
            .get_component_for_entity::<TextureTab>(self.document)
            .and_then(|tab| tab.value);
        let Some(before) = before else {
            return Ok(false);
        };
        let mut value = before;
        edit(&mut value);
        if value.role != before.role {
            return Err(format!(
                "A texture's role is set by the importer and stays {:?}; import its source again to change it",
                before.role
            ));
        }
        let Some(tab) = world.get_component_for_entity_mut::<TextureTab>(self.document) else {
            return Ok(false);
        };
        tab.value = Some(value);
        Ok(true)
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        world.get_resource::<AppTypeRegistry>().cloned()
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }
}

pub(crate) struct OpenedTexture {
    pub(crate) texture: Texture,
    pub(crate) source: TextureSource,
}

pub(crate) fn read_texture_payload(root: &Path, asset: &AssetEntry) -> anyhow::Result<Texture> {
    let bytes = read_asset_record_without_uncooked(root, asset)
        .with_context(|| format!("Cannot read {}", asset.location()))?;
    let (header, texture) = decode_content_asset::<Texture>(&bytes)?;
    anyhow::ensure!(
        header.asset_id == asset.id,
        "{} holds another asset's id",
        asset.address
    );
    Ok(texture)
}

pub(crate) fn load_textures(
    mut documents: Query<(Entity, &mut EditorDocument, &mut TextureTab)>,
    project: Res<ProjectState>,
    mut cmd: CommandQueue,
) {
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    for (entity, mut document, mut tab) in documents.iter_mut() {
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        match OpenedTexture::read(&root, &asset) {
            Ok(opened) => {
                tab.hold_texture(opened);
                tab.derive_wanted = true;
                cmd.remove::<Unsaved>(entity);
                cmd.trigger(RefreshInspector);
                document.finish_request(&asset, Ok(()));
            }
            Err(error) => {
                document.finish_request(&asset, Err(error));
            }
        }
    }
}

pub(crate) fn follow_texture_edits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    mut documents: Query<&mut TextureTab>,
    mut cmd: CommandQueue,
) {
    for hosted in committed.read() {
        if hosted.event.result.is_ok()
            && let Some(mut tab) = documents.get_entity_mut(hosted.document)
        {
            tab.derive_wanted = true;
            cmd.insert(Unsaved, hosted.document);
        }
    }
}

pub(crate) fn clear_status_on_edit(
    on: On<Add<Unsaved>>,
    mut documents: Query<&mut EditorDocument, With<TextureTab>>,
) {
    if let Some(mut document) = documents.get_entity_mut(on.signal().entity)
        && !document.status.is_empty()
    {
        document.status.clear();
    }
}

pub(crate) fn restore_discarded_edits(
    on: On<DiscardEdits>,
    mut documents: Query<&mut TextureTab>,
    mut cmd: CommandQueue,
) {
    let Some(mut tab) = documents.get_entity_mut(on.signal().0) else {
        return;
    };
    if tab.loaded.is_some() && tab.value != tab.loaded {
        tab.value = tab.loaded;
        tab.derive_wanted = true;
        cmd.trigger(RefreshInspector);
    }
}

impl OpenedTexture {
    pub(crate) fn read(root: &Path, asset: &AssetEntry) -> anyhow::Result<Self> {
        let texture = read_texture_payload(root, asset)?;
        let source = read_uncooked::<TextureSource>(root, asset)
            .with_context(|| format!("Cannot read the source image of {}", asset.location()))?
            .with_context(|| {
                format!(
                    "{} holds no source image; import its source again",
                    asset.address
                )
            })?;
        Ok(Self { texture, source })
    }
}

impl TextureTab {
    pub(crate) fn hold_document(&mut self, texture: &Texture) {
        let value = TextureDocument::of(texture);
        if self.value != Some(value) {
            self.derive_wanted = true;
        }
        self.value = Some(value);
        self.loaded = Some(value);
    }

    pub(crate) fn hold_texture(&mut self, opened: OpenedTexture) {
        self.hold_document(&opened.texture);
        self.source = Some(opened.source);
    }
}

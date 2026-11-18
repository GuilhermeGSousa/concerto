use std::thread::JoinHandle;

use anyhow::anyhow;
use concerto_color::Color;
use concerto_ecs::{Added, Component, Entity, Query, Res, With, command::CommandQueue};
use concerto_editor::asset_editor::{BackgroundWork, EditorDocument, EditorHosts};
use concerto_foundation::assets::{
    Asset, AssetId,
    asset_server::AssetServer,
    asset_store::AssetStore,
    derived::{DerivedData, DerivedDataCache, DerivedInputs},
};
use concerto_render::assets::texture::{Texture, TextureLevels, TextureSource};
use concerto_ui::{
    material::UIMaterial,
    node::{AlignContent, AlignItems, FlexDirection, UILayout, UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};

use crate::document::{TextureDocument, TextureTab};

pub(crate) struct DeriveJob {
    handle: JoinHandle<anyhow::Result<Texture>>,
    asset: AssetId,
}

fn derived(
    document: TextureDocument,
    source: TextureSource,
    cache: DerivedDataCache,
) -> anyhow::Result<Texture> {
    let (name, version) = (Texture::name(), Texture::DERIVED_VERSION);
    let mut texture = Texture::imported(document.role, document.settings, &source);
    let inputs = DerivedInputs::default().with_own_uncooked(&source)?;
    let key = texture.derived_key(&inputs)?;
    if let Some(blob) = cache.get(name, version, key) {
        match texture.apply_derived(&blob) {
            Ok(()) => return Ok(texture),
            Err(error) => log::warn!("discarding the cached {name} blob {key:016x}: {error:#}"),
        }
    }
    let blob = texture.derive(&inputs, &mut |_, _| {})?;
    cache.put(name, version, key, &blob);
    texture.apply_derived(&blob)?;
    Ok(texture)
}

fn size_line(levels: &TextureLevels) -> String {
    let bytes: usize = levels.levels.iter().map(Vec::len).sum();
    format!(
        "{}×{} · {:?} · {} mips · {:.1} MB on the GPU",
        levels.width,
        levels.height,
        levels.format,
        levels.levels.len(),
        bytes as f64 / (1024.0 * 1024.0)
    )
}

fn finish_derive(
    editor: &mut EditorDocument,
    tab: &mut TextureTab,
    job: DeriveJob,
    server: &AssetServer,
) {
    let result = job
        .handle
        .join()
        .map_err(|_| anyhow!("the worker panicked"))
        .and_then(|derived| derived);
    let current = editor.current.as_ref().map(|asset| asset.id);
    if current != Some(job.asset) || editor.pending.is_some() || tab.derive_wanted {
        return;
    }
    match result {
        Ok(texture) => {
            tab.derive_error = None;
            if let Some(levels) = texture.levels() {
                editor.status = size_line(levels);
            }
            tab.preview = Some(server.add(texture));
        }
        Err(error) => {
            let message = format!("Derive failed: {error:#}");
            editor.status = message.clone();
            tab.derive_error = Some(message);
        }
    }
}

pub(crate) fn derive_textures(
    documents: Query<(&mut EditorDocument, &mut TextureTab)>,
    server: Res<AssetServer>,
) {
    for (mut editor, mut tab) in documents.iter() {
        if tab.job.as_ref().is_some_and(|job| job.handle.is_finished())
            && let Some(job) = tab.job.take()
        {
            finish_derive(&mut editor, &mut tab, job, &server);
        }
        if !tab.derive_wanted || tab.job.is_some() || editor.pending.is_some() {
            continue;
        }
        tab.derive_wanted = false;
        let (Some(asset), Some(value), Some(source)) =
            (editor.current.as_ref(), tab.value, tab.source.clone())
        else {
            continue;
        };
        let cache = server.derived_cache();
        tab.job = Some(DeriveJob {
            handle: std::thread::spawn(move || derived(value, source, cache)),
            asset: asset.id,
        });
    }
}

pub(crate) fn mark_background_work(
    documents: Query<(Entity, &TextureTab, Option<&BackgroundWork>)>,
    mut cmd: CommandQueue,
) {
    for (document, tab, marked) in documents.iter() {
        match (tab.job.is_some(), marked.is_some()) {
            (true, false) => cmd.insert(BackgroundWork, document),
            (false, true) => cmd.remove::<BackgroundWork>(document),
            _ => {}
        }
    }
}

#[derive(Component)]
pub(crate) struct TexturePreview {
    frame: Entity,
    image: Entity,
    line: Entity,
    texels: Option<[f32; 2]>,
}

pub(crate) fn build_previews(
    documents: Query<(Entity, &EditorHosts), (With<TextureTab>, Added<EditorHosts>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, hosts) in documents.iter() {
        let Some(canvas) = hosts.canvas else {
            continue;
        };
        let mut host = cmd.entity(canvas);
        let mut frame = host.spawn_child(
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_direction(FlexDirection::Column)
                .with_align_items(AlignItems::Center)
                .with_justify_content(AlignContent::Center)
                .with_padding(UIRect::all(theme.spacing_md)),
        );
        let image = frame
            .spawn_child((
                UINode::default()
                    .with_size(UIValue::Px(0.0), UIValue::Px(0.0))
                    .with_flex_shrink(0.0),
                UIMaterial::flat(Color::WHITE),
            ))
            .entity();
        let line = frame.spawn_child(theme.label("")).entity();
        let frame = frame.entity();
        cmd.insert(
            TexturePreview {
                frame,
                image,
                line,
                texels: None,
            },
            document,
        );
    }
}

fn fitted(texels: [f32; 2], room: [f32; 2]) -> [f32; 2] {
    let scale = (room[0] / texels[0])
        .min(room[1] / texels[1])
        .clamp(0.0, 1.0);
    [(texels[0] * scale).floor(), (texels[1] * scale).floor()]
}

pub(crate) fn show_previews(
    documents: Query<(&EditorDocument, &TextureTab, &mut TexturePreview)>,
    textures: Option<Res<AssetStore<Texture>>>,
    materials: Query<&mut UIMaterial>,
    layouts: Query<&UILayout>,
    nodes: Query<&mut UINode>,
    texts: Query<&mut UIText>,
) {
    for (editor, tab, mut preview) in documents.iter() {
        if let Some(mut text) = texts.get_entity(preview.line)
            && text.text != editor.status
        {
            text.text = editor.status.clone();
        }
        if let Some(handle) = &tab.preview
            && let Some(mut material) = materials.get_entity(preview.image)
            && material
                .texture
                .as_ref()
                .is_none_or(|shown| shown.id() != handle.id())
            && let Some(levels) = textures
                .as_deref()
                .and_then(|textures| textures.get(handle))
                .and_then(Texture::levels)
        {
            preview.texels = Some([levels.width.max(1) as f32, levels.height.max(1) as f32]);
            material.texture = Some(handle.clone());
        }
        let (Some(texels), Some(frame), Some(line)) = (
            preview.texels,
            layouts.get_entity(preview.frame),
            layouts.get_entity(preview.line),
        ) else {
            continue;
        };
        let room = frame.content_rect.size;
        let [width, height] = fitted(texels, [room.x, room.y - line.rect.size.y]);
        if let Some(mut node) = nodes.get_entity(preview.image)
            && (node.width != UIValue::Px(width) || node.height != UIValue::Px(height))
        {
            node.width = UIValue::Px(width);
            node.height = UIValue::Px(height);
        }
    }
}

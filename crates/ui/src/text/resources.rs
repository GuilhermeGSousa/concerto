use derive_more::{Deref, DerefMut};
use ecs::resource::Resource;

/// One [`glyphon::TextRenderer`] per z-layer that carries text this frame.
///
/// A single renderer draws all of its text in one pass, which would put every
/// label above every quad regardless of paint order. Splitting the text into
/// per-layer batches lets the UI render pass interleave them: layer `n`'s text
/// is drawn after layer `n`'s quads and before layer `n + 1`'s.
#[derive(Resource, Default)]
pub(crate) struct TextRenderers {
    /// Renderers are pooled across frames; only the first `layers.len()` of
    /// them hold this frame's text.
    pub(crate) renderers: Vec<glyphon::TextRenderer>,
    /// The z-layer each prepared renderer belongs to, in ascending order.
    pub(crate) layers: Vec<i32>,
}

#[derive(Resource, Deref)]
pub(crate) struct TextCache(pub(crate) glyphon::Cache);

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct TextSwashCache(pub(crate) glyphon::SwashCache);

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct TextViewport(pub(crate) glyphon::Viewport);

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct TextFontSystem(pub(crate) glyphon::FontSystem);

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct TextAtlas(pub(crate) glyphon::TextAtlas);

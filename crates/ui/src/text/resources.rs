use concerto_ecs::resource::Resource;
use derive_more::{Deref, DerefMut};

/// One [`glyphon::TextRenderer`] per z-layer that carries text this frame.
#[derive(Resource, Default)]
pub(crate) struct TextRenderers {
    /// Renderers are pooled across frames; only some hold this frame's text.
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

use concerto_ecs::resource::Resource;
use derive_more::{Deref, DerefMut};

/// One [`glyphon::TextRenderer`] per z-layer that carries text this frame.
#[derive(Resource, Default)]
pub(crate) struct TextRenderers {
    /// The renderer of the z-layer at the same index of `layers`.
    pub(crate) renderers: Vec<glyphon::TextRenderer>,
    /// The z-layers that carry text, in ascending order.
    pub(crate) layers: Vec<i32>,
    /// Renderers whose z-layer no longer carries text, kept for reuse.
    pub(crate) spare: Vec<glyphon::TextRenderer>,
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

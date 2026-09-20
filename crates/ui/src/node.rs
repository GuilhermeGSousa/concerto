#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use app::extractor::Extracted;
use ecs::{
    command::CommandQueue,
    component::Component,
    entity::{
        Entity,
        hierarchy::{ChildOf, Children},
    },
    query::{Query, filter::Without},
    resource::{Res, Resource},
};
use essential::assets::handle::AssetHandle;
use glam::Vec2;
use log::warn;
use render::{
    assets::{material::AsBindGroup, texture::Texture},
    components::render_entity::{RenderEntity, SyncWithRenderWorld},
    device::RenderDevice,
    render_asset::{
        RenderAssets,
        render_texture::{DummyRenderTexture, RenderTexture},
    },
};
pub use taffy::{AlignContent, AlignItems, FlexDirection, Overflow, Position};
use taffy::{
    AvailableSpace, Dimension, Display, LengthPercentage, LengthPercentageAuto, NodeId, Point,
    Rect, Size, Style, TaffyTree,
};
use wgpu::{Buffer, util::DeviceExt};
use window::plugin::Window;

use crate::{
    material::UIMaterial,
    resources::UIRenderDiagnostics,
    transform::UIValue,
    vertex::{QUAD_INDICES, UIVertex},
};

/// A uniform padding/margin value for one or all sides of a UI node (in pixels).
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct UIRect {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl UIRect {
    /// Applies the same value to all four sides.
    pub fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// Applies `vertical` to top/bottom and `horizontal` to left/right.
    pub fn axes(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    fn to_taffy_padding(self) -> Rect<LengthPercentage> {
        Rect {
            top: LengthPercentage::length(self.top),
            right: LengthPercentage::length(self.right),
            bottom: LengthPercentage::length(self.bottom),
            left: LengthPercentage::length(self.left),
        }
    }

    fn to_taffy_margin(self) -> Rect<LengthPercentageAuto> {
        Rect {
            top: LengthPercentageAuto::length(self.top),
            right: LengthPercentageAuto::length(self.right),
            bottom: LengthPercentageAuto::length(self.bottom),
            left: LengthPercentageAuto::length(self.left),
        }
    }

    fn inset_box(self, rect: UIBox) -> UIBox {
        let min = rect.min + Vec2::new(self.left, self.top);
        let size = Vec2::new(
            (rect.size.x - self.left - self.right).max(0.0),
            (rect.size.y - self.top - self.bottom).max(0.0),
        );
        UIBox { min, size }
    }
}

/// Offsets of a positioned node from its parent's edges.
///
/// Each side is independent and defaults to [`UIValue::Auto`], which is what
/// makes `UIInset { right: Px(18.0), ..Default::default() }` mean "18 from the
/// right and wherever the layout puts it otherwise". A plain length rect cannot
/// express that: a zero on the opposite side pins the node there instead.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct UIInset {
    pub top: UIValue,
    pub right: UIValue,
    pub bottom: UIValue,
    pub left: UIValue,
}

impl UIInset {
    fn to_taffy(self) -> Rect<LengthPercentageAuto> {
        fn side(value: UIValue) -> LengthPercentageAuto {
            match value {
                UIValue::Auto => LengthPercentageAuto::auto(),
                UIValue::Px(px) => LengthPercentageAuto::length(px),
                UIValue::Percent(percent) => LengthPercentageAuto::percent(percent / 100.0),
            }
        }
        Rect {
            top: side(self.top),
            right: side(self.right),
            bottom: side(self.bottom),
            left: side(self.left),
        }
    }
}

/// An axis-aligned rectangle in logical pixels, relative to the window origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UIBox {
    pub min: Vec2,
    pub size: Vec2,
}

impl UIBox {
    pub fn max(self) -> Vec2 {
        self.min + self.size
    }

    pub fn contains(self, point: Vec2) -> bool {
        let max = self.max();
        point.x >= self.min.x && point.y >= self.min.y && point.x <= max.x && point.y <= max.y
    }

    pub fn intersection(self, other: Self) -> Self {
        let min = self.min.max(other.min);
        let max = self.max().min(other.max()).max(min);
        Self {
            min,
            size: max - min,
        }
    }
}

// User defined layout data
#[derive(Component, Clone, Debug, PartialEq)]
pub struct UINode {
    pub width: UIValue,
    pub height: UIValue,
    pub min_width: UIValue,
    pub min_height: UIValue,
    pub max_width: UIValue,
    pub max_height: UIValue,
    pub flex_direction: FlexDirection,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    /// Horizontal and vertical space between children, in logical pixels.
    pub gap: Vec2,
    pub align_items: Option<AlignItems>,
    pub align_self: Option<AlignItems>,
    pub justify_content: Option<AlignContent>,
    pub padding: UIRect,
    pub margin: UIRect,
    pub position: Position,
    pub inset: UIInset,
    pub visible: bool,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    /// Explicit paint offset. Tree order remains the deterministic tie-breaker.
    pub z_index: i32,
}

impl UINode {
    /// Whether both axes are pinned, so no amount of text can resize this node.
    ///
    /// A per-frame-changing label on a rigid node — a readout, a counter — would
    /// otherwise invalidate the whole layout pass every frame for a size that
    /// cannot move.
    fn is_rigid(&self) -> bool {
        matches!(self.width, UIValue::Px(_) | UIValue::Percent(_))
            && matches!(self.height, UIValue::Px(_) | UIValue::Percent(_))
    }

    /// Clips descendants to this node's box on both axes. Containers that own a
    /// fixed region of the screen need this: without it, children that no
    /// longer fit keep painting over their neighbours.
    pub fn clipped(mut self) -> Self {
        self.overflow_x = Overflow::Hidden;
        self.overflow_y = Overflow::Hidden;
        self
    }

    fn size(&self) -> Size<Dimension> {
        Size {
            width: dimension(self.width),
            height: dimension(self.height),
        }
    }

    fn dimensions(width: UIValue, height: UIValue) -> Size<Dimension> {
        Size {
            width: dimension(width),
            height: dimension(height),
        }
    }

    fn style(&self) -> Style {
        Style {
            size: self.size(),
            min_size: Self::dimensions(self.min_width, self.min_height),
            max_size: Self::dimensions(self.max_width, self.max_height),
            flex_direction: self.flex_direction,
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            gap: Size {
                width: LengthPercentage::length(self.gap.x),
                height: LengthPercentage::length(self.gap.y),
            },
            align_items: self.align_items,
            align_self: self.align_self,
            justify_content: self.justify_content,
            padding: self.padding.to_taffy_padding(),
            margin: self.margin.to_taffy_margin(),
            position: self.position,
            inset: self.inset.to_taffy(),
            display: if self.visible {
                Display::Flex
            } else {
                Display::None
            },
            overflow: Point {
                x: self.overflow_x,
                y: self.overflow_y,
            },
            ..Default::default()
        }
    }
}

fn dimension(value: UIValue) -> Dimension {
    match value {
        UIValue::Auto => Dimension::auto(),
        UIValue::Px(value) => Dimension::length(value),
        // Taffy works in fractions; `UIValue::Percent` is on a 0-100 scale.
        UIValue::Percent(value) => Dimension::percent(value / 100.0),
    }
}

impl Default for UINode {
    fn default() -> Self {
        Self {
            width: Default::default(),
            height: Default::default(),
            min_width: Default::default(),
            min_height: Default::default(),
            max_width: Default::default(),
            max_height: Default::default(),
            flex_direction: Default::default(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            gap: Vec2::ZERO,
            align_items: None,
            align_self: None,
            justify_content: None,
            padding: UIRect::default(),
            margin: UIRect::default(),
            position: Position::Relative,
            inset: UIInset::default(),
            visible: true,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            z_index: 0,
        }
    }
}

/// Computed read-only layout in logical pixels.
#[derive(Component)]
pub struct UILayout {
    pub rect: UIBox,
    pub content_rect: UIBox,
    pub clip_rect: UIBox,
    pub paint_order: i64,
}

/// Layout instrumentation consumed by the showcase and profilers.
#[derive(Resource, Default)]
pub struct UILayoutDiagnostics {
    pub layout_passes: u64,
    pub tree_rebuilds: u64,
}

/// What the layout pass needs to know to measure one text node.
///
/// A snapshot rather than a borrow, because Taffy holds the context for the
/// duration of the solve while the query that produced it is long since done.
pub(crate) struct TextMeasure {
    text: String,
    font_size: f32,
    line_height: f32,
    family: crate::text::FontFamily,
    weight: u16,
    italic: bool,
    wrap: bool,
    /// Not part of the measured size, but it decides whether the node may be
    /// laid out narrower than that size.
    ellipsis: bool,
    signature: u64,
}

/// Shapes text so the layout pass can size nodes to their content.
///
/// Text is the one thing Taffy cannot size on its own: it has no idea what a
/// glyph is. Without this, any node hugging a label collapses to its padding,
/// which is why buttons, tabs, pills and strips all had to carry hard-coded
/// dimensions.
#[derive(Resource)]
pub struct UITextMeasure {
    font_system: glyphon::FontSystem,
    /// Taffy asks for the same node at several widths while solving, so a
    /// measured size is worth keeping for the rest of the pass.
    cache: HashMap<(u64, u32), Vec2>,
}

impl UITextMeasure {
    pub(crate) fn new(font_system: glyphon::FontSystem) -> Self {
        Self {
            font_system,
            cache: HashMap::new(),
        }
    }

    /// Size of `measure`'s text when laid out into `width`, in logical pixels.
    fn measure(&mut self, measure: &TextMeasure, width: Option<f32>) -> Vec2 {
        let key = (measure.signature, width.map_or(u32::MAX, f32::to_bits));
        if let Some(size) = self.cache.get(&key) {
            return *size;
        }

        let mut buffer = glyphon::Buffer::new(
            &mut self.font_system,
            glyphon::Metrics {
                font_size: measure.font_size,
                line_height: measure.line_height,
            },
        );
        buffer.set_size(&mut self.font_system, width, None);
        buffer.set_wrap(
            &mut self.font_system,
            if measure.wrap {
                glyphon::Wrap::Word
            } else {
                glyphon::Wrap::None
            },
        );
        buffer.set_text(
            &mut self.font_system,
            &measure.text,
            text_attrs(measure.family.clone(), measure.weight, measure.italic),
            glyphon::Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut widest = 0.0_f32;
        let mut lines = 0.0_f32;
        for run in buffer.layout_runs() {
            widest = widest.max(run.line_w);
            lines += 1.0;
        }
        // Ceil the width: a fractional advance that Taffy floors would clip the
        // last glyph, and a hair of slack never shows.
        let size = Vec2::new(widest.ceil(), lines * measure.line_height);
        self.cache.insert(key, size);
        size
    }

    fn clear(&mut self) {
        self.cache.clear();
    }
}

/// Shared by measurement and rendering so both shape the same glyphs.
pub(crate) fn text_attrs(
    family: crate::text::FontFamily,
    weight: u16,
    italic: bool,
) -> glyphon::Attrs<'static> {
    use crate::text::FontFamily;
    let family = match family {
        FontFamily::SansSerif => glyphon::Family::SansSerif,
        FontFamily::Serif => glyphon::Family::Serif,
        FontFamily::Monospace => glyphon::Family::Monospace,
        // Interned rather than leaked: this runs per text node per frame, so
        // leaking a fresh copy of the name would grow without bound.
        FontFamily::Name(name) => glyphon::Family::Name(intern_family(&name)),
    };
    glyphon::Attrs::new()
        .family(family)
        .weight(glyphon::Weight(weight))
        .style(if italic {
            glyphon::Style::Italic
        } else {
            glyphon::Style::Normal
        })
}

/// Interns a family name for the lifetime of the process.
///
/// `glyphon::Attrs<'static>` needs a `&'static str`, and the set of family
/// names an application uses is small and fixed.
fn intern_family(name: &str) -> &'static str {
    static NAMES: std::sync::Mutex<Option<std::collections::HashSet<&'static str>>> =
        std::sync::Mutex::new(None);
    let mut guard = NAMES.lock().expect("family interner poisoned");
    let names = guard.get_or_insert_with(std::collections::HashSet::new);
    if let Some(interned) = names.get(name) {
        return interned;
    }
    let interned: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(interned);
    interned
}

#[derive(Resource)]
pub(crate) struct UILayoutEngine {
    hierarchy_signature: Vec<(Entity, Vec<Entity>)>,
    style_snapshot: Vec<(Entity, UINode)>,
    text_snapshot: Vec<(Entity, u64)>,
    logical_size: Vec2,
    scale_factor: f64,
}

impl Default for UILayoutEngine {
    fn default() -> Self {
        Self {
            hierarchy_signature: Vec::new(),
            style_snapshot: Vec::new(),
            text_snapshot: Vec::new(),
            logical_size: Vec2::ZERO,
            scale_factor: 0.0,
        }
    }
}

impl UILayout {
    pub fn location(&self) -> Vec2 {
        self.rect.min
    }

    pub fn size(&self) -> Vec2 {
        self.rect.size
    }
}

#[derive(Component)]
pub(crate) struct RenderUINode {
    pub(crate) index_buffer: Buffer,
    pub(crate) index_count: u32,
    pub(crate) vertex_buffer: Buffer,
    source_rect: UIBox,
    clip_rect: UIBox,
    /// Physical surface the baked NDC vertices were computed against. Vertices
    /// are stored in NDC, so a surface resize invalidates them even when the
    /// node's logical rect is unchanged.
    surface: (u32, u32, u64),
    pub(crate) z_index: i64,
}

#[derive(Component)]
pub(crate) struct RenderUIMaterial {
    pub(crate) material_bind_group: wgpu::BindGroup,
    signature: [u32; 19],
}

pub(crate) fn compute_ui_nodes(
    ui_nodes: Query<(Entity, &UINode, Option<&Children>)>,
    ui_roots: Query<(Entity, &UINode, Option<&Children>), Without<ChildOf>>,
    texts: Query<&crate::text::TextComponent>,
    window: Res<Window>,
    mut engine: ecs::resource::ResMut<UILayoutEngine>,
    mut measurer: ecs::resource::ResMut<UITextMeasure>,
    mut diagnostics: ecs::resource::ResMut<UILayoutDiagnostics>,
    mut cmd: CommandQueue,
) {
    let logical_size = window.logical_size();
    let window_size = Size {
        width: AvailableSpace::Definite(logical_size.x),
        height: AvailableSpace::Definite(logical_size.y),
    };

    let mut signature = ui_nodes
        .iter()
        .map(|(entity, _, children)| {
            (
                entity,
                children
                    .map(|children| children.iter().copied().collect())
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    signature.sort_by_key(|(entity, _)| (entity.index(), entity.generation()));
    let mut styles = ui_nodes
        .iter()
        .map(|(entity, node, _)| (entity, node.clone()))
        .collect::<Vec<_>>();
    styles.sort_by_key(|(entity, _)| (entity.index(), entity.generation()));
    // Text is part of the layout now, so a changed label has to invalidate the
    // pass exactly as a changed style does.
    let mut text_snapshot = ui_nodes
        .iter()
        .filter(|(_, node, _)| !node.is_rigid())
        .filter_map(|(entity, _, _)| {
            texts
                .get_entity(entity)
                .map(|text| (entity, text_measure_signature(text)))
        })
        .collect::<Vec<_>>();
    text_snapshot.sort_by_key(|(entity, _)| (entity.index(), entity.generation()));

    let hierarchy_changed = engine.hierarchy_signature != signature;
    if hierarchy_changed {
        diagnostics.tree_rebuilds += 1;
    }
    let scale_factor = window.scale_factor();
    if !hierarchy_changed
        && engine.style_snapshot == styles
        && engine.text_snapshot == text_snapshot
        && engine.logical_size == logical_size
        && engine.scale_factor == scale_factor
    {
        return;
    }
    engine.hierarchy_signature = signature;
    engine.style_snapshot = styles;
    engine.text_snapshot = text_snapshot;
    engine.logical_size = logical_size;
    engine.scale_factor = scale_factor;

    // Taffy's compact style representation is deliberately !Send/!Sync, so it
    // cannot live in an ECS Resource. Keep the structural signature retained
    // for invalidation diagnostics while the short-lived solver stays local to
    // this system invocation.
    // Sizes are only valid for this pass: the text they were measured from may
    // have changed, which is what got us here.
    measurer.clear();

    let mut taffy: TaffyTree<TextMeasure> = TaffyTree::new();
    let mut entity_to_taffy = HashMap::new();
    for (entity, node, children) in ui_roots.iter() {
        let Ok(node_id) = new_node(&mut taffy, node, &texts, entity) else {
            warn!("Error adding root UI node");
            continue;
        };
        entity_to_taffy.insert(entity, node_id);
        if let Some(children) = children {
            build_taffy_tree(
                &mut taffy,
                node_id,
                children,
                &ui_nodes,
                &texts,
                &mut entity_to_taffy,
            );
        }
    }

    for (entity, _, _) in ui_roots.iter() {
        let Some(&node_id) = entity_to_taffy.get(&entity) else {
            continue;
        };
        let measurer = &mut *measurer;
        let result = taffy.compute_layout_with_measure(
            node_id,
            window_size,
            |known, available, _id, context, _style| {
                measure_node(measurer, known, available, context)
            },
        );
        if let Err(error) = result {
            warn!("Error computing UI layout: {}", error);
        }
    }
    diagnostics.layout_passes += 1;

    // Phase 2: walk each root and accumulate absolute screen positions as we
    // descend.  Taffy's `layout().location` is relative to the parent, so we
    // must add the parent's absolute position at every level.
    let window_clip = UIBox {
        min: Vec2::ZERO,
        size: logical_size,
    };
    let mut sequence = 0_i64;
    for (entity, root_node, children) in ui_roots.iter() {
        let Some(&node_id) = entity_to_taffy.get(&entity) else {
            continue;
        };
        let Ok(layout) = taffy.layout(node_id) else {
            continue;
        };

        let abs_pos = Vec2::new(layout.location.x, layout.location.y);
        let size = Vec2::new(layout.size.width, layout.size.height);

        let rect = UIBox { min: abs_pos, size };
        let clip_rect = node_clip(rect, window_clip, root_node);
        cmd.insert(
            (
                UILayout {
                    rect,
                    content_rect: root_node.padding.inset_box(rect),
                    clip_rect,
                    paint_order: paint_order(root_node.z_index, sequence),
                },
                SyncWithRenderWorld,
            ),
            entity,
        );
        sequence += 1;

        if let Some(children) = children {
            write_absolute_positions(
                &taffy,
                abs_pos,
                clip_rect,
                children,
                &ui_nodes,
                &entity_to_taffy,
                &mut cmd,
                &mut sequence,
            );
        }
    }
}

/// What Taffy asks of one measured leaf, answered in logical pixels.
fn measure_node(
    measurer: &mut UITextMeasure,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    context: Option<&mut TextMeasure>,
) -> Size<f32> {
    let Some(measure) = context else {
        return Size::ZERO;
    };
    // An explicit size wins outright; there is nothing to work out.
    if let (Some(width), Some(height)) = (known.width, known.height) {
        return Size { width, height };
    }
    let constraint = match (known.width, available.width) {
        (Some(width), _) => Some(width),
        // Min-content asks how narrow this can get, which for wrapped text is
        // its longest unbreakable word; max-content asks for one long line.
        (None, AvailableSpace::Definite(width)) => Some(width),
        (None, AvailableSpace::MinContent) => Some(0.0),
        (None, AvailableSpace::MaxContent) => None,
    };
    let size = measurer.measure(measure, constraint);
    Size {
        width: known.width.unwrap_or(size.x),
        height: known.height.unwrap_or(size.y),
    }
}

/// A node's measure context, when it carries text.
fn text_context(texts: &Query<&crate::text::TextComponent>, entity: Entity) -> Option<TextMeasure> {
    let text = texts.get_entity(entity)?;
    Some(TextMeasure {
        signature: text_measure_signature(text),
        text: text.text.clone(),
        font_size: text.font_size,
        line_height: text.line_height,
        family: text.font_family.clone(),
        weight: text.font_weight,
        italic: text.font_style == crate::text::FontStyle::Italic,
        wrap: text.wrap,
        ellipsis: text.ellipsis,
    })
}

/// Everything that changes a text node's measured size.
fn text_measure_signature(text: &crate::text::TextComponent) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.text.hash(&mut hasher);
    text.font_size.to_bits().hash(&mut hasher);
    text.line_height.to_bits().hash(&mut hasher);
    text.font_weight.hash(&mut hasher);
    (text.font_style as u8).hash(&mut hasher);
    text.wrap.hash(&mut hasher);
    match &text.font_family {
        crate::text::FontFamily::SansSerif => 0_u8.hash(&mut hasher),
        crate::text::FontFamily::Serif => 1_u8.hash(&mut hasher),
        crate::text::FontFamily::Monospace => 2_u8.hash(&mut hasher),
        crate::text::FontFamily::Name(name) => {
            3_u8.hash(&mut hasher);
            name.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Adds one node, giving it a measure context when it carries text.
fn new_node(
    taffy: &mut TaffyTree<TextMeasure>,
    node: &UINode,
    texts: &Query<&crate::text::TextComponent>,
    entity: Entity,
) -> Result<NodeId, taffy::TaffyError> {
    match text_context(texts, entity) {
        Some(measure) => taffy.new_leaf_with_context(text_leaf_style(node, &measure), measure),
        None => taffy.new_leaf(node.style()),
    }
}

/// The style of a node carrying text.
///
/// A label that ellipsises is allowed to be narrower than its text — that is
/// what the ellipsis is for — so it opts out of the flex automatic minimum
/// size, exactly as `min-width: 0` does in CSS. Without this it pushes the row
/// holding it wider than the panel, and the text spills out instead of ending
/// in an ellipsis.
fn text_leaf_style(node: &UINode, measure: &TextMeasure) -> Style {
    let mut style = node.style();
    if measure.ellipsis && !measure.wrap && matches!(node.min_width, UIValue::Auto) {
        style.min_size.width = Dimension::length(0.0);
    }
    style
}

/// Iteratively registers descendants, avoiding call-stack growth for deep trees.
fn build_taffy_tree(
    taffy: &mut TaffyTree<TextMeasure>,
    parent_id: NodeId,
    children: &Children,
    ui_nodes: &Query<(Entity, &UINode, Option<&Children>)>,
    texts: &Query<&crate::text::TextComponent>,
    entity_to_taffy: &mut HashMap<Entity, NodeId>,
) {
    let mut stack = vec![(parent_id, children.iter().copied().collect::<Vec<_>>())];
    while let Some((parent, child_entities)) = stack.pop() {
        let mut descendants = Vec::new();
        for child in child_entities {
            let Some((_, child_node, grand_children)) = ui_nodes.get_entity(child) else {
                continue;
            };
            let Ok(child_id) = new_node(taffy, child_node, texts, child) else {
                continue;
            };
            if taffy.add_child(parent, child_id).is_err() {
                continue;
            }
            entity_to_taffy.insert(child, child_id);
            if let Some(grand_children) = grand_children {
                descendants.push((child_id, grand_children.iter().copied().collect()));
            }
        }
        stack.extend(descendants.into_iter().rev());
    }
}

/// Iteratively writes [`UILayout`] with absolute logical coordinates
/// for every node in the subtree rooted at `children`.
///
/// `parent_origin` is the absolute screen position of the parent so we can
/// convert each child's parent-relative `location` to an absolute position.
fn write_absolute_positions(
    taffy: &TaffyTree<TextMeasure>,
    parent_origin: Vec2,
    parent_clip: UIBox,
    children: &Children,
    ui_nodes: &Query<(Entity, &UINode, Option<&Children>)>,
    entity_to_taffy: &HashMap<Entity, NodeId>,
    cmd: &mut CommandQueue,
    sequence: &mut i64,
) {
    let mut stack = children
        .iter()
        .copied()
        .map(|entity| (parent_origin, parent_clip, entity))
        .collect::<Vec<_>>();
    stack.reverse();
    while let Some((origin, inherited_clip, child_entity)) = stack.pop() {
        let Some(&node_id) = entity_to_taffy.get(&child_entity) else {
            continue;
        };
        let Ok(layout) = taffy.layout(node_id) else {
            continue;
        };

        // layout.location is relative to the parent — add the parent's
        // absolute position to obtain the screen-space position.
        let abs_pos = origin + Vec2::new(layout.location.x, layout.location.y);
        let size = Vec2::new(layout.size.width, layout.size.height);
        let Some((_, node, grand_children)) = ui_nodes.get_entity(child_entity) else {
            continue;
        };
        let rect = UIBox { min: abs_pos, size };
        let clip_rect = node_clip(rect, inherited_clip, node);

        cmd.insert(
            (
                UILayout {
                    rect,
                    content_rect: node.padding.inset_box(rect),
                    clip_rect,
                    paint_order: paint_order(node.z_index, *sequence),
                },
                SyncWithRenderWorld,
            ),
            child_entity,
        );
        *sequence += 1;

        if let Some(grand_children) = grand_children {
            let mut descendants = grand_children
                .iter()
                .copied()
                .map(|entity| (abs_pos, clip_rect, entity))
                .collect::<Vec<_>>();
            descendants.reverse();
            stack.extend(descendants);
        }
    }
}

fn node_clip(rect: UIBox, inherited: UIBox, node: &UINode) -> UIBox {
    if node.overflow_x == Overflow::Visible && node.overflow_y == Overflow::Visible {
        inherited
    } else {
        let mut clipped = inherited;
        if node.overflow_x != Overflow::Visible {
            let x = UIBox {
                min: Vec2::new(rect.min.x, clipped.min.y),
                size: Vec2::new(rect.size.x, clipped.size.y),
            };
            clipped = clipped.intersection(x);
        }
        if node.overflow_y != Overflow::Visible {
            let y = UIBox {
                min: Vec2::new(clipped.min.x, rect.min.y),
                size: Vec2::new(clipped.size.x, rect.size.y),
            };
            clipped = clipped.intersection(y);
        }
        clipped
    }
}

fn paint_order(z_index: i32, sequence: i64) -> i64 {
    ((z_index as i64) << 32) + sequence
}

pub(crate) fn extract_ui_nodes(
    computed_nodes: Extracted<Query<(&UILayout, &RenderEntity)>>,
    render_ui_nodes: Query<&mut RenderUINode>,
    device: Res<RenderDevice>,
    window: Extracted<Res<Window>>,
    diagnostics: Res<UIRenderDiagnostics>,
    mut cmd: CommandQueue,
) {
    let win_w = window.width() as f32;
    let win_h = window.height() as f32;
    let scale = window.scale_factor() as f32;
    let surface = (window.width(), window.height(), scale.to_bits() as u64);

    // Convert pixel coordinates to Normalized Device Coordinates (NDC).
    // Screen space: (0,0) = top-left corner, Y increases downward.
    // NDC space:    (-1,-1) = bottom-left, (+1,+1) = top-right, Y increases upward.
    let to_ndc = |px: f32, py: f32| -> [f32; 2] {
        [
            (px / win_w) * 2.0 - 1.0, // map [0, width]  → [-1, +1]
            1.0 - (py / win_h) * 2.0, // map [0, height] → [+1, -1] (flip Y)
        ]
    };

    for (computed_node, render_entity) in computed_nodes.iter() {
        let draw_rect = computed_node.rect.intersection(computed_node.clip_rect);
        let location = draw_rect.min * scale;
        let size = draw_rect.size * scale;
        if render_ui_nodes
            .get_entity(**render_entity)
            .is_some_and(|node| {
                node.source_rect == computed_node.rect
                    && node.clip_rect == computed_node.clip_rect
                    && node.surface == surface
                    && node.z_index == computed_node.paint_order
            })
        {
            continue;
        }
        diagnostics.record_geometry_rebuild();
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI Index Buffer"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let x = location.x;
        let y = location.y;
        let w = size.x;
        let h = size.y;
        let source_size = computed_node.rect.size.max(Vec2::splat(f32::EPSILON));
        let relative_min = (draw_rect.min - computed_node.rect.min) / source_size;
        let relative_max = (draw_rect.max() - computed_node.rect.min) / source_size;

        let vertices = [
            UIVertex {
                pos_coords: to_ndc(x, y),
                uv: relative_min.into(),
            }, // top-left
            UIVertex {
                pos_coords: to_ndc(x, y + h),
                uv: [relative_min.x, relative_max.y],
            }, // bottom-left
            UIVertex {
                pos_coords: to_ndc(x + w, y + h),
                uv: relative_max.into(),
            }, // bottom-right
            UIVertex {
                pos_coords: to_ndc(x + w, y),
                uv: [relative_max.x, relative_min.y],
            }, // top-right
        ];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let render_ui_node = RenderUINode {
            index_buffer,
            index_count: QUAD_INDICES.len() as u32,
            vertex_buffer,
            source_rect: computed_node.rect,
            clip_rect: computed_node.clip_rect,
            surface,
            z_index: computed_node.paint_order,
        };

        if let Some(mut node) = render_ui_nodes.get_entity(**render_entity) {
            **node = render_ui_node;
        } else {
            cmd.insert(render_ui_node, **render_entity);
        }
    }
}

/// Syncs the user-facing material fields into the GPU-side uniforms each frame.
///
/// `border_params` carries the node's measured size because the shader needs
/// pixels to round corners and inset borders at any scale; `flags` carries
/// whether a texture is bound, because the dummy texture bound in its absence
/// reads as zeros and would otherwise multiply the fill away.
///
/// Runs in `LateUpdate` after `compute_ui_nodes`. Writing these marks the
/// material changed, so `extract_ui_materials` rebuilds the bind group.
pub(crate) fn sync_material_params(nodes: Query<(&UILayout, &mut UIMaterial)>) {
    for (node, mut material) in nodes.iter() {
        let params = [
            material.border_width,
            node.rect.size.x,
            node.rect.size.y,
            material.corner_radius,
        ];
        if material.border_params != params {
            material.border_params = params;
        }
        let has_texture = if material.texture.is_some() { 1.0 } else { 0.0 };
        if material.flags[1] != material.rotation {
            material.flags[1] = material.rotation;
        }
        if material.flags[0] != has_texture {
            material.flags[0] = has_texture;
        }
    }
}

/// Shows a camera's render target on this node.
///
/// A thin convenience over [`UIMaterial::texture`]: a camera renders into a
/// texture asset like any other, so displaying one is just binding its handle.
/// Attach alongside [`UINode`] and [`UIMaterial`]; the `texture` must be the
/// same handle passed to [`Camera::render_target`].
#[derive(Component)]
pub struct UIViewport {
    pub texture: AssetHandle<Texture>,
}

/// Copies a viewport's handle into its material.
///
/// Runs in `LateUpdate`, before the layout and extract passes read the
/// material, so a viewport added this frame is bound this frame.
pub(crate) fn sync_viewport_textures(viewports: Query<(&UIViewport, &mut UIMaterial)>) {
    for (viewport, mut material) in viewports.iter() {
        if material
            .texture
            .as_ref()
            .is_none_or(|bound| bound.id() != viewport.texture.id())
        {
            material.texture = Some(viewport.texture.clone());
        }
    }
}

/// Extracts [`UIMaterial`] changes into GPU-side [`RenderUIMaterial`] bind groups.
///
/// The bind group is created via [`UIMaterial::create_bind_group`] — the same
/// macro-generated method that is used to verify bind-group layout compatibility
/// — so the layout used here is always consistent with the one used to build the
/// UI render pipeline.
pub(crate) fn extract_ui_materials(
    computed_nodes: Extracted<Query<(&UIMaterial, &RenderEntity)>>,
    device: Res<RenderDevice>,
    render_textures: Res<RenderAssets<RenderTexture>>,
    dummy_texture: Res<DummyRenderTexture>,
    ui_pipeline: Res<render::MaterialPipeline<UIMaterial>>,
    render_materials: Query<&RenderUIMaterial>,
    diagnostics: Res<UIRenderDiagnostics>,
    mut cmd: CommandQueue,
) {
    for (node_material, render_entity) in computed_nodes.iter() {
        // A texture the store has not prepared yet would bind the dummy, and the
        // signature would not change afterwards — so the node would stay bound to
        // an empty texture forever. Wait for it instead.
        if node_material
            .texture
            .as_ref()
            .is_some_and(|handle| !render_textures.contains(&handle.id()))
        {
            continue;
        }
        let signature = material_signature(node_material, &render_textures);
        if render_materials
            .get_entity(**render_entity)
            .is_some_and(|material| material.signature == signature)
        {
            continue;
        }
        let Ok(material_bind_group) = node_material.create_bind_group(
            &device,
            &render_textures,
            &dummy_texture,
            &ui_pipeline.bind_group_layout,
        ) else {
            continue;
        };
        diagnostics.record_binding_rebuild();

        cmd.insert(
            RenderUIMaterial {
                material_bind_group,
                signature,
            },
            **render_entity,
        );
    }
}

fn material_signature(
    material: &UIMaterial,
    render_textures: &RenderAssets<RenderTexture>,
) -> [u32; 19] {
    let color = material.color.to_array();
    let border = material.border_color.to_array();
    [
        color[0].to_bits(),
        color[1].to_bits(),
        color[2].to_bits(),
        color[3].to_bits(),
        border[0].to_bits(),
        border[1].to_bits(),
        border[2].to_bits(),
        border[3].to_bits(),
        material.border_params[0].to_bits(),
        material.border_params[1].to_bits(),
        material.border_params[2].to_bits(),
        material.border_params[3].to_bits(),
        material.border_width.to_bits(),
        material.corner_radius.to_bits(),
        material.flags[0].to_bits(),
        material.flags[1].to_bits(),
        material.flags[2].to_bits(),
        material.flags[3].to_bits(),
        // The bind group holds the texture view, so it must be rebuilt both
        // when the handle changes and when the texture behind the handle does —
        // a render target is reallocated whenever its camera resizes.
        material.texture.as_ref().map_or(0, |handle| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            handle.id().hash(&mut hasher);
            render_textures
                .get(&handle.id())
                .map(|texture| &texture.view)
                .hash(&mut hasher);
            hasher.finish() as u32
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_rect_accounts_for_padding() {
        let rect = UIBox {
            min: Vec2::new(10.0, 20.0),
            size: Vec2::new(100.0, 80.0),
        };
        let content = UIRect {
            top: 3.0,
            right: 7.0,
            bottom: 11.0,
            left: 5.0,
        }
        .inset_box(rect);
        assert_eq!(content.min, Vec2::new(15.0, 23.0));
        assert_eq!(content.size, Vec2::new(88.0, 66.0));
    }

    #[test]
    fn hidden_nodes_map_to_display_none() {
        let node = UINode {
            visible: false,
            ..Default::default()
        };
        assert_eq!(node.style().display, Display::None);
    }

    #[test]
    fn percentages_use_a_zero_to_hundred_scale() {
        assert_eq!(dimension(UIValue::Percent(100.0)), Dimension::percent(1.0));
        assert_eq!(dimension(UIValue::Percent(25.0)), Dimension::percent(0.25));
        // No fractional heuristic: one is one percent, not one hundred.
        assert_eq!(dimension(UIValue::Percent(1.0)), Dimension::percent(0.01));
    }

    #[test]
    fn clipping_can_be_applied_per_axis() {
        let inherited = UIBox {
            min: Vec2::ZERO,
            size: Vec2::splat(100.0),
        };
        let rect = UIBox {
            min: Vec2::new(25.0, 30.0),
            size: Vec2::new(50.0, 40.0),
        };
        let node = UINode {
            overflow_x: Overflow::Hidden,
            ..Default::default()
        };
        assert_eq!(
            node_clip(rect, inherited, &node),
            UIBox {
                min: Vec2::new(25.0, 0.0),
                size: Vec2::new(50.0, 100.0),
            }
        );
    }

    fn measurer() -> UITextMeasure {
        UITextMeasure::new(crate::text::fonts::build_font_system(
            &crate::text::fonts::UIFonts::default(),
        ))
    }

    fn label(text: &str) -> TextMeasure {
        let component = crate::text::TextComponent {
            text: text.into(),
            font_size: 14.0,
            line_height: 20.0,
            wrap: false,
            ..Default::default()
        };
        TextMeasure {
            signature: text_measure_signature(&component),
            text: component.text,
            font_size: component.font_size,
            line_height: component.line_height,
            family: component.font_family,
            weight: component.font_weight,
            italic: false,
            wrap: component.wrap,
            ellipsis: component.ellipsis,
        }
    }

    /// Lays a label out inside a row of `row_width`, returning the width it
    /// ended up with.
    fn label_width_in_row(mut measure: TextMeasure, ellipsis: bool, row_width: f32) -> f32 {
        measure.ellipsis = ellipsis;
        let mut taffy: TaffyTree<TextMeasure> = TaffyTree::new();
        let node = UINode {
            flex_grow: 1.0,
            ..Default::default()
        };
        let label = taffy
            .new_leaf_with_context(text_leaf_style(&node, &measure), measure)
            .unwrap();
        let row = taffy
            .new_with_children(
                UINode {
                    width: UIValue::Px(row_width),
                    flex_direction: FlexDirection::Row,
                    ..Default::default()
                }
                .style(),
                &[label],
            )
            .unwrap();
        let mut measurer = measurer();
        taffy
            .compute_layout_with_measure(
                row,
                Size {
                    width: AvailableSpace::Definite(row_width),
                    height: AvailableSpace::MaxContent,
                },
                |known, available, _id, context, _style| {
                    measure_node(&mut measurer, known, available, context)
                },
            )
            .unwrap();
        taffy.layout(label).unwrap().size.width
    }

    /// Two cards sharing a rail, each holding a scrolling viewport over `rows`
    /// rows. Returns the height each card ended up with.
    fn card_heights(rows: f32, content_position: Position) -> (f32, f32) {
        let mut taffy: TaffyTree<TextMeasure> = TaffyTree::new();
        let mut card = || {
            let pool = taffy
                .new_leaf(
                    UINode {
                        height: UIValue::Px(rows * 32.0),
                        flex_shrink: 0.0,
                        position: content_position,
                        inset: UIInset {
                            left: UIValue::Px(0.0),
                            right: UIValue::Px(0.0),
                            top: UIValue::Px(0.0),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                    .style(),
                )
                .unwrap();
            let view = taffy
                .new_with_children(
                    UINode {
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        ..Default::default()
                    }
                    .clipped()
                    .style(),
                    &[pool],
                )
                .unwrap();
            taffy
                .new_with_children(
                    UINode {
                        flex_grow: 1.0,
                        flex_shrink: 1.0,
                        flex_direction: FlexDirection::Column,
                        ..Default::default()
                    }
                    .style(),
                    &[view],
                )
                .unwrap()
        };
        let (first, second) = (card(), card());
        let rail = taffy
            .new_with_children(
                UINode {
                    width: UIValue::Px(238.0),
                    height: UIValue::Px(600.0),
                    flex_direction: FlexDirection::Column,
                    ..Default::default()
                }
                .style(),
                &[first, second],
            )
            .unwrap();
        let mut measurer = measurer();
        taffy
            .compute_layout_with_measure(
                rail,
                Size {
                    width: AvailableSpace::Definite(238.0),
                    height: AvailableSpace::Definite(600.0),
                },
                |known, available, _id, context, _style| {
                    measure_node(&mut measurer, known, available, context)
                },
            )
            .unwrap();
        (
            taffy.layout(first).unwrap().size.height,
            taffy.layout(second).unwrap().size.height,
        )
    }

    /// The bug this guards: the scroll content used to sit in flow, so a taller
    /// list grew its card and a scrolled one shrank it. Panels then collapsed as
    /// the viewport and its content chased each other frame after frame.
    #[test]
    fn a_scroll_viewport_does_not_take_its_size_from_its_content() {
        let (short, tall) = (
            card_heights(4.0, Position::Absolute),
            card_heights(60.0, Position::Absolute),
        );
        assert_eq!(short, (300.0, 300.0), "cards split the rail evenly");
        assert_eq!(
            tall, short,
            "how many rows a list holds must not change the card holding it"
        );

        let in_flow = card_heights(60.0, Position::Relative);
        assert_ne!(
            in_flow, short,
            "in flow the content does drive the card, which is what absolute positioning avoids"
        );
    }

    /// Reproduces the panel structure: a clipped view, a column pool, a row of
    /// [fixed toggle, fixed glyph, growing label].
    #[test]
    fn a_long_label_does_not_widen_the_row_inside_a_scroll_pool() {
        let mut taffy: TaffyTree<TextMeasure> = TaffyTree::new();
        let mut measure = label("content/UAL1/scene.gasset");
        measure.ellipsis = true;
        let label_node = UINode {
            flex_grow: 1.0,
            padding: UIRect::axes(6.0, 8.0),
            ..Default::default()
        };
        let label_id = taffy
            .new_leaf_with_context(text_leaf_style(&label_node, &measure), measure)
            .unwrap();
        let fixed = |taffy: &mut TaffyTree<TextMeasure>, width: f32| {
            taffy
                .new_leaf(
                    UINode {
                        width: UIValue::Px(width),
                        // Icon columns hold their width; only the label gives.
                        flex_shrink: 0.0,
                        ..Default::default()
                    }
                    .style(),
                )
                .unwrap()
        };
        let toggle = fixed(&mut taffy, 30.0);
        let glyph = fixed(&mut taffy, 20.0);
        let row = taffy
            .new_with_children(
                UINode {
                    height: UIValue::Px(32.0),
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Row,
                    ..Default::default()
                }
                .style(),
                &[toggle, glyph, label_id],
            )
            .unwrap();
        let pool = taffy
            .new_with_children(
                UINode {
                    flex_direction: FlexDirection::Column,
                    flex_shrink: 0.0,
                    ..Default::default()
                }
                .style(),
                &[row],
            )
            .unwrap();
        let view = taffy
            .new_with_children(
                UINode {
                    width: UIValue::Px(197.0),
                    height: UIValue::Px(300.0),
                    flex_direction: FlexDirection::Column,
                    ..Default::default()
                }
                .clipped()
                .style(),
                &[pool],
            )
            .unwrap();

        let mut measurer = measurer();
        taffy
            .compute_layout_with_measure(
                view,
                Size {
                    width: AvailableSpace::Definite(197.0),
                    height: AvailableSpace::Definite(300.0),
                },
                |known, available, _id, context, _style| {
                    measure_node(&mut measurer, known, available, context)
                },
            )
            .unwrap();

        assert_eq!(taffy.layout(pool).unwrap().size.width, 197.0, "pool");
        assert_eq!(taffy.layout(row).unwrap().size.width, 197.0, "row");
        assert_eq!(
            taffy.layout(label_id).unwrap().size.width,
            197.0 - 50.0,
            "label"
        );
    }

    /// The widths a row of grown text fields ends up with, one per `texts`.
    ///
    /// Reproduces an inspector property row: a fixed label column followed by
    /// fields that share what is left of the row.
    fn field_widths(texts: &[&str], basis: UIValue) -> Vec<f32> {
        let mut taffy: TaffyTree<TextMeasure> = TaffyTree::new();
        let fields: Vec<_> = texts
            .iter()
            .map(|text| {
                let measure = label(text);
                let node = UINode {
                    flex_grow: 1.0,
                    width: basis,
                    height: UIValue::Px(28.0),
                    padding: UIRect::axes(0.0, 4.0),
                    ..Default::default()
                }
                .clipped();
                taffy
                    .new_leaf_with_context(text_leaf_style(&node, &measure), measure)
                    .unwrap()
            })
            .collect();
        let mut children = vec![
            taffy
                .new_leaf(
                    UINode {
                        width: UIValue::Px(72.0),
                        flex_shrink: 0.0,
                        ..Default::default()
                    }
                    .style(),
                )
                .unwrap(),
        ];
        children.extend(fields.iter().copied());
        let row = taffy
            .new_with_children(
                UINode {
                    width: UIValue::Px(260.0),
                    flex_direction: FlexDirection::Row,
                    gap: Vec2::new(4.0, 0.0),
                    ..Default::default()
                }
                .style(),
                &children,
            )
            .unwrap();
        let mut measurer = measurer();
        taffy
            .compute_layout_with_measure(
                row,
                Size {
                    width: AvailableSpace::Definite(260.0),
                    height: AvailableSpace::MaxContent,
                },
                |known, available, _id, context, _style| {
                    measure_node(&mut measurer, known, available, context)
                },
            )
            .unwrap();
        fields
            .into_iter()
            .map(|field| taffy.layout(field).unwrap().size.width)
            .collect()
    }

    /// The bug this guards: a text field with an automatic flex basis takes its
    /// width from its own content, so typing into one of a row of fields — the
    /// caret counts as content too — widened it and squeezed its neighbours.
    /// Nothing a field holds may move its edges.
    #[test]
    fn a_row_of_grown_fields_keeps_equal_widths_whatever_they_contain() {
        let uneven = ["0.000", "-1284.375", "7.5"];
        let widths = field_widths(&uneven, UIValue::Px(0.0));
        assert_eq!(
            widths,
            field_widths(&["0.000"; 3], UIValue::Px(0.0)),
            "a field's width must not depend on its text"
        );
        // Free space that does not divide evenly leaves a pixel somewhere; what
        // matters is that it is a pixel of rounding and not a word of text.
        assert!(
            widths
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() <= 1.0),
            "fields sharing a row must be even: {widths:?}"
        );

        let automatic = field_widths(&uneven, UIValue::Auto);
        assert!(
            automatic.windows(2).any(|pair| pair[0] != pair[1]),
            "an automatic basis is what let content drive the width: {automatic:?}"
        );
    }

    /// Uniforms live in the bind group, so a slot the signature ignored would
    /// leave a rotated shape drawn at its old angle.
    #[test]
    fn the_rotation_slot_is_part_of_the_material_signature() {
        use crate::material::UIMaterial;
        let mut material = UIMaterial::flat(color::Color::WHITE);
        let textures = RenderAssets::<RenderTexture>::new();
        let before = material_signature(&material, &textures);

        material.flags[1] = std::f32::consts::FRAC_PI_4;
        assert_ne!(before, material_signature(&material, &textures));
    }

    #[test]
    fn an_ellipsising_label_stays_inside_the_row_that_holds_it() {
        let text = "a very long entity name that no narrow panel could ever show in full";
        let natural = label_width_in_row(label(text), false, 120.0);
        assert!(
            natural > 120.0,
            "without an ellipsis the label keeps its full width ({natural}) and overflows"
        );
        assert_eq!(
            label_width_in_row(label(text), true, 120.0),
            120.0,
            "an ellipsising label shrinks to the row instead of pushing past it"
        );
    }

    #[test]
    fn an_explicit_size_is_never_second_guessed() {
        let mut measurer = measurer();
        let size = measure_node(
            &mut measurer,
            Size {
                width: Some(120.0),
                height: Some(30.0),
            },
            Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            Some(&mut label("anything at all")),
        );
        assert_eq!(size.width, 120.0);
        assert_eq!(size.height, 30.0);
    }

    #[test]
    fn a_node_without_text_measures_to_nothing() {
        let mut measurer = measurer();
        let size = measure_node(
            &mut measurer,
            Size::NONE,
            Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            None,
        );
        assert_eq!(size, Size::ZERO);
    }

    #[test]
    fn longer_text_measures_wider() {
        let mut measurer = measurer();
        let short = measurer.measure(&label("Warren"), None);
        let long = measurer.measure(&label("Warren and Curiosities"), None);
        assert!(
            long.x > short.x,
            "more text must claim more width: {} vs {}",
            long.x,
            short.x
        );
        assert!(short.x > 0.0, "text must have some width at all");
    }

    #[test]
    fn each_line_adds_its_line_height() {
        let mut measurer = measurer();
        let one = measurer.measure(&label("one"), None);
        let three = measurer.measure(&label("one\ntwo\nthree"), None);
        assert_eq!(one.y, 20.0);
        assert_eq!(three.y, 60.0, "height is line count times line height");
    }

    #[test]
    fn the_same_query_is_only_shaped_once() {
        let mut measurer = measurer();
        let first = measurer.measure(&label("Curiosities"), None);
        let cached = measurer.measure(&label("Curiosities"), None);
        assert_eq!(first, cached);
        assert_eq!(measurer.cache.len(), 1, "a repeat must hit the cache");
    }

    #[test]
    fn a_changed_label_invalidates_its_measurement() {
        let before = text_measure_signature(&crate::text::TextComponent {
            text: "Warren".into(),
            ..Default::default()
        });
        let after = text_measure_signature(&crate::text::TextComponent {
            text: "Curiosities".into(),
            ..Default::default()
        });
        assert_ne!(
            before, after,
            "the layout pass keys on this to know it must re-measure"
        );
    }

    #[test]
    fn explicit_z_index_dominates_tree_sequence() {
        assert!(paint_order(1, 0) > paint_order(0, i32::MAX as i64));
    }
}

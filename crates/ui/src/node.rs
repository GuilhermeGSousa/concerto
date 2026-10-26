#![allow(clippy::too_many_arguments)]

use std::collections::{HashMap, HashSet};

use concerto_app::extractor::Extracted;
use concerto_ecs::{
    World,
    command::CommandQueue,
    component::Component,
    entity::{
        Entity,
        hierarchy::{ChildOf, Children},
    },
    query::{
        Query,
        change_detection::DetectChanges,
        filter::{Changed, Or, With, Without},
    },
    resource::{Res, ResMut, Resource},
    signal::{
        On,
        lifecycle::{Add, Remove, Replace},
    },
};
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_render::{
    assets::{material::AsBindGroup, texture::Texture},
    components::render_entity::{RenderEntity, SyncWithRenderWorld},
    device::RenderDevice,
    draw_target::SharedTexture,
    render_asset::{
        RenderAssets,
        render_texture::{DummyRenderTexture, RenderTexture},
    },
};
use concerto_window::plugin::Window;
use glam::Vec2;
use log::warn;
pub use taffy::{AlignContent, AlignItems, FlexDirection, Overflow, Position};
use taffy::{
    AvailableSpace, Dimension, Display, LengthPercentage, LengthPercentageAuto, NodeId, Point,
    Rect, Size, Style, TaffyTree,
};
use wgpu::{Buffer, util::DeviceExt};

use crate::{
    anchor::{UIAnchoredPanel, UIPanelStack},
    items::UIWidgetPart,
    material::UIMaterial,
    resources::UIRenderDiagnostics,
    scroll::UIScrollArea,
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

impl From<f32> for UIRect {
    /// The same value on all four sides.
    fn from(value: f32) -> Self {
        Self::all(value)
    }
}

/// Offsets of a positioned node from its parent's edges.
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

    /// Whether the box covers any area, on both axes.
    pub fn has_area(self) -> bool {
        self.size.x > 0.0 && self.size.y > 0.0
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
    /// Clips descendants to this node's box on both axes.
    pub fn clipped(mut self) -> Self {
        self.overflow_x = Overflow::Hidden;
        self.overflow_y = Overflow::Hidden;
        self
    }

    pub fn with_width(mut self, width: UIValue) -> Self {
        self.width = width;
        self
    }

    pub fn with_height(mut self, height: UIValue) -> Self {
        self.height = height;
        self
    }

    pub fn with_size(self, width: UIValue, height: UIValue) -> Self {
        self.with_width(width).with_height(height)
    }

    pub fn with_min_width(mut self, min_width: UIValue) -> Self {
        self.min_width = min_width;
        self
    }

    pub fn with_min_height(mut self, min_height: UIValue) -> Self {
        self.min_height = min_height;
        self
    }

    pub fn with_min_size(self, min_width: UIValue, min_height: UIValue) -> Self {
        self.with_min_width(min_width).with_min_height(min_height)
    }

    pub fn with_max_width(mut self, max_width: UIValue) -> Self {
        self.max_width = max_width;
        self
    }

    pub fn with_max_height(mut self, max_height: UIValue) -> Self {
        self.max_height = max_height;
        self
    }

    pub fn with_max_size(self, max_width: UIValue, max_height: UIValue) -> Self {
        self.with_max_width(max_width).with_max_height(max_height)
    }

    pub fn with_flex_direction(mut self, flex_direction: FlexDirection) -> Self {
        self.flex_direction = flex_direction;
        self
    }

    pub fn with_flex_grow(mut self, flex_grow: f32) -> Self {
        self.flex_grow = flex_grow;
        self
    }

    pub fn with_flex_shrink(mut self, flex_shrink: f32) -> Self {
        self.flex_shrink = flex_shrink;
        self
    }

    pub fn with_gap(mut self, gap: Vec2) -> Self {
        self.gap = gap;
        self
    }

    pub fn with_align_items(mut self, align_items: AlignItems) -> Self {
        self.align_items = Some(align_items);
        self
    }

    pub fn with_align_self(mut self, align_self: AlignItems) -> Self {
        self.align_self = Some(align_self);
        self
    }

    pub fn with_justify_content(mut self, justify_content: AlignContent) -> Self {
        self.justify_content = Some(justify_content);
        self
    }

    pub fn with_padding(mut self, padding: UIRect) -> Self {
        self.padding = padding;
        self
    }

    pub fn with_margin(mut self, margin: UIRect) -> Self {
        self.margin = margin;
        self
    }

    pub fn with_position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }

    pub fn with_inset(mut self, inset: UIInset) -> Self {
        self.inset = inset;
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn with_overflow_x(mut self, overflow_x: Overflow) -> Self {
        self.overflow_x = overflow_x;
        self
    }

    pub fn with_overflow_y(mut self, overflow_y: Overflow) -> Self {
        self.overflow_y = overflow_y;
        self
    }

    pub fn with_z_index(mut self, z_index: i32) -> Self {
        self.z_index = z_index;
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
#[derive(Component, Clone, PartialEq)]
pub struct UILayout {
    pub rect: UIBox,
    pub content_rect: UIBox,
    pub clip_rect: UIBox,
    pub paint_order: i64,
}

/// Set by lists and trees, never by callers, on rows they leave out of layout; a culled node and its subtree are not laid out, drawn or hit.
#[derive(Component)]
pub struct UICulled;

#[derive(Component)]
pub(crate) struct UIIntrinsicHeight(pub(crate) f32);

pub(crate) fn blank_culled_layouts(
    on: On<Add<UICulled>>,
    children: Query<&Children>,
    layouts: Query<&mut UILayout>,
) {
    let mut stack = vec![on.signal().entity];
    while let Some(entity) = stack.pop() {
        if let Some(mut layout) = layouts.get_entity(entity) {
            layout.clip_rect = UIBox {
                min: layout.rect.min,
                size: Vec2::ZERO,
            };
        }
        if let Some(children) = children.get_entity(entity) {
            stack.extend(children.iter().copied());
        }
    }
}

/// Layout instrumentation consumed by the showcase and profilers.
#[derive(Resource, Default)]
pub struct UILayoutDiagnostics {
    pub layout_passes: u64,
    /// Taffy nodes whose style, measure context or children were set by the last layout run that had anything to look at.
    pub nodes_updated: u64,
}

pub(crate) enum LeafMeasure {
    Text(TextMeasure),
    Extent(f32),
}

/// What the layout pass needs to know to measure one text node.
pub(crate) struct TextMeasure {
    text: String,
    font_size: f32,
    line_height: f32,
    family: crate::text::FontFamily,
    weight: u16,
    italic: bool,
    wrap: bool,
}

/// Shapes text so the layout pass can size nodes to their content.
#[derive(Resource)]
pub(crate) struct UITextMeasure {
    font_system: glyphon::FontSystem,
    #[cfg(test)]
    measured: Vec<String>,
}

impl UITextMeasure {
    pub(crate) fn new(font_system: glyphon::FontSystem) -> Self {
        Self {
            font_system,
            #[cfg(test)]
            measured: Vec::new(),
        }
    }

    fn measure(&mut self, measure: &TextMeasure, width: Option<f32>) -> Vec2 {
        #[cfg(test)]
        self.measured.push(measure.text.clone());

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
        Vec2::new(widest.ceil(), lines * measure.line_height)
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
    taffy: TaffyTree<LeafMeasure>,
    nodes: HashMap<Entity, TaffyNode>,
    stale: HashSet<Entity>,
    roots: Vec<Entity>,
    logical_size: Vec2,
    #[cfg(test)]
    visited: u64,
}

// SAFETY: Taffy stores a pointer in a length only for `calc()`, which no `UINode` style produces, so the tree shares nothing between threads.
unsafe impl Send for UILayoutEngine {}
// SAFETY: as above, every length in the tree is plain packed bits.
unsafe impl Sync for UILayoutEngine {}

struct TaffyNode {
    id: NodeId,
    measure_key: Option<u64>,
    z_index: i32,
}

impl Default for UILayoutEngine {
    fn default() -> Self {
        Self {
            taffy: TaffyTree::new(),
            nodes: HashMap::new(),
            stale: HashSet::new(),
            roots: Vec::new(),
            logical_size: Vec2::ZERO,
            #[cfg(test)]
            visited: 0,
        }
    }
}

impl UILayoutEngine {
    fn forget(&mut self, entity: Entity) {
        let Some(node) = self.nodes.remove(&entity) else {
            return;
        };
        if let Some(parent) = self.taffy.parent(node.id) {
            let _ = self.taffy.remove_child(parent, node.id);
        }
        let _ = self.taffy.remove(node.id);
    }

    fn mark_dirty_upwards(&mut self, id: NodeId) {
        let mut node = Some(id);
        while let Some(current) = node {
            let _ = self.taffy.mark_dirty(current);
            node = self.taffy.parent(current);
        }
    }

    fn revisit_with_parent(&mut self, entity: Entity, parents: &Query<&ChildOf>) {
        self.stale.insert(entity);
        if let Some(parent) = parents.get_entity(entity) {
            self.stale.insert(parent.parent());
        }
    }
}

/// Registers the listeners that keep the retained layout tree in step with removals, re-parenting and culling.
pub(crate) fn install_layout_listeners(world: &mut World) {
    world.add_listener(forget_removed_node);
    world.add_listener(revisit_parent_on_detach);
    world.add_listener(revisit_parent_on_reparent);
    world.add_listener(revisit_on_text_removed);
    world.add_listener(revisit_on_extent_removed);
    world.add_listener(revisit_on_cull);
    world.add_listener(revisit_on_uncull);
}

fn forget_removed_node(on: On<Remove<UINode>>, engine: Option<ResMut<UILayoutEngine>>) {
    if let Some(mut engine) = engine {
        engine.forget(on.signal().entity);
    }
}

fn revisit_parent_on_detach(
    on: On<Remove<ChildOf>>,
    parents: Query<&ChildOf>,
    engine: Option<ResMut<UILayoutEngine>>,
) {
    revisit_parent(on.signal().entity, &parents, engine);
}

fn revisit_parent_on_reparent(
    on: On<Replace<ChildOf>>,
    parents: Query<&ChildOf>,
    engine: Option<ResMut<UILayoutEngine>>,
) {
    revisit_parent(on.signal().entity, &parents, engine);
}

fn revisit_parent(
    entity: Entity,
    parents: &Query<&ChildOf>,
    engine: Option<ResMut<UILayoutEngine>>,
) {
    if let (Some(mut engine), Some(parent)) = (engine, parents.get_entity(entity)) {
        engine.stale.insert(parent.parent());
    }
}

fn revisit_on_text_removed(
    on: On<Remove<crate::text::UIText>>,
    engine: Option<ResMut<UILayoutEngine>>,
) {
    if let Some(mut engine) = engine {
        engine.stale.insert(on.signal().entity);
    }
}

fn revisit_on_extent_removed(
    on: On<Remove<UIIntrinsicHeight>>,
    engine: Option<ResMut<UILayoutEngine>>,
) {
    if let Some(mut engine) = engine {
        engine.stale.insert(on.signal().entity);
    }
}

fn revisit_on_cull(
    on: On<Add<UICulled>>,
    parents: Query<&ChildOf>,
    engine: Option<ResMut<UILayoutEngine>>,
) {
    if let Some(mut engine) = engine {
        engine.revisit_with_parent(on.signal().entity, &parents);
    }
}

fn revisit_on_uncull(
    on: On<Remove<UICulled>>,
    parents: Query<&ChildOf>,
    engine: Option<ResMut<UILayoutEngine>>,
) {
    let Some(mut engine) = engine else {
        return;
    };
    let entity = on.signal().entity;
    engine.revisit_with_parent(entity, &parents);
    if let Some(id) = engine.nodes.get(&entity).map(|node| node.id) {
        engine.mark_dirty_upwards(id);
    }
}

impl UILayout {
    pub fn location(&self) -> Vec2 {
        self.rect.min
    }

    pub fn size(&self) -> Vec2 {
        self.rect.size
    }

    /// Whether any of the node may be seen; false for a culled node, whose clip is empty.
    pub fn is_shown(&self) -> bool {
        self.clip_rect.has_area()
    }
}

#[derive(Component)]
pub(crate) struct RenderUINode {
    pub(crate) index_buffer: Buffer,
    pub(crate) index_count: u32,
    pub(crate) vertex_buffer: Buffer,
    source_rect: UIBox,
    clip_rect: UIBox,
    surface: (u32, u32, u64),
    pub(crate) z_index: i64,
}

#[derive(Component)]
pub(crate) struct RenderUIMaterial {
    pub(crate) material_bind_group: wgpu::BindGroup,
    signature: [u32; 19],
}

pub(crate) fn compute_ui_nodes(
    (nodes, roots, changed, changed_children): (
        Query<(Entity, &UINode, Option<&Children>)>,
        Query<Entity, (With<UINode>, Without<ChildOf>, Without<UICulled>)>,
        Query<
            Entity,
            (
                With<UINode>,
                Or<(
                    Changed<UINode>,
                    Changed<crate::text::UIText>,
                    Changed<UIIntrinsicHeight>,
                )>,
            ),
        >,
        Query<Entity, Changed<Children>>,
    ),
    (culled, texts, extents, parents): (
        Query<&UICulled>,
        Query<&crate::text::UIText>,
        Query<&UIIntrinsicHeight>,
        Query<&ChildOf>,
    ),
    (scroll_areas, widget_parts, scrolled): (
        Query<&UIScrollArea>,
        Query<&UIWidgetPart>,
        Query<Entity, Changed<UIScrollArea>>,
    ),
    layouts: Query<&mut UILayout>,
    panels: Query<(Entity, &mut UIAnchoredPanel)>,
    panel_stack: Res<UIPanelStack>,
    window: Res<Window>,
    mut engine: ResMut<UILayoutEngine>,
    mut measurer: ResMut<UITextMeasure>,
    mut diagnostics: ResMut<UILayoutDiagnostics>,
    mut cmd: CommandQueue,
) {
    let engine = &mut *engine;
    let mut stale = std::mem::take(&mut engine.stale);
    let mut examined = false;
    let mut updated = HashSet::new();
    let mut created = Vec::new();
    let mut layers_changed = false;

    for entity in changed.iter().chain(stale.iter().copied()) {
        examined = true;
        let Some((_, node, _)) = nodes.get_entity(entity) else {
            continue;
        };
        let extent = extents.get_entity(entity).map(|extent| extent.0);
        let text = match extent {
            Some(_) => None,
            None => texts.get_entity(entity),
        };
        let style = match text {
            Some(text) => leaf_style(node, text.ellipsis, text.wrap),
            None => node.style(),
        };
        let measure_key = match (extent, text) {
            (Some(extent), _) => Some(extent_measure_signature(extent)),
            (None, Some(text)) => Some(text_measure_signature(text)),
            (None, None) => None,
        };
        let Some(existing) = engine.nodes.get_mut(&entity) else {
            let id = match leaf_context(text, extent) {
                Some(context) => engine.taffy.new_leaf_with_context(style, context),
                None => engine.taffy.new_leaf(style),
            };
            let Ok(id) = id else {
                warn!("Error adding UI node {entity:?} to the layout tree");
                continue;
            };
            engine.nodes.insert(
                entity,
                TaffyNode {
                    id,
                    measure_key,
                    z_index: node.z_index,
                },
            );
            created.push(entity);
            updated.insert(entity);
            continue;
        };
        if engine
            .taffy
            .style(existing.id)
            .is_ok_and(|current| *current != style)
        {
            let _ = engine.taffy.set_style(existing.id, style);
            updated.insert(entity);
        }
        if existing.measure_key != measure_key {
            let _ = engine
                .taffy
                .set_node_context(existing.id, leaf_context(text, extent));
            existing.measure_key = measure_key;
            updated.insert(entity);
        }
        if existing.z_index != node.z_index {
            existing.z_index = node.z_index;
            layers_changed = true;
        }
        if updated.contains(&entity) {
            let id = existing.id;
            engine.mark_dirty_upwards(id);
        }
    }
    for entity in created {
        stale.insert(entity);
        if let Some(parent) = parents.get_entity(entity) {
            stale.insert(parent.parent());
        }
    }

    let mut shown = Vec::new();
    for entity in changed_children.iter().chain(stale.iter().copied()) {
        examined = true;
        let Some(id) = engine.nodes.get(&entity).map(|node| node.id) else {
            continue;
        };
        shown.clear();
        if let Some((_, _, Some(children))) = nodes.get_entity(entity) {
            shown.extend(
                children
                    .iter()
                    .filter(|child| !culled.contains_entity(**child))
                    .filter_map(|child| engine.nodes.get(child).map(|node| node.id)),
            );
        }
        if engine
            .taffy
            .children(id)
            .is_ok_and(|current| current != shown)
        {
            let _ = engine.taffy.set_children(id, &shown);
            engine.mark_dirty_upwards(id);
            updated.insert(entity);
        }
    }

    let logical_size = window.logical_size();
    let resized = engine.logical_size != logical_size;
    engine.logical_size = logical_size;
    let roots_changed = !roots.iter().eq(engine.roots.iter().copied());
    let previous_roots = if roots_changed {
        std::mem::replace(&mut engine.roots, roots.iter().collect())
    } else {
        Vec::new()
    };
    let window_size = Size {
        width: AvailableSpace::Definite(logical_size.x),
        height: AvailableSpace::Definite(logical_size.y),
    };
    let mut computed = false;
    for root in &engine.roots {
        let Some(id) = engine.nodes.get(root).map(|node| node.id) else {
            continue;
        };
        let new_root = roots_changed && !previous_roots.contains(root);
        if !resized && !new_root && !engine.taffy.dirty(id).unwrap_or(true) {
            continue;
        }
        let measurer = &mut *measurer;
        let result = engine.taffy.compute_layout_with_measure(
            id,
            window_size,
            |known, available, _id, context, _style| {
                measure_node(measurer, known, available, context)
            },
        );
        if let Err(error) = result {
            warn!("Error computing UI layout: {}", error);
        }
        computed = true;
    }
    if computed {
        diagnostics.layout_passes += 1;
    }

    let panels_changed = panel_stack.is_changed()
        || panel_stack.open().iter().any(|entity| {
            panels
                .get_entity(*entity)
                .is_some_and(|(_, panel)| panel.is_changed())
        });
    let scrolling = scrolled.iter().next().is_some();
    let walk = computed || roots_changed || layers_changed || panels_changed || scrolling;
    if examined || walk {
        diagnostics.nodes_updated = updated.len() as u64;
    }
    if !walk {
        return;
    }

    let window_clip = UIBox {
        min: Vec2::ZERO,
        size: logical_size,
    };
    let mut sequence = 0_i64;
    let mut anchor_rects: HashMap<Entity, UIBox> = HashMap::new();
    for &entity in &engine.roots {
        if panel_stack.open().contains(&entity) {
            continue;
        }
        let Some((_, root_node, children)) = nodes.get_entity(entity) else {
            continue;
        };
        let Some(layout) = engine
            .nodes
            .get(&entity)
            .and_then(|node| engine.taffy.layout(node.id).ok())
        else {
            continue;
        };

        let abs_pos = Vec2::new(layout.location.x, layout.location.y);
        let size = Vec2::new(layout.size.width, layout.size.height);

        let rect = UIBox { min: abs_pos, size };
        let clip_rect = node_clip(rect, window_clip, root_node);
        write_layout(
            &layouts,
            &mut cmd,
            entity,
            UILayout {
                rect,
                content_rect: root_node.padding.inset_box(rect),
                clip_rect,
                paint_order: paint_order(root_node.z_index, sequence),
            },
        );
        if root_node.visible {
            anchor_rects.insert(entity, rect);
        }
        sequence += 1;

        if let Some(children) = children {
            write_absolute_positions(
                engine,
                (abs_pos, scroll_offset(&scroll_areas, entity)),
                clip_rect,
                root_node.z_index,
                root_node.visible,
                children,
                (&nodes, &culled, &scroll_areas, &widget_parts),
                &layouts,
                &mut cmd,
                &mut sequence,
                &mut anchor_rects,
            );
        }
    }

    for panel_entity in panel_stack.open() {
        let Some((_, mut panel)) = panels.get_entity(*panel_entity) else {
            continue;
        };
        if !engine.roots.contains(panel_entity) {
            continue;
        }
        let Some((entity, root_node, children)) = nodes.get_entity(*panel_entity) else {
            continue;
        };
        let Some(layout) = engine
            .nodes
            .get(&entity)
            .and_then(|node| engine.taffy.layout(node.id).ok())
        else {
            continue;
        };
        let size = Vec2::new(layout.size.width, layout.size.height);

        let Some(anchor) = panel.target.anchor_box(&anchor_rects) else {
            continue;
        };

        let placement = crate::anchor::place(anchor, size, logical_size, &panel);
        if panel.resolved_side != placement.side {
            panel.resolved_side = placement.side;
        }
        let rect = UIBox {
            min: placement.origin,
            size,
        };
        let clip_rect = node_clip(rect, window_clip, root_node);
        let layer = root_node.z_index;
        write_layout(
            &layouts,
            &mut cmd,
            entity,
            UILayout {
                rect,
                content_rect: root_node.padding.inset_box(rect),
                clip_rect,
                paint_order: paint_order(layer, sequence),
            },
        );
        anchor_rects.insert(entity, rect);
        sequence += 1;

        if let Some(children) = children {
            write_absolute_positions(
                engine,
                (placement.origin, scroll_offset(&scroll_areas, entity)),
                clip_rect,
                layer,
                root_node.visible,
                children,
                (&nodes, &culled, &scroll_areas, &widget_parts),
                &layouts,
                &mut cmd,
                &mut sequence,
                &mut anchor_rects,
            );
        }
    }

    #[cfg(test)]
    {
        engine.visited += sequence as u64;
    }
}

fn measure_node(
    measurer: &mut UITextMeasure,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    context: Option<&mut LeafMeasure>,
) -> Size<f32> {
    let Some(measure) = context else {
        return Size::ZERO;
    };
    if let (Some(width), Some(height)) = (known.width, known.height) {
        return Size { width, height };
    }
    let measure = match measure {
        LeafMeasure::Text(measure) => measure,
        LeafMeasure::Extent(extent) => {
            let natural = match available.height {
                AvailableSpace::MinContent => 0.0,
                _ => *extent,
            };
            return Size {
                width: known.width.unwrap_or(0.0),
                height: known.height.unwrap_or(natural),
            };
        }
    };
    let constraint = match (known.width, available.width) {
        (Some(width), _) => Some(width),
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

fn text_measure(text: &crate::text::UIText) -> TextMeasure {
    TextMeasure {
        text: text.text.clone(),
        font_size: text.font_size,
        line_height: text.line_height,
        family: text.font_family.clone(),
        weight: text.font_weight,
        italic: text.font_style == crate::text::FontStyle::Italic,
        wrap: text.wrap,
    }
}

fn leaf_context(text: Option<&crate::text::UIText>, extent: Option<f32>) -> Option<LeafMeasure> {
    match (extent, text) {
        (Some(extent), _) => Some(LeafMeasure::Extent(extent)),
        (None, Some(text)) => Some(LeafMeasure::Text(text_measure(text))),
        (None, None) => None,
    }
}

fn text_measure_signature(text: &crate::text::UIText) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    0_u8.hash(&mut hasher);
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

fn extent_measure_signature(extent: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    1_u8.hash(&mut hasher);
    extent.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn leaf_style(node: &UINode, ellipsis: bool, wrap: bool) -> Style {
    let mut style = node.style();
    if ellipsis && !wrap && matches!(node.min_width, UIValue::Auto) {
        style.min_size.width = Dimension::length(0.0);
    }
    style
}

fn scroll_offset(areas: &Query<&UIScrollArea>, entity: Entity) -> f32 {
    areas.get_entity(entity).map_or(0.0, |area| area.offset)
}

fn write_absolute_positions(
    engine: &UILayoutEngine,
    (parent_origin, parent_scroll): (Vec2, f32),
    parent_clip: UIBox,
    parent_layer: i32,
    parent_visible: bool,
    children: &Children,
    (ui_nodes, culled, scroll_areas, widget_parts): (
        &Query<(Entity, &UINode, Option<&Children>)>,
        &Query<&UICulled>,
        &Query<&UIScrollArea>,
        &Query<&UIWidgetPart>,
    ),
    layouts: &Query<&mut UILayout>,
    cmd: &mut CommandQueue,
    sequence: &mut i64,
    anchor_rects: &mut HashMap<Entity, UIBox>,
) {
    let mut stack = children
        .iter()
        .copied()
        .map(|entity| {
            (
                parent_origin,
                parent_scroll,
                parent_clip,
                parent_layer,
                parent_visible,
                entity,
            )
        })
        .collect::<Vec<_>>();
    stack.reverse();
    while let Some((
        origin,
        inherited_scroll,
        inherited_clip,
        inherited_layer,
        inherited_visible,
        child_entity,
    )) = stack.pop()
    {
        if culled.contains_entity(child_entity) {
            continue;
        }
        let Some(node_id) = engine.nodes.get(&child_entity).map(|node| node.id) else {
            continue;
        };
        let Ok(layout) = engine.taffy.layout(node_id) else {
            continue;
        };

        let scroll = if widget_parts.contains_entity(child_entity) {
            0.0
        } else {
            inherited_scroll
        };
        let abs_pos = origin + Vec2::new(layout.location.x, layout.location.y - scroll);
        let size = Vec2::new(layout.size.width, layout.size.height);
        let Some((_, node, grand_children)) = ui_nodes.get_entity(child_entity) else {
            continue;
        };
        let rect = UIBox { min: abs_pos, size };
        let clip_rect = node_clip(rect, inherited_clip, node);
        let layer = effective_layer(inherited_layer, node.z_index);

        write_layout(
            layouts,
            cmd,
            child_entity,
            UILayout {
                rect,
                content_rect: node.padding.inset_box(rect),
                clip_rect,
                paint_order: paint_order(layer, *sequence),
            },
        );
        let visible = inherited_visible && node.visible;
        if visible {
            anchor_rects.insert(child_entity, rect);
        }
        *sequence += 1;

        if let Some(grand_children) = grand_children {
            let own_scroll = scroll_offset(scroll_areas, child_entity);
            let mut descendants = grand_children
                .iter()
                .copied()
                .map(|entity| (abs_pos, own_scroll, clip_rect, layer, visible, entity))
                .collect::<Vec<_>>();
            descendants.reverse();
            stack.extend(descendants);
        }
    }
}

fn write_layout(
    layouts: &Query<&mut UILayout>,
    cmd: &mut CommandQueue,
    entity: Entity,
    layout: UILayout,
) {
    match layouts.get_entity(entity) {
        Some(mut current) => {
            current.set_if_neq(layout);
        }
        None => cmd.insert((layout, SyncWithRenderWorld), entity),
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

fn effective_layer(inherited: i32, own: i32) -> i32 {
    inherited.max(own)
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

    let to_ndc =
        |px: f32, py: f32| -> [f32; 2] { [(px / win_w) * 2.0 - 1.0, 1.0 - (py / win_h) * 2.0] };

    for (computed_node, render_entity) in computed_nodes.iter() {
        let draw_rect = computed_node.rect.intersection(computed_node.clip_rect);
        if !computed_node.is_shown() || !draw_rect.has_area() {
            if render_ui_nodes.contains_entity(**render_entity) {
                cmd.remove::<RenderUINode>(**render_entity);
            }
            continue;
        }
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
            },
            UIVertex {
                pos_coords: to_ndc(x, y + h),
                uv: [relative_min.x, relative_max.y],
            },
            UIVertex {
                pos_coords: to_ndc(x + w, y + h),
                uv: relative_max.into(),
            },
            UIVertex {
                pos_coords: to_ndc(x + w, y),
                uv: [relative_max.x, relative_min.y],
            },
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
#[derive(Component)]
pub struct UIViewport {
    pub texture: AssetHandle<Texture>,
}

/// Copies a viewport's handle into its material.
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

/// Shows the picture another `App` draws into a [`SharedTexture`].
#[derive(Component)]
pub struct UISharedViewport {
    pub picture: Option<SharedTexture>,
    id: AssetId,
}

impl UISharedViewport {
    pub fn new() -> Self {
        Self {
            picture: None,
            id: AssetId::new(),
        }
    }
}

impl Default for UISharedViewport {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn sync_shared_viewport_textures(
    viewports: Query<(&UISharedViewport, &mut UIMaterial)>,
) {
    for (viewport, mut material) in viewports.iter() {
        let drawn = viewport
            .picture
            .as_ref()
            .is_some_and(|picture| picture.get().is_some());
        let bound = material
            .texture
            .as_ref()
            .is_some_and(|handle| handle.id() == viewport.id);
        if drawn && !bound {
            material.texture = Some(AssetHandle::weak(viewport.id));
        } else if !drawn && material.texture.is_some() {
            material.texture = None;
        }
    }
}

pub(crate) fn extract_shared_viewports(
    viewports: Extracted<Query<&UISharedViewport>>,
    mut render_textures: ResMut<RenderAssets<RenderTexture>>,
) {
    for viewport in viewports.iter() {
        match viewport.picture.as_ref().and_then(SharedTexture::get) {
            Some(texture) => render_textures.insert(viewport.id, texture),
            None => {
                render_textures.remove(&viewport.id);
            }
        }
    }
}

/// Extracts [`UIMaterial`] changes into GPU-side [`RenderUIMaterial`] bind groups.
pub(crate) fn extract_ui_materials(
    computed_nodes: Extracted<Query<(&UIMaterial, &RenderEntity)>>,
    device: Res<RenderDevice>,
    render_textures: Res<RenderAssets<RenderTexture>>,
    dummy_texture: Res<DummyRenderTexture>,
    ui_pipeline: Res<concerto_render::MaterialPipeline<UIMaterial>>,
    render_materials: Query<&RenderUIMaterial>,
    diagnostics: Res<UIRenderDiagnostics>,
    mut cmd: CommandQueue,
) {
    for (node_material, render_entity) in computed_nodes.iter() {
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
    use crate::anchor::UIAnchorTarget;

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
        let node = UINode::default().with_visible(false);
        assert_eq!(node.style().display, Display::None);
    }

    #[test]
    fn percentages_use_a_zero_to_hundred_scale() {
        assert_eq!(dimension(UIValue::Percent(100.0)), Dimension::percent(1.0));
        assert_eq!(dimension(UIValue::Percent(25.0)), Dimension::percent(0.25));
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
        let node = UINode::default().with_overflow_x(Overflow::Hidden);
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
        text_measure(&ui_text(text))
    }

    fn label_width_in_row(measure: TextMeasure, ellipsis: bool, row_width: f32) -> f32 {
        let mut taffy: TaffyTree<LeafMeasure> = TaffyTree::new();
        let node = UINode::default().with_flex_grow(1.0);
        let label = taffy
            .new_leaf_with_context(
                leaf_style(&node, ellipsis, measure.wrap),
                LeafMeasure::Text(measure),
            )
            .unwrap();
        let row = taffy
            .new_with_children(
                UINode::default()
                    .with_width(UIValue::Px(row_width))
                    .with_flex_direction(FlexDirection::Row)
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

    fn card_heights(rows: f32, content_position: Position) -> (f32, f32) {
        let mut taffy: TaffyTree<LeafMeasure> = TaffyTree::new();
        let mut card = || {
            let pool = taffy
                .new_leaf(
                    UINode::default()
                        .with_height(UIValue::Px(rows * 32.0))
                        .with_flex_shrink(0.0)
                        .with_position(content_position)
                        .with_inset(UIInset {
                            left: UIValue::Px(0.0),
                            right: UIValue::Px(0.0),
                            top: UIValue::Px(0.0),
                            ..Default::default()
                        })
                        .style(),
                )
                .unwrap();
            let view = taffy
                .new_with_children(
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_direction(FlexDirection::Column)
                        .clipped()
                        .style(),
                    &[pool],
                )
                .unwrap();
            taffy
                .new_with_children(
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_shrink(1.0)
                        .with_flex_direction(FlexDirection::Column)
                        .style(),
                    &[view],
                )
                .unwrap()
        };
        let (first, second) = (card(), card());
        let rail = taffy
            .new_with_children(
                UINode::default()
                    .with_size(UIValue::Px(238.0), UIValue::Px(600.0))
                    .with_flex_direction(FlexDirection::Column)
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

    #[test]
    fn a_long_label_does_not_widen_the_row_inside_a_scroll_pool() {
        let mut taffy: TaffyTree<LeafMeasure> = TaffyTree::new();
        let measure = label("content/UAL1/scene.gasset");
        let label_node = UINode::default()
            .with_flex_grow(1.0)
            .with_padding(UIRect::axes(6.0, 8.0));
        let label_id = taffy
            .new_leaf_with_context(
                leaf_style(&label_node, true, measure.wrap),
                LeafMeasure::Text(measure),
            )
            .unwrap();
        let fixed = |taffy: &mut TaffyTree<LeafMeasure>, width: f32| {
            taffy
                .new_leaf(
                    UINode::default()
                        .with_width(UIValue::Px(width))
                        .with_flex_shrink(0.0)
                        .style(),
                )
                .unwrap()
        };
        let toggle = fixed(&mut taffy, 30.0);
        let glyph = fixed(&mut taffy, 20.0);
        let row = taffy
            .new_with_children(
                UINode::default()
                    .with_height(UIValue::Px(32.0))
                    .with_flex_shrink(0.0)
                    .with_flex_direction(FlexDirection::Row)
                    .style(),
                &[toggle, glyph, label_id],
            )
            .unwrap();
        let pool = taffy
            .new_with_children(
                UINode::default()
                    .with_flex_direction(FlexDirection::Column)
                    .with_flex_shrink(0.0)
                    .style(),
                &[row],
            )
            .unwrap();
        let view = taffy
            .new_with_children(
                UINode::default()
                    .with_size(UIValue::Px(197.0), UIValue::Px(300.0))
                    .with_flex_direction(FlexDirection::Column)
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

    fn field_widths(texts: &[&str], basis: UIValue) -> Vec<f32> {
        let mut taffy: TaffyTree<LeafMeasure> = TaffyTree::new();
        let fields: Vec<_> = texts
            .iter()
            .map(|text| {
                let measure = label(text);
                let node = UINode::default()
                    .with_flex_grow(1.0)
                    .with_size(basis, UIValue::Px(28.0))
                    .with_padding(UIRect::axes(0.0, 4.0))
                    .clipped();
                taffy
                    .new_leaf_with_context(
                        leaf_style(&node, false, measure.wrap),
                        LeafMeasure::Text(measure),
                    )
                    .unwrap()
            })
            .collect();
        let mut children = vec![
            taffy
                .new_leaf(
                    UINode::default()
                        .with_width(UIValue::Px(72.0))
                        .with_flex_shrink(0.0)
                        .style(),
                )
                .unwrap(),
        ];
        children.extend(fields.iter().copied());
        let row = taffy
            .new_with_children(
                UINode::default()
                    .with_width(UIValue::Px(260.0))
                    .with_flex_direction(FlexDirection::Row)
                    .with_gap(Vec2::new(4.0, 0.0))
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

    #[test]
    fn a_row_of_grown_fields_keeps_equal_widths_whatever_they_contain() {
        let uneven = ["0.000", "-1284.375", "7.5"];
        let widths = field_widths(&uneven, UIValue::Px(0.0));
        assert_eq!(
            widths,
            field_widths(&["0.000"; 3], UIValue::Px(0.0)),
            "a field's width must not depend on its text"
        );
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

    #[test]
    fn the_rotation_slot_is_part_of_the_material_signature() {
        use crate::material::UIMaterial;
        let mut material = UIMaterial::flat(concerto_color::Color::WHITE);
        let textures = RenderAssets::<RenderTexture>::new();
        let before = material_signature(&material, &textures);

        material.flags[1] = std::f32::consts::FRAC_PI_4;
        assert_ne!(before, material_signature(&material, &textures));
    }

    #[test]
    fn a_shared_viewport_without_a_picture_binds_no_texture() {
        let mut world = concerto_ecs::World::new();
        let node = world.spawn((
            UIMaterial::flat(concerto_color::Color::WHITE),
            UISharedViewport::new(),
        ));
        let mut system = concerto_ecs::IntoSystem::into_system(sync_shared_viewport_textures);
        concerto_ecs::System::initialize(&mut system, &mut world);

        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

        assert!(
            world
                .get_component_for_entity::<UIMaterial>(node)
                .unwrap()
                .texture
                .is_none()
        );
    }

    #[test]
    fn a_shared_viewport_with_an_empty_slot_binds_no_texture() {
        let mut world = concerto_ecs::World::new();
        let mut viewport = UISharedViewport::new();
        viewport.picture = Some(concerto_render::draw_target::SharedTexture::new(
            concerto_render::wgpu::TextureFormat::Rgba8Unorm,
        ));
        let node = world.spawn((UIMaterial::flat(concerto_color::Color::WHITE), viewport));
        let mut system = concerto_ecs::IntoSystem::into_system(sync_shared_viewport_textures);
        concerto_ecs::System::initialize(&mut system, &mut world);

        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

        assert!(
            world
                .get_component_for_entity::<UIMaterial>(node)
                .unwrap()
                .texture
                .is_none()
        );
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
            Some(&mut LeafMeasure::Text(label("anything at all"))),
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

    fn layout_world() -> concerto_ecs::World {
        let mut world = concerto_ecs::World::new();
        world.insert_resource(Window::new((800, 600), 1.0));
        world.insert_resource(UILayoutEngine::default());
        world.insert_resource(UILayoutDiagnostics::default());
        world.insert_resource(UIPanelStack::default());
        world.insert_resource(measurer());
        world.add_listener(blank_culled_layouts);
        install_layout_listeners(&mut world);
        lay_out(&mut world);
        world
    }

    #[derive(Resource)]
    struct LayOut(concerto_ecs::system::BoxedSystem);

    fn lay_out(world: &mut concerto_ecs::World) {
        let mut system = match world.remove_resource::<LayOut>() {
            Some(LayOut(system)) => system,
            None => {
                let mut system = concerto_ecs::IntoSystem::into_boxed_system(compute_ui_nodes);
                system.initialize(world);
                system
            }
        };
        system.run_and_apply((), world);
        world.insert_resource(LayOut(system));
    }

    fn layout_passes(world: &concerto_ecs::World) -> u64 {
        world
            .get_resource::<UILayoutDiagnostics>()
            .unwrap()
            .layout_passes
    }

    fn rect_of(world: &concerto_ecs::World, entity: Entity) -> UIBox {
        world
            .get_component_for_entity::<UILayout>(entity)
            .unwrap()
            .rect
    }

    fn clip_of(world: &concerto_ecs::World, entity: Entity) -> UIBox {
        world
            .get_component_for_entity::<UILayout>(entity)
            .unwrap()
            .clip_rect
    }

    fn column(world: &mut concerto_ecs::World) -> Entity {
        world.spawn(
            UINode::default()
                .with_size(UIValue::Px(800.0), UIValue::Px(600.0))
                .with_flex_direction(FlexDirection::Column),
        )
    }

    fn row(world: &mut concerto_ecs::World, parent: Entity) -> Entity {
        world.spawn((
            UINode::default()
                .with_size(UIValue::Px(100.0), UIValue::Px(50.0))
                .with_flex_shrink(0.0),
            ChildOf::new(parent),
        ))
    }

    #[test]
    fn a_first_layout_inserts_a_layout_and_a_render_mirror_on_every_node() {
        let mut world = layout_world();
        let root = column(&mut world);
        let first = row(&mut world, root);
        let inner = row(&mut world, first);
        let second = row(&mut world, root);

        lay_out(&mut world);

        for entity in [root, first, inner, second] {
            assert!(world.get_component_for_entity::<UILayout>(entity).is_some());
            assert!(
                world
                    .get_component_for_entity::<SyncWithRenderWorld>(entity)
                    .is_some()
            );
        }
    }

    fn settled_column(world: &mut concerto_ecs::World) -> (crate::testing::Watch, Vec<Entity>) {
        let root = column(world);
        let rows = (0..4).map(|_| row(world, root)).collect::<Vec<_>>();
        let mut watch = crate::testing::Watch::new::<UILayout, _>(world, compute_ui_nodes);
        watch.run(world);
        assert_eq!(watch.run(world), 0, "a settled layout writes nothing");
        (watch, rows)
    }

    #[test]
    fn a_taller_last_row_restamps_only_that_row() {
        let mut world = layout_world();
        let (mut watch, rows) = settled_column(&mut world);
        let above = rows[..3]
            .iter()
            .map(|&row| rect_of(&world, row))
            .collect::<Vec<_>>();
        let last = rows[3];
        world
            .get_component_for_entity_mut::<UINode>(last)
            .unwrap()
            .height = UIValue::Px(80.0);

        assert_eq!(watch.run(&mut world), 1);
        assert_eq!(rect_of(&world, last).size.y, 80.0);
        for (&row, rect) in rows[..3].iter().zip(above) {
            assert_eq!(rect_of(&world, row), rect);
        }
    }

    #[test]
    fn a_taller_first_row_restamps_it_and_the_rows_below_but_not_the_column() {
        let mut world = layout_world();
        let (mut watch, rows) = settled_column(&mut world);
        world
            .get_component_for_entity_mut::<UINode>(rows[0])
            .unwrap()
            .height = UIValue::Px(80.0);

        assert_eq!(watch.run(&mut world), rows.len());
        assert_eq!(rect_of(&world, rows[1]).min.y, 80.0);
    }

    #[test]
    fn a_culled_node_and_its_subtree_get_no_layout() {
        let mut world = layout_world();
        let root = column(&mut world);
        let first = row(&mut world, root);
        let middle = row(&mut world, root);
        let inner = row(&mut world, middle);
        let third = row(&mut world, root);
        world.insert(UICulled, middle);

        lay_out(&mut world);

        assert!(world.get_component_for_entity::<UILayout>(middle).is_none());
        assert!(world.get_component_for_entity::<UILayout>(inner).is_none());
        assert_eq!(rect_of(&world, third).min.y, rect_of(&world, first).max().y);
    }

    #[test]
    fn the_item_listeners_blank_a_culled_subtree_at_once() {
        let mut world = concerto_ecs::World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        crate::items::install_listeners(&mut world);
        let rect = UIBox {
            min: Vec2::new(5.0, 5.0),
            size: Vec2::new(40.0, 20.0),
        };
        let layout = || UILayout {
            rect,
            content_rect: rect,
            clip_rect: rect,
            paint_order: 0,
        };
        let parent = world.spawn(layout());
        let child = world.spawn((layout(), ChildOf::new(parent)));

        world.insert(UICulled, parent);

        assert!(!clip_of(&world, parent).has_area());
        assert!(!clip_of(&world, child).has_area());
    }

    #[test]
    fn culling_or_unculling_a_node_relays_out() {
        let mut world = layout_world();
        let root = column(&mut world);
        row(&mut world, root);
        let child = row(&mut world, root);
        lay_out(&mut world);
        let laid_out = layout_passes(&world);
        let shown_clip = clip_of(&world, child);
        assert!(shown_clip.has_area());

        world.insert(UICulled, child);
        lay_out(&mut world);
        let culled = layout_passes(&world);
        assert!(culled > laid_out, "culling must relay out");

        world.remove_component::<UICulled>(child);
        lay_out(&mut world);
        assert!(layout_passes(&world) > culled, "unculling must relay out");
        assert_eq!(rect_of(&world, child).size, Vec2::new(100.0, 50.0));
        assert_eq!(clip_of(&world, child), shown_clip);
    }

    fn hovered_at(world: &mut concerto_ecs::World, cursor: Vec2) -> Option<Entity> {
        world
            .get_resource_mut::<concerto_window::input::Input>()
            .unwrap()
            .update_mouse_position(cursor.x as f64, cursor.y as f64);
        let mut system =
            concerto_ecs::IntoSystem::into_system(crate::interaction::update_ui_interaction);
        concerto_ecs::System::initialize(&mut system, world);
        concerto_ecs::System::run_and_apply(&mut system, (), world);
        **world
            .get_resource::<crate::interaction::HoveredNode>()
            .unwrap()
    }

    #[test]
    fn a_newly_culled_subtree_is_neither_drawn_nor_hit() {
        let mut world = layout_world();
        world.insert_resource(concerto_window::input::Input::new());
        world.insert_resource(crate::interaction::HoveredNode::default());
        world.insert_resource(crate::interaction::UIInputState::default());
        let root = column(&mut world);
        row(&mut world, root);
        let child = row(&mut world, root);
        world.insert(crate::interaction::Interactable, child);
        let inner = world.spawn((
            UINode::default().with_size(UIValue::Px(10.0), UIValue::Px(10.0)),
            crate::interaction::Interactable,
            ChildOf::new(child),
        ));
        lay_out(&mut world);
        let old = rect_of(&world, child);
        let centre = old.min + old.size / 2.0;
        assert_eq!(hovered_at(&mut world, centre), Some(child));

        world.insert(UICulled, child);

        for entity in [child, inner] {
            assert_eq!(clip_of(&world, entity).size, Vec2::ZERO);
        }
        lay_out(&mut world);
        for entity in [child, inner] {
            assert_eq!(clip_of(&world, entity).size, Vec2::ZERO);
        }
        for cursor in [centre, old.min, rect_of(&world, inner).min] {
            let hovered = hovered_at(&mut world, cursor);
            assert!(
                hovered != Some(child) && hovered != Some(inner),
                "{cursor:?} hit a culled node"
            );
        }
    }

    #[test]
    fn nothing_moving_means_no_layout_pass() {
        let mut world = layout_world();
        let root = column(&mut world);
        row(&mut world, root);
        let culled = row(&mut world, root);
        row(&mut world, culled);
        world.insert(UICulled, culled);
        lay_out(&mut world);
        let passes = layout_passes(&world);

        lay_out(&mut world);
        lay_out(&mut world);

        assert_eq!(layout_passes(&world), passes);
    }

    #[test]
    fn a_panel_that_keeps_its_place_keeps_its_resolved_side_unchanged() {
        use crate::{
            anchor::{UIAnchorTarget, track_panel_stack},
            focus::{FocusedWidget, UIFocusGained, UIFocusLost},
            testing::Watch,
        };
        use concerto_ecs::events::event_channel::EventChannel;

        let mut world = layout_world();
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(EventChannel::<UIFocusLost>::default());
        world.insert_resource(EventChannel::<UIFocusGained>::default());
        let root = column(&mut world);
        let button = row(&mut world, root);
        let below = row(&mut world, root);
        world.spawn((
            UINode::default().with_size(UIValue::Px(80.0), UIValue::Px(40.0)),
            UIAnchoredPanel::new(UIAnchorTarget::from_node(button)).with_open(true),
        ));
        let mut track = concerto_ecs::IntoSystem::into_system(track_panel_stack);
        concerto_ecs::System::initialize(&mut track, &mut world);
        concerto_ecs::System::run_and_apply(&mut track, (), &mut world);

        let mut watch = Watch::new::<UIAnchoredPanel, _>(&mut world, compute_ui_nodes);
        watch.run(&mut world);
        let passes = layout_passes(&world);
        world
            .get_component_for_entity_mut::<UINode>(below)
            .unwrap()
            .height = UIValue::Px(60.0);

        assert_eq!(watch.run(&mut world), 0);
        assert_eq!(layout_passes(&world), passes + 1);
    }

    #[test]
    fn culled_subtrees_stay_out_of_the_taffy_tree() {
        let mut world = layout_world();
        let root = column(&mut world);
        let shown = row(&mut world, root);
        for _ in 0..1_000 {
            let hidden = row(&mut world, root);
            row(&mut world, hidden);
            world.insert(UICulled, hidden);
        }
        let visited = visited(&world);

        lay_out(&mut world);

        let engine = world.get_resource::<UILayoutEngine>().unwrap();
        let root_id = engine.nodes[&root].id;
        assert_eq!(engine.taffy.children(root_id).unwrap().len(), 1);
        assert_eq!(
            engine.visited - visited,
            2,
            "only the root and its shown row are walked"
        );
        assert!(world.get_component_for_entity::<UILayout>(shown).is_some());
    }

    fn visited(world: &concerto_ecs::World) -> u64 {
        world.get_resource::<UILayoutEngine>().unwrap().visited
    }

    fn nodes_updated(world: &concerto_ecs::World) -> u64 {
        world
            .get_resource::<UILayoutDiagnostics>()
            .unwrap()
            .nodes_updated
    }

    fn measured(world: &mut concerto_ecs::World) -> Vec<String> {
        std::mem::take(&mut world.get_resource_mut::<UITextMeasure>().unwrap().measured)
    }

    fn ui_text(text: &str) -> crate::text::UIText {
        crate::text::UIText {
            text: text.into(),
            font_size: 14.0,
            line_height: 20.0,
            wrap: false,
            ..Default::default()
        }
    }

    fn labelled(world: &mut concerto_ecs::World, parent: Entity, label: &str) -> Entity {
        world.spawn((
            UINode::default()
                .with_flex_shrink(0.0)
                .with_align_self(AlignItems::FlexStart),
            ui_text(label),
            ChildOf::new(parent),
        ))
    }

    #[test]
    fn nothing_changing_adds_no_layout_pass_and_updates_no_node() {
        let mut world = layout_world();
        let root = column(&mut world);
        let first = row(&mut world, root);
        row(&mut world, first);
        labelled(&mut world, root, "Warren");
        lay_out(&mut world);
        world.get_component_for_entity_mut::<UINode>(root);
        lay_out(&mut world);
        assert_eq!(
            nodes_updated(&world),
            0,
            "an identical write updates no node"
        );
        let passes = layout_passes(&world);

        for _ in 0..3 {
            lay_out(&mut world);
            assert_eq!(layout_passes(&world), passes);
            assert_eq!(nodes_updated(&world), 0);
        }
    }

    #[test]
    fn nothing_changing_walks_no_node() {
        let mut world = layout_world();
        let root = column(&mut world);
        let first = row(&mut world, root);
        row(&mut world, first);
        row(&mut world, root);
        lay_out(&mut world);
        let walked = visited(&world);
        assert!(walked > 0, "the first layout walks the tree");

        for _ in 0..3 {
            lay_out(&mut world);
        }

        assert_eq!(visited(&world), walked);
    }

    #[test]
    fn one_changed_label_among_a_thousand_measures_only_that_label() {
        let mut world = layout_world();
        let root = column(&mut world);
        let labels = (0..1_000)
            .map(|index| labelled(&mut world, root, &format!("label {index}")))
            .collect::<Vec<_>>();
        lay_out(&mut world);
        lay_out(&mut world);
        measured(&mut world);
        let passes = layout_passes(&world);

        world
            .get_component_for_entity_mut::<crate::text::UIText>(labels[500])
            .unwrap()
            .text = "a label that changed".into();
        lay_out(&mut world);

        assert_eq!(layout_passes(&world), passes + 1);
        assert_eq!(nodes_updated(&world), 1);
        let measured = measured(&mut world);
        assert!(!measured.is_empty(), "the changed label is measured");
        assert!(
            measured.iter().all(|text| text == "a label that changed"),
            "only the changed label is measured: {measured:?}"
        );
    }

    #[test]
    fn a_pass_that_leaves_a_label_alone_does_not_shape_it_again() {
        let mut world = layout_world();
        let root = column(&mut world);
        let sibling = row(&mut world, root);
        labelled(&mut world, root, "Curiosities");
        lay_out(&mut world);
        measured(&mut world);
        let passes = layout_passes(&world);

        world
            .get_component_for_entity_mut::<UINode>(sibling)
            .unwrap()
            .width = UIValue::Px(120.0);
        lay_out(&mut world);

        assert_eq!(layout_passes(&world), passes + 1);
        assert_eq!(measured(&mut world), Vec::<String>::new());
    }

    #[test]
    fn a_changed_label_is_measured_again() {
        let mut world = layout_world();
        let root = column(&mut world);
        let label = labelled(&mut world, root, "Warren");
        labelled(&mut world, root, "Curiosities");
        lay_out(&mut world);
        let short = rect_of(&world, label).size.x;
        measured(&mut world);

        world
            .get_component_for_entity_mut::<crate::text::UIText>(label)
            .unwrap()
            .text = "Warren and Curiosities".into();
        lay_out(&mut world);

        let measured = measured(&mut world);
        assert!(!measured.is_empty());
        assert!(measured.iter().all(|text| text == "Warren and Curiosities"));
        assert!(rect_of(&world, label).size.x > short);
    }

    #[test]
    fn writing_an_identical_node_and_text_adds_no_layout_pass() {
        let mut world = layout_world();
        let root = column(&mut world);
        let label = labelled(&mut world, root, "Warren");
        lay_out(&mut world);
        let passes = layout_passes(&world);

        for _ in 0..3 {
            let node = world
                .get_component_for_entity::<UINode>(label)
                .unwrap()
                .clone();
            *world.get_component_for_entity_mut::<UINode>(label).unwrap() = node;
            let component = world
                .get_component_for_entity_mut::<crate::text::UIText>(label)
                .unwrap();
            component.text = component.text.clone();
            lay_out(&mut world);
        }

        assert_eq!(layout_passes(&world), passes);
        assert_eq!(nodes_updated(&world), 0);
    }

    type Snapshot = Vec<(Entity, UIBox, UIBox, UIBox, i64)>;

    fn snapshot(world: &mut concerto_ecs::World) -> Snapshot {
        let mut layouts = world
            .query::<(Entity, &UILayout), ()>()
            .iter(world)
            .map(|(entity, layout)| {
                (
                    entity,
                    layout.rect,
                    layout.content_rect,
                    layout.clip_rect,
                    layout.paint_order,
                )
            })
            .collect::<Vec<_>>();
        layouts.sort_by_key(|(entity, ..)| (entity.index(), entity.generation()));
        layouts
    }

    fn assert_matches_a_fresh_layout(world: &mut concerto_ecs::World) {
        let retained = snapshot(world);
        let engine = world.remove_resource::<UILayoutEngine>().unwrap();
        let system = world.remove_resource::<LayOut>();
        world.insert_resource(UILayoutEngine::default());
        lay_out(world);
        let fresh = snapshot(world);
        world.insert_resource(engine);
        match system {
            Some(system) => world.insert_resource(system),
            None => {
                world.remove_resource::<LayOut>();
            }
        }
        assert_eq!(
            retained, fresh,
            "the retained layout differs from a fresh one"
        );
    }

    struct Structure {
        root: Entity,
        rows: Vec<Entity>,
        nested: Entity,
        grown: Entity,
    }

    fn structure(world: &mut concerto_ecs::World) -> Structure {
        let root = column(world);
        let rows = (0..3).map(|_| row(world, root)).collect::<Vec<_>>();
        let grown = world.spawn((
            UINode::default()
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Column)
                .with_padding(UIRect::all(4.0)),
            ChildOf::new(root),
        ));
        let nested = row(world, grown);
        row(world, grown);
        labelled(world, root, "Warren");
        lay_out(world);
        lay_out(world);
        Structure {
            root,
            rows,
            nested,
            grown,
        }
    }

    #[test]
    fn adding_a_child_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        row(&mut world, tree.grown);
        lay_out(&mut world);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn removing_a_child_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        world.despawn(tree.rows[1]);
        lay_out(&mut world);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn reordering_children_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        let before = rect_of(&world, tree.rows[0]);
        world
            .get_component_for_entity_mut::<Children>(tree.root)
            .unwrap()
            .sort_by_key(|entity| std::cmp::Reverse(entity.index()));
        lay_out(&mut world);
        assert_ne!(rect_of(&world, tree.rows[0]), before);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn culling_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        world.insert(UICulled, tree.grown);
        lay_out(&mut world);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn unculling_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        world.insert(UICulled, tree.grown);
        lay_out(&mut world);
        world
            .get_component_for_entity_mut::<UINode>(tree.nested)
            .unwrap()
            .height = UIValue::Px(70.0);
        lay_out(&mut world);
        world.remove_component::<UICulled>(tree.grown);
        lay_out(&mut world);
        assert_eq!(rect_of(&world, tree.nested).size.y, 70.0);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn culling_and_unculling_between_passes_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        world.insert(UICulled, tree.grown);
        world.remove_component::<UICulled>(tree.grown);
        lay_out(&mut world);
        assert!(clip_of(&world, tree.nested).has_area());
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn despawning_a_subtree_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        world.despawn(tree.grown);
        lay_out(&mut world);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn giving_a_root_a_parent_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        let other = world.spawn(
            UINode::default()
                .with_size(UIValue::Px(40.0), UIValue::Px(30.0))
                .with_flex_shrink(0.0),
        );
        lay_out(&mut world);
        world.insert(ChildOf::new(tree.grown), other);
        lay_out(&mut world);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn removing_a_parent_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        world.remove_component::<ChildOf>(tree.nested);
        lay_out(&mut world);
        assert_eq!(rect_of(&world, tree.nested).min, Vec2::ZERO);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn adding_and_removing_text_lays_out_as_a_fresh_engine_would() {
        let mut world = layout_world();
        let tree = structure(&mut world);
        let bare = rect_of(&world, tree.grown);
        world.insert(ui_text("a label on a grown node"), tree.nested);
        world
            .get_component_for_entity_mut::<UINode>(tree.nested)
            .unwrap()
            .height = UIValue::Auto;
        lay_out(&mut world);
        assert_ne!(rect_of(&world, tree.grown), bare);
        assert_matches_a_fresh_layout(&mut world);

        world.remove_component::<crate::text::UIText>(tree.nested);
        lay_out(&mut world);
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn a_window_resize_lays_out_again() {
        let mut world = layout_world();
        let root = world.spawn(
            UINode::default()
                .with_size(UIValue::Percent(50.0), UIValue::Percent(100.0))
                .with_flex_direction(FlexDirection::Column),
        );
        lay_out(&mut world);
        assert_eq!(rect_of(&world, root).size, Vec2::new(400.0, 600.0));
        let passes = layout_passes(&world);

        world.insert_resource(Window::new((1000, 500), 1.0));
        lay_out(&mut world);

        assert_eq!(layout_passes(&world), passes + 1);
        assert_eq!(rect_of(&world, root).size, Vec2::new(500.0, 500.0));
    }

    fn panel_world() -> concerto_ecs::World {
        use crate::focus::{FocusedWidget, UIFocusGained, UIFocusLost};
        use concerto_ecs::events::event_channel::EventChannel;

        let mut world = layout_world();
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(EventChannel::<UIFocusLost>::default());
        world.insert_resource(EventChannel::<UIFocusGained>::default());
        world
    }

    #[derive(Resource)]
    struct Track(concerto_ecs::system::BoxedSystem);

    fn track_and_lay_out(world: &mut concerto_ecs::World) {
        let mut system = match world.remove_resource::<Track>() {
            Some(Track(system)) => system,
            None => {
                let mut system =
                    concerto_ecs::IntoSystem::into_boxed_system(crate::anchor::track_panel_stack);
                system.initialize(world);
                system
            }
        };
        system.run_and_apply((), world);
        world.insert_resource(Track(system));
        lay_out(world);
    }

    fn panel(world: &mut concerto_ecs::World, target: Entity) -> Entity {
        world.spawn((
            UINode::default().with_size(UIValue::Px(80.0), UIValue::Px(40.0)),
            UIAnchoredPanel::new(UIAnchorTarget::from_node(target)),
        ))
    }

    fn open(world: &mut concerto_ecs::World, panel: Entity) {
        world
            .get_component_for_entity_mut::<UIAnchoredPanel>(panel)
            .unwrap()
            .open = true;
    }

    #[test]
    fn an_open_panel_follows_its_target_when_only_the_target_moves() {
        let mut world = panel_world();
        let root = column(&mut world);
        let above = row(&mut world, root);
        let button = row(&mut world, root);
        let menu = panel(&mut world, button);
        open(&mut world, menu);
        track_and_lay_out(&mut world);
        track_and_lay_out(&mut world);
        let before = rect_of(&world, menu).min;

        world
            .get_component_for_entity_mut::<UINode>(above)
            .unwrap()
            .height = UIValue::Px(90.0);
        track_and_lay_out(&mut world);

        assert_eq!(rect_of(&world, menu).min, before + Vec2::new(0.0, 40.0));
        assert_eq!(
            rect_of(&world, menu).min.y,
            rect_of(&world, button).max().y + 4.0
        );
    }

    #[test]
    fn moving_an_open_panels_point_re_places_it_without_a_layout_pass() {
        let mut world = panel_world();
        column(&mut world);
        let menu = world.spawn((
            UINode::default().with_size(UIValue::Px(80.0), UIValue::Px(40.0)),
            UIAnchoredPanel::new(UIAnchorTarget::from_point(Vec2::new(100.0, 100.0)))
                .with_open(true),
        ));
        track_and_lay_out(&mut world);
        track_and_lay_out(&mut world);
        let passes = layout_passes(&world);

        world
            .get_component_for_entity_mut::<UIAnchoredPanel>(menu)
            .unwrap()
            .target = UIAnchorTarget::from_point(Vec2::new(200.0, 150.0));
        track_and_lay_out(&mut world);

        assert_eq!(layout_passes(&world), passes);
        assert_eq!(rect_of(&world, menu).min.x, 200.0);
    }

    #[test]
    fn opening_a_second_panel_moves_the_first_to_its_new_layer() {
        let mut world = panel_world();
        let root = column(&mut world);
        let button = row(&mut world, root);
        let second = panel(&mut world, button);
        let first = panel(&mut world, button);
        open(&mut world, first);
        track_and_lay_out(&mut world);
        track_and_lay_out(&mut world);
        let before = world
            .get_component_for_entity::<UILayout>(first)
            .unwrap()
            .paint_order;

        open(&mut world, second);
        track_and_lay_out(&mut world);

        let z_index = world
            .get_component_for_entity::<UINode>(first)
            .unwrap()
            .z_index;
        let after = world
            .get_component_for_entity::<UILayout>(first)
            .unwrap()
            .paint_order;
        assert_ne!(after, before);
        assert_eq!(after >> 32, z_index as i64);
    }

    #[test]
    fn a_new_z_index_alone_repaints_without_a_layout_pass() {
        let mut world = layout_world();
        let root = column(&mut world);
        let first = row(&mut world, root);
        let inner = row(&mut world, first);
        lay_out(&mut world);
        let passes = layout_passes(&world);

        world
            .get_component_for_entity_mut::<UINode>(first)
            .unwrap()
            .z_index = 7;
        lay_out(&mut world);

        assert_eq!(layout_passes(&world), passes);
        for entity in [first, inner] {
            let paint_order = world
                .get_component_for_entity::<UILayout>(entity)
                .unwrap()
                .paint_order;
            assert_eq!(paint_order >> 32, 7);
        }
    }

    #[test]
    fn a_node_added_deep_inside_a_hidden_node_is_laid_out() {
        let mut world = layout_world();
        let root = column(&mut world);
        let hidden = row(&mut world, root);
        world
            .get_component_for_entity_mut::<UINode>(hidden)
            .unwrap()
            .visible = false;
        let inner = row(&mut world, hidden);
        lay_out(&mut world);

        let added = row(&mut world, inner);
        lay_out(&mut world);

        assert!(world.get_component_for_entity::<UILayout>(added).is_some());
        assert_matches_a_fresh_layout(&mut world);
    }

    #[test]
    fn despawning_one_child_relays_out_its_siblings() {
        let mut world = layout_world();
        let root = column(&mut world);
        let rows = (0..3).map(|_| row(&mut world, root)).collect::<Vec<_>>();
        lay_out(&mut world);
        assert_eq!(rect_of(&world, rows[1]).min.y, 50.0);

        world.despawn(rows[0]);
        lay_out(&mut world);

        assert_eq!(rect_of(&world, rows[1]).min.y, 0.0);
        assert_eq!(rect_of(&world, rows[2]).min.y, 50.0);
    }

    #[test]
    fn detaching_a_parents_only_child_relays_out_the_parent() {
        let mut world = layout_world();
        let root = column(&mut world);
        let parent = world.spawn((
            UINode::default()
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Column),
            ChildOf::new(root),
        ));
        let only = row(&mut world, parent);
        lay_out(&mut world);
        assert_eq!(rect_of(&world, parent).size.y, 50.0);

        world.remove_component::<ChildOf>(only);
        lay_out(&mut world);

        assert_eq!(rect_of(&world, parent).size.y, 0.0);
    }

    fn drawing_app(gpu: &concerto_render::gpu::Gpu) -> concerto_app::App {
        let mut app = concerto_app::App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(concerto_render::draw_target::DrawTarget::Texture(
                SharedTexture::new(wgpu::TextureFormat::Rgba8Unorm),
            ))
            .insert_resource(Window::new((64, 64), 1.0));
        app.register_plugin(concerto_app::main_schedule::MainSchedulePlugin)
            .register_plugin(concerto_app::plugins::AssetManagerPlugin)
            .register_plugin(concerto_app::plugins::TimePlugin)
            .register_plugin(concerto_window::plugin::WindowPlugin)
            .register_plugin(concerto_app::plugins::TransformPlugin)
            .register_plugin(concerto_render::plugin::RenderPlugin)
            .register_plugin(crate::plugin::UIPlugin);
        app
    }

    fn drawn_quad(app: &concerto_app::App, node: Entity) -> bool {
        let render_entity = **app
            .main()
            .world()
            .get_component_for_entity::<RenderEntity>(node)
            .unwrap();
        app.render()
            .world()
            .get_component_for_entity::<RenderUINode>(render_entity)
            .is_some()
    }

    #[test]
    fn a_quad_whose_clip_is_empty_is_not_drawn() {
        let Some(gpu) = pollster::block_on(concerto_render::gpu::Gpu::request(
            &concerto_render::gpu::Gpu::instance(),
            None,
        )) else {
            return;
        };
        let mut app = drawing_app(&gpu);
        let node = app.main_mut().world_mut().spawn((
            UINode::default().with_size(UIValue::Px(32.0), UIValue::Px(32.0)),
            UIMaterial::flat(concerto_color::Color::WHITE),
        ));
        app.finish_plugin_build();
        app.update();
        app.update();
        assert!(drawn_quad(&app, node), "a shown quad is drawn");

        app.main_mut().world_mut().insert(UICulled, node);
        app.update();
        assert!(!drawn_quad(&app, node), "a culled quad leaves no draw");

        app.main_mut()
            .world_mut()
            .remove_component::<UICulled>(node);
        app.update();
        assert!(drawn_quad(&app, node), "an unculled quad is drawn again");
    }

    #[test]
    fn explicit_z_index_dominates_tree_sequence() {
        assert!(paint_order(1, 0) > paint_order(0, i32::MAX as i64));
    }

    #[test]
    fn a_child_paints_on_its_parents_layer_unless_it_asks_for_more() {
        assert_eq!(effective_layer(200, 0), 200);
        assert_eq!(effective_layer(70, 71), 71);
        assert_eq!(effective_layer(70, 10), 70);
    }

    #[test]
    fn paint_order_packs_the_layer_above_the_sequence() {
        assert_eq!(paint_order(3, 0) >> 32, 3);
        assert_eq!(paint_order(3, i32::MAX as i64) >> 32, 3);
        assert!(paint_order(3, 1) > paint_order(3, 0));
        assert!(paint_order(4, 0) > paint_order(3, i32::MAX as i64));
    }
}

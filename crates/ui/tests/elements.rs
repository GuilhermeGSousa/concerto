//! Covers the themed element builders: defaults follow the theme, modifiers compose in any order.
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, World,
    component::bundle::ComponentBundle,
};
use concerto_ui::{
    elements::prelude::*,
    interaction::UIInteractionStyle,
    material::UIMaterial,
    node::{AlignItems, FlexDirection, Overflow, UINode, UIRect},
    text::{FontFamily, UIText},
    theme::{ButtonVariant, ChipColors, UITheme},
    transform::UIValue,
};
use glam::Vec2;

struct Spawned {
    world: World,
    entity: Entity,
}

impl Spawned {
    fn get<T: Component>(&self) -> &T {
        self.world
            .get_component_for_entity::<T>(self.entity)
            .unwrap_or_else(|| panic!("missing {}", std::any::type_name::<T>()))
    }

    fn has<T: Component>(&self) -> bool {
        self.world.get_component_for_entity::<T>(self.entity).is_some()
    }
}

fn spawn(bundle: impl ComponentBundle + 'static) -> Spawned {
    let mut world = World::default();
    let entity = world.spawn(bundle);
    Spawned { world, entity }
}

fn theme() -> UITheme {
    UITheme::nocturne()
}

fn colors(style: &UIInteractionStyle) -> [Color; 4] {
    [style.normal, style.hovered, style.pressed, style.disabled]
}

#[derive(Component)]
struct Marker;

#[test]
fn a_single_value_pads_every_side() {
    assert_eq!(UIRect::from(3.0), UIRect::all(3.0));
}

#[test]
fn selection_is_the_accent_at_a_fifth_alpha() {
    let theme = theme();
    let accent = theme.accent.to_srgba();
    assert_eq!(
        theme.selection(),
        Color::srgba(accent.r, accent.g, accent.b, 0.2)
    );
}

#[test]
fn interaction_follows_the_variant_table() {
    let t = theme();
    let disabled_solid = Color::srgba(0.09, 0.075, 0.11, 0.55);
    let cases = [
        (ButtonVariant::Solid, false, [t.surface_raised, t.surface_hovered, t.accent, disabled_solid]),
        (ButtonVariant::Solid, true, [t.surface_hovered, t.surface_hovered, t.accent, disabled_solid]),
        (ButtonVariant::Ghost, false, [Color::TRANSPARENT, t.surface_hovered, t.selection(), Color::TRANSPARENT]),
        (ButtonVariant::Ghost, true, [t.selection(), t.surface_hovered, t.selection(), Color::TRANSPARENT]),
        (ButtonVariant::Tab, false, [t.surface, t.surface_hovered, t.accent, t.surface]),
        (ButtonVariant::Tab, true, [t.surface_raised, t.surface_hovered, t.accent, t.surface]),
    ];
    for (variant, selected, expected) in cases {
        assert_eq!(
            colors(&t.interaction(variant, selected)),
            expected,
            "{variant:?} selected={selected}"
        );
    }
}

#[test]
fn chip_colours_mark_the_selected_chip() {
    let t = theme();
    assert_eq!(
        t.chip_colors(false),
        ChipColors { fill: Color::TRANSPARENT, border: t.border, text: t.text_muted }
    );
    assert_eq!(
        t.chip_colors(true),
        ChipColors { fill: t.selection(), border: t.accent, text: t.text }
    );
}

#[test]
fn text_defaults_follow_the_theme() {
    let t = theme();
    let spawned = spawn(t.text("Warren"));
    let text = spawned.get::<UIText>();
    assert_eq!(text.text, "Warren");
    assert_eq!(text.font_size, t.font_size_md);
    assert_eq!(text.line_height, t.line_height(t.font_size_md));
    assert_eq!(text.color, t.text, "body text uses the theme colour, not UIText's default");
    assert!(text.wrap);
    assert!(!text.ellipsis);
}

#[test]
fn a_label_is_a_node_with_text() {
    let spawned = spawn(theme().label("Curiosities"));
    assert_eq!(spawned.get::<UINode>(), &UINode::default());
    assert_eq!(spawned.get::<UIText>().text, "Curiosities");
}

#[test]
fn font_size_always_pairs_its_line_height() {
    let t = theme();
    let spawned = spawn(t.label("x").font_size(20.0));
    let text = spawned.get::<UIText>();
    assert_eq!(text.font_size, 20.0);
    assert_eq!(text.line_height, t.line_height(20.0));
}

#[test]
fn typography_modifiers_compose_in_any_order() {
    let t = theme();
    let a = spawn(t.text("x").small().muted().mono().weight(500));
    let b = spawn(t.text("x").mono().weight(500).muted().small());
    for spawned in [&a, &b] {
        let text = spawned.get::<UIText>();
        assert_eq!(text.font_size, t.font_size_sm);
        assert_eq!(text.line_height, t.line_height(t.font_size_sm));
        assert_eq!(text.color, t.text_muted);
        assert!(matches!(text.font_family, FontFamily::Monospace));
        assert_eq!(text.font_weight, 500);
    }
}

#[test]
fn large_color_and_family_set_their_fields() {
    let t = theme();
    let spawned = spawn(
        t.text("x")
            .large()
            .color(t.accent)
            .family(FontFamily::Name("Inter".into())),
    );
    let text = spawned.get::<UIText>();
    assert_eq!(text.font_size, t.font_size_lg);
    assert_eq!(text.color, t.accent);
    assert!(matches!(&text.font_family, FontFamily::Name(name) if name == "Inter"));
}

#[test]
fn single_line_ellipsises_and_no_wrap_does_not() {
    let t = theme();
    let single = spawn(t.text("x").single_line());
    assert!(!single.get::<UIText>().wrap);
    assert!(single.get::<UIText>().ellipsis);
    let plain = spawn(t.text("x").no_wrap());
    assert!(!plain.get::<UIText>().wrap);
    assert!(!plain.get::<UIText>().ellipsis);
}

#[test]
fn layout_modifiers_shape_the_node() {
    let spawned = spawn(
        theme()
            .label("x")
            .width(UIValue::Px(10.0))
            .height(UIValue::Px(12.0))
            .min_width(UIValue::Px(1.0))
            .max_height(UIValue::Px(40.0))
            .row()
            .gap(3.0)
            .padding(2.0)
            .margin(UIRect::axes(1.0, 4.0))
            .grow()
            .fixed()
            .align_self(AlignItems::End)
            .clipped()
            .hidden()
            .z_index(5),
    );
    let node = spawned.get::<UINode>();
    assert_eq!(node.width, UIValue::Px(10.0));
    assert_eq!(node.height, UIValue::Px(12.0));
    assert_eq!(node.min_width, UIValue::Px(1.0));
    assert_eq!(node.max_height, UIValue::Px(40.0));
    assert_eq!(node.flex_direction, FlexDirection::Row);
    assert_eq!(node.align_items, Some(AlignItems::Center), "row() centres its children");
    assert_eq!(node.gap, Vec2::splat(3.0));
    assert_eq!(node.padding, UIRect::all(2.0));
    assert_eq!(node.margin, UIRect::axes(1.0, 4.0));
    assert_eq!(node.flex_grow, 1.0);
    assert_eq!(node.flex_shrink, 0.0);
    assert_eq!(node.align_self, Some(AlignItems::End));
    assert_eq!(node.overflow_x, Overflow::Hidden);
    assert!(!node.visible);
    assert_eq!(node.z_index, 5);
}

#[test]
fn the_node_escape_hatch_reaches_any_uinode_setting() {
    let spawned = spawn(theme().label("x").node(|node| node.with_overflow_y(Overflow::Scroll)));
    assert_eq!(spawned.get::<UINode>().overflow_y, Overflow::Scroll);
}

#[test]
fn label_nests_with_extra_components() {
    let spawned = spawn((theme().label("x").muted(), Marker));
    assert!(spawned.has::<Marker>());
    assert!(spawned.has::<UINode>());
    assert_eq!(spawned.get::<UIText>().color, theme().text_muted);
}

#[test]
fn text_reinserted_replaces_existing_text() {
    let t = theme();
    let mut spawned = spawn((UINode::default(), t.text("before")));
    spawned.world.insert(t.text("after").muted(), spawned.entity);
    assert_eq!(spawned.get::<UIText>().text, "after");
    assert_eq!(spawned.get::<UIText>().color, t.text_muted);
}

#[test]
fn text_converts_into_a_plain_uitext() {
    let t = theme();
    let text: UIText = t.text("x").small().into();
    assert_eq!(text.font_size, t.font_size_sm);
}

fn material_of(spawned: &Spawned) -> &UIMaterial {
    spawned.get::<UIMaterial>()
}

#[test]
fn each_surface_takes_its_fill_and_radius_from_the_theme() {
    let t = theme();
    let cases = [
        (spawn(t.canvas()), t.canvas, 0.0),
        (spawn(t.panel()), t.surface, t.radius_lg),
        (spawn(t.card()), t.surface_raised, t.radius_md),
    ];
    for (spawned, fill, radius) in &cases {
        let material = material_of(spawned);
        assert_eq!(material.color, fill.to_linear());
        assert_eq!(material.corner_radius, *radius);
        assert_eq!(material.border_width, 0.0);
    }
}

#[test]
fn a_popup_is_a_bordered_clipped_column_card() {
    let t = theme();
    let spawned = spawn(t.popup());
    let material = material_of(&spawned);
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.corner_radius, t.radius_md);
    assert_eq!(material.border_color, t.border.to_linear());
    assert_eq!(material.border_width, 1.0);
    let node = spawned.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Column);
    assert_eq!(node.overflow_x, Overflow::Hidden);
    assert_eq!(node.overflow_y, Overflow::Hidden);
}

#[test]
fn shape_modifiers_restyle_a_surface() {
    let t = theme();
    let spawned = spawn(
        t.panel()
            .fill(t.accent)
            .radius_sm()
            .border(t.focus, 2.0)
            .rotation(0.5),
    );
    let material = material_of(&spawned);
    assert_eq!(material.color, t.accent.to_linear());
    assert_eq!(material.corner_radius, t.radius_sm);
    assert_eq!(material.border_color, t.focus.to_linear());
    assert_eq!(material.border_width, 2.0);
    assert_eq!(material.rotation, 0.5);
}

#[test]
fn stacks_space_their_children_by_the_theme() {
    let t = theme();
    let row = spawn(t.row());
    let node = row.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Row);
    assert_eq!(node.align_items, Some(AlignItems::Center));
    assert_eq!(node.gap, Vec2::splat(t.spacing_sm));

    let column = spawn(t.column().gap(0.0));
    let node = column.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Column);
    assert_eq!(node.gap, Vec2::ZERO);
}

#[test]
fn dividers_are_one_border_coloured_pixel() {
    let t = theme();
    let horizontal = spawn(t.divider());
    assert_eq!(horizontal.get::<UINode>().height, UIValue::Px(1.0));
    assert_eq!(horizontal.get::<UINode>().flex_shrink, 0.0);
    assert_eq!(horizontal.get::<UINode>().align_self, Some(AlignItems::Stretch));
    assert_eq!(material_of(&horizontal).color, t.border.to_linear());

    let vertical = spawn(t.divider().vertical());
    assert_eq!(vertical.get::<UINode>().width, UIValue::Px(1.0));
    assert_eq!(vertical.get::<UINode>().height, UIValue::Auto);
}

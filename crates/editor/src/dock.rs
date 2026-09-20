//! The shell's skeleton: named regions overlaid on a full-bleed scene.
//!
//! Nothing here knows what Warren or the ECS inspector is. Panels declare an id,
//! a title and a region; the dock places the regions and hands each panel a body
//! entity to build into.
//!
//! The topology is a constant. Panels move between regions by changing their
//! declaration, never by dragging — which is what lets shortcuts and muscle
//! memory mean something.
use std::collections::HashMap;

use app::{schedule_groups::Startup, App, Plugin};
use ecs::{command::CommandQueue, system::NonSendMarker, Entity, Res, ResMut, Resource};
use taffy::{FlexDirection, Position};
use ui::{
    interaction::Interactable,
    material::UIMaterial,
    node::{UIInset, UINode, UIRect},
    text::TextComponent,
    theme::UITheme,
    transform::UIValue,
};

/// Where a panel sits over the scene.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Region {
    /// Full-bleed, behind everything else. The scene itself.
    Scene,
    /// Top strip, flush left — what is open.
    Brand,
    /// Top strip, flush right — running costs.
    Stats,
    /// Left rail. Panels stack, sharing its height.
    Rail,
    /// Right card.
    Side,
    /// Centre foot, stacked above the bottom edge.
    Foot,
}

impl Region {
    const ALL: [Region; 6] = [
        Region::Scene,
        Region::Brand,
        Region::Stats,
        Region::Rail,
        Region::Side,
        Region::Foot,
    ];

    /// Whether the region's panels are floating cards rather than bare chrome.
    fn is_card(self) -> bool {
        matches!(self, Region::Rail | Region::Side)
    }
}

/// A panel's declaration. Data only: building is the panel's own business, done
/// by a `Startup` system that looks its body up by id.
#[derive(Clone, Copy)]
pub struct PanelDescriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub region: Region,
}

#[derive(Resource, Default)]
pub struct PanelRegistry {
    panels: Vec<PanelDescriptor>,
    bodies: HashMap<&'static str, Entity>,
    root: Option<Entity>,
}

impl PanelRegistry {
    pub fn register(&mut self, panel: PanelDescriptor) {
        if self.panels.iter().any(|existing| existing.id == panel.id) {
            log::warn!(
                "Panel '{}' is already registered; ignoring the duplicate",
                panel.id
            );
            return;
        }
        self.panels.push(panel);
    }

    /// The node every region is placed over, once the dock has run. Anything
    /// that has to sit above the whole shell rather than inside one panel — a
    /// window resize grip, say — belongs here.
    pub fn root(&self) -> Option<Entity> {
        self.root
    }

    /// The entity a panel builds its contents into, once the dock has run.
    pub fn body(&self, id: &str) -> Option<Entity> {
        self.bodies.get(id).copied()
    }

    pub fn panels(&self) -> &[PanelDescriptor] {
        &self.panels
    }

    fn in_region(&self, region: Region) -> Vec<PanelDescriptor> {
        self.panels
            .iter()
            .copied()
            .filter(|panel| panel.region == region)
            .collect()
    }
}

/// Registers a panel. Call from a panel plugin's `build`.
pub trait DockedApp {
    fn add_panel(&mut self, panel: PanelDescriptor) -> &mut Self;
}

impl DockedApp for App {
    fn add_panel(&mut self, panel: PanelDescriptor) -> &mut Self {
        self.get_resource_mut::<PanelRegistry>()
            .expect("DockPlugin must be registered before any panel")
            .register(panel);
        self
    }
}

pub struct DockPlugin;

impl Plugin for DockPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, build_dock);
    }
}

/// Gap from the window edge, in logical pixels.
const MARGIN: f32 = 18.0;
/// Height of the top strip.
const TOP: f32 = 34.0;
/// Height of the band above the cards: the top strip plus its margin. This is
/// the window's title area, even though nothing draws a title bar.
pub const TOP_STRIP: f32 = TOP + MARGIN;
/// Width of the left rail.
const RAIL: f32 = 238.0;
/// Width of the right card.
const SIDE: f32 = 290.0;

fn build_dock(
    // `set_title` sends the window a message and waits for its thread to
    // answer; from a worker that wait never ends on Windows.
    _: NonSendMarker,
    mut cmd: CommandQueue,
    mut registry: ResMut<PanelRegistry>,
    theme: Res<UITheme>,
    window: Res<window::plugin::Window>,
) {
    window.window_handle.set_title("Rabbithole");
    window
        .window_handle
        .set_min_inner_size(Some(winit::dpi::PhysicalSize::new(900, 600)));

    // The root is the scene's ground. Panels are absolutely positioned over it,
    // so the scene keeps the whole window rather than a centre column.
    let root = cmd
        .spawn((
            UINode {
                width: UIValue::Percent(100.0),
                height: UIValue::Percent(100.0),
                ..Default::default()
            }
            .clipped(),
            UIMaterial::flat(theme.canvas),
        ))
        .entity();

    registry.root = Some(root);

    for region in Region::ALL {
        let panels = registry.in_region(region);
        if panels.is_empty() {
            continue;
        }
        let container = spawn_region(&mut cmd, root, region, &theme);
        for (id, body) in fill_region(&mut cmd, container, region, &panels, &theme) {
            registry.bodies.insert(id, body);
        }
    }
}

/// Places one region over the scene.
///
/// Every region but [`Region::Scene`] is absolutely positioned, so adding or
/// removing one never reflows another — the whole point of an overlay.
fn spawn_region(cmd: &mut CommandQueue, root: Entity, region: Region, theme: &UITheme) -> Entity {
    let rail_top = TOP + MARGIN;
    let node = match region {
        Region::Scene => UINode {
            width: UIValue::Percent(100.0),
            height: UIValue::Percent(100.0),
            position: Position::Absolute,
            ..Default::default()
        },
        // Both top strips size to their own text now.
        Region::Brand => UINode {
            height: UIValue::Px(TOP),
            position: Position::Absolute,
            inset: UIInset {
                top: UIValue::Px(MARGIN),
                left: UIValue::Px(MARGIN),
                right: UIValue::Px(340.0),
                ..Default::default()
            },
            flex_direction: FlexDirection::Row,
            ..Default::default()
        },
        Region::Stats => UINode {
            height: UIValue::Px(TOP),
            position: Position::Absolute,
            inset: UIInset {
                top: UIValue::Px(MARGIN),
                right: UIValue::Px(MARGIN),
                ..Default::default()
            },
            flex_direction: FlexDirection::Row,
            ..Default::default()
        },
        // One rail holding both cards, so flex splits the height between them
        // rather than each guessing an offset the other has to agree with.
        Region::Rail => UINode {
            width: UIValue::Px(RAIL),
            position: Position::Absolute,
            inset: UIInset {
                top: UIValue::Px(rail_top),
                left: UIValue::Px(MARGIN),
                bottom: UIValue::Px(MARGIN),
                ..Default::default()
            },
            flex_direction: FlexDirection::Column,
            gap: glam::Vec2::new(0.0, 10.0),
            ..Default::default()
        },
        Region::Side => UINode {
            width: UIValue::Px(SIDE),
            position: Position::Absolute,
            inset: UIInset {
                top: UIValue::Px(rail_top),
                right: UIValue::Px(MARGIN),
                bottom: UIValue::Px(MARGIN),
                ..Default::default()
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
        Region::Foot => UINode {
            position: Position::Absolute,
            inset: UIInset {
                left: UIValue::Px(RAIL + MARGIN * 2.0),
                right: UIValue::Px(SIDE + MARGIN * 2.0),
                bottom: UIValue::Px(MARGIN),
                ..Default::default()
            },
            flex_direction: FlexDirection::Column,
            gap: glam::Vec2::new(0.0, 8.0),
            ..Default::default()
        },
    };

    let entity = cmd.spawn(node.clipped()).entity();
    cmd.add_child(root, entity);
    let _ = theme;
    entity
}

/// Gives each panel in a region a body to build into, with a card title where
/// the region wants one.
fn fill_region(
    cmd: &mut CommandQueue,
    container: Entity,
    region: Region,
    panels: &[PanelDescriptor],
    theme: &UITheme,
) -> Vec<(&'static str, Entity)> {
    let mut bodies = Vec::with_capacity(panels.len());
    for panel in panels {
        let body = cmd
            .spawn(
                UINode {
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    flex_direction: FlexDirection::Column,
                    padding: if region.is_card() {
                        UIRect::all(theme.spacing_md)
                    } else {
                        UIRect::default()
                    },
                    ..Default::default()
                }
                .clipped(),
            )
            .entity();
        cmd.add_child(container, body);
        if region != Region::Scene {
            cmd.insert(Interactable, body);
        }

        if region.is_card() {
            // Each panel is its own glass card; stacking two in the rail then
            // needs no divider.
            cmd.insert(
                UIMaterial {
                    corner_radius: theme.radius_lg,
                    ..UIMaterial::flat(theme.surface)
                },
                body,
            );
            let title = cmd
                .spawn((
                    UINode {
                        height: UIValue::Px(16.0),
                        flex_shrink: 0.0,
                        ..Default::default()
                    },
                    TextComponent {
                        // Nocturne sets these kickers in caps; the shaper has no
                        // letter-spacing control, so the text carries the case.
                        text: panel.title.to_uppercase(),
                        font_weight: crate::fonts::MEDIUM,
                        font_size: theme.font_size_sm,
                        line_height: theme.line_height(theme.font_size_sm),
                        color: theme.text_muted,
                        ..Default::default()
                    },
                ))
                .entity();
            cmd.add_child(body, title);
        }
        bodies.push((panel.id, body));
    }
    bodies
}

#[cfg(test)]
mod tests {
    use ecs::{IntoSystem, System, World};

    use super::*;

    fn panel(id: &'static str, region: Region) -> PanelDescriptor {
        PanelDescriptor {
            id,
            title: "Panel",
            region,
        }
    }

    #[test]
    fn panels_are_grouped_by_the_region_they_declare() {
        let mut registry = PanelRegistry::default();
        registry.register(panel("a", Region::Rail));
        registry.register(panel("b", Region::Foot));
        registry.register(panel("c", Region::Foot));

        assert_eq!(registry.in_region(Region::Rail).len(), 1);
        assert_eq!(registry.in_region(Region::Foot).len(), 2);
        assert!(
            registry.in_region(Region::Side).is_empty(),
            "an empty region is legal; the dock just leaves it unplaced"
        );
    }

    #[test]
    fn a_duplicate_id_is_rejected_rather_than_shadowing() {
        let mut registry = PanelRegistry::default();
        registry.register(panel("a", Region::Rail));
        registry.register(panel("a", Region::Side));

        assert_eq!(registry.panels().len(), 1);
        assert_eq!(
            registry.panels()[0].region,
            Region::Rail,
            "the first registration wins, so a stray duplicate cannot move a panel"
        );
    }

    #[test]
    fn only_the_rail_and_side_regions_are_cards() {
        assert!(Region::Rail.is_card());
        assert!(Region::Side.is_card());
        assert!(
            !Region::Scene.is_card() && !Region::Foot.is_card() && !Region::Brand.is_card(),
            "chrome must stay bare so the scene shows through it"
        );
    }

    #[derive(Resource, Default)]
    struct Built(Vec<(Region, Entity)>);

    fn build_every_region(mut cmd: CommandQueue, theme: Res<UITheme>, mut built: ResMut<Built>) {
        for region in Region::ALL {
            let container = cmd.spawn(UINode::default()).entity();
            for (_, body) in fill_region(&mut cmd, container, region, &[panel("p", region)], &theme)
            {
                built.0.push((region, body));
            }
        }
    }

    #[test]
    fn panels_over_the_scene_block_the_pointer() {
        let mut world = World::default();
        world.insert_resource(UITheme::default());
        world.insert_resource(Built::default());
        let mut system = build_every_region.into_system();
        system.initialize(&mut world);
        system.run_and_apply(&mut world);

        let built = &world.get_resource::<Built>().unwrap().0;
        assert_eq!(built.len(), Region::ALL.len());
        for (region, body) in built {
            assert_eq!(
                world
                    .get_component_for_entity::<Interactable>(*body)
                    .is_some(),
                *region != Region::Scene,
                "{region:?}: a panel drawn over the scene must be hit before the viewport behind it"
            );
        }
    }

    #[test]
    fn bodies_are_unknown_until_the_dock_has_built() {
        let registry = PanelRegistry::default();
        assert_eq!(registry.body("rabbithole.warren"), None);
    }
}

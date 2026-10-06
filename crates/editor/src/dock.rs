//! The shell's skeleton: named regions overlaid on a full-bleed scene.
use std::collections::HashMap;

use concerto_app::{App, Plugin, schedule_groups::Startup};
use concerto_ecs::{Entity, Res, ResMut, Resource, command::CommandQueue, system::NonSendMarker};
use concerto_ui::{
    elements::prelude::*,
    interaction::Interactable,
    node::{UIInset, UINode},
    theme::UITheme,
    transform::UIValue,
};
use taffy::{FlexDirection, Position};

/// Where a panel sits over the scene.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Region {
    Scene,
    Brand,
    Stats,
    Rail,
    Side,
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

    fn is_card(self) -> bool {
        matches!(self, Region::Rail | Region::Side)
    }
}

/// A panel's declaration.
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

    /// The node every region is placed over, once the dock has run.
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

const MARGIN: f32 = 18.0;
const TOP: f32 = 34.0;
/// Height of the band above the cards.
pub const TOP_STRIP: f32 = TOP + MARGIN;
const RAIL: f32 = 238.0;
const SIDE: f32 = 290.0;

fn build_dock(
    _: NonSendMarker,
    mut cmd: CommandQueue,
    mut registry: ResMut<PanelRegistry>,
    theme: Res<UITheme>,
    window: Res<concerto_window::plugin::Window>,
) {
    window.window_handle.set_title("Concerto");
    window
        .window_handle
        .set_min_inner_size(Some(winit::dpi::PhysicalSize::new(900, 600)));

    let root = cmd
        .spawn(
            theme
                .canvas()
                .size(UIValue::Percent(100.0), UIValue::Percent(100.0))
                .clipped(),
        )
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

fn spawn_region(cmd: &mut CommandQueue, root: Entity, region: Region, theme: &UITheme) -> Entity {
    let rail_top = TOP + MARGIN;
    let node = match region {
        Region::Scene => UINode::default()
            .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .with_position(Position::Absolute),
        Region::Brand => UINode::default()
            .with_height(UIValue::Px(TOP))
            .with_position(Position::Absolute)
            .with_inset(UIInset {
                top: UIValue::Px(MARGIN),
                left: UIValue::Px(MARGIN),
                right: UIValue::Px(340.0),
                ..Default::default()
            })
            .with_flex_direction(FlexDirection::Row),
        Region::Stats => UINode::default()
            .with_height(UIValue::Px(TOP))
            .with_position(Position::Absolute)
            .with_inset(UIInset {
                top: UIValue::Px(MARGIN),
                right: UIValue::Px(MARGIN),
                ..Default::default()
            })
            .with_flex_direction(FlexDirection::Row),
        Region::Rail => UINode::default()
            .with_width(UIValue::Px(RAIL))
            .with_position(Position::Absolute)
            .with_inset(UIInset {
                top: UIValue::Px(rail_top),
                left: UIValue::Px(MARGIN),
                bottom: UIValue::Px(MARGIN),
                ..Default::default()
            })
            .with_flex_direction(FlexDirection::Column)
            .with_gap(glam::Vec2::new(0.0, 10.0)),
        Region::Side => UINode::default()
            .with_width(UIValue::Px(SIDE))
            .with_position(Position::Absolute)
            .with_inset(UIInset {
                top: UIValue::Px(rail_top),
                right: UIValue::Px(MARGIN),
                bottom: UIValue::Px(MARGIN),
                ..Default::default()
            })
            .with_flex_direction(FlexDirection::Column),
        Region::Foot => UINode::default()
            .with_position(Position::Absolute)
            .with_inset(UIInset {
                left: UIValue::Px(RAIL + MARGIN * 2.0),
                right: UIValue::Px(SIDE + MARGIN * 2.0),
                bottom: UIValue::Px(MARGIN),
                ..Default::default()
            })
            .with_flex_direction(FlexDirection::Column)
            .with_gap(glam::Vec2::new(0.0, 8.0)),
    };

    let entity = cmd.entity(root).spawn_child_queue(node.clipped()).entity();
    let _ = theme;
    entity
}

fn fill_region(
    cmd: &mut CommandQueue,
    container: Entity,
    region: Region,
    panels: &[PanelDescriptor],
    theme: &UITheme,
) -> Vec<(&'static str, Entity)> {
    let mut bodies = Vec::with_capacity(panels.len());
    for panel in panels {
        let mut container_queue = cmd.entity(container);
        let mut body_queue = if region.is_card() {
            container_queue.spawn_child_queue(
                theme
                    .panel()
                    .grow()
                    .column()
                    .padding(theme.spacing_md)
                    .clipped(),
            )
        } else {
            container_queue.spawn_child_queue(
                UINode::default()
                    .with_flex_grow(1.0)
                    .with_flex_direction(FlexDirection::Column)
                    .clipped(),
            )
        };
        let body = body_queue.entity();
        if region != Region::Scene {
            body_queue.insert(Interactable);
        }
        if region.is_card() {
            body_queue.add_child(
                theme
                    .label(panel.title.to_uppercase())
                    .small()
                    .muted()
                    .weight(crate::fonts::MEDIUM)
                    .height(UIValue::Px(16.0))
                    .fixed(),
            );
        }
        bodies.push((panel.id, body));
    }
    bodies
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{IntoSystem, System, World};

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
        system.run_and_apply((), &mut world);

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
        assert_eq!(registry.body("concerto.warren"), None);
    }
}

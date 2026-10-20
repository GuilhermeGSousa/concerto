use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::Children},
    query::{Query, filter::With},
    resource::Res,
    signal::{On, lifecycle::Add},
};

use crate::{
    items::{
        UIItem, UIRowSpacer, UIWidgetPart, fit_owner, keep_parts_last, place_rows, set_up_owner,
    },
    node::{UICulled, UIIntrinsicHeight, UILayout, UINode},
    scroll::UIScrollArea,
    theme::UITheme,
};

/// A vertical list whose items are the node's own children, one row each, in child order; it clips them and owns its overflow.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct UIList {
    pub row_height: Option<f32>,
    pub selectable: bool,
}

impl Default for UIList {
    fn default() -> Self {
        Self {
            row_height: None,
            selectable: true,
        }
    }
}

pub(crate) fn set_up_list(
    on: On<Add<UIList>>,
    areas: Query<&UIScrollArea>,
    children: Query<&Children>,
    spacers: Query<&UIRowSpacer>,
    mut cmd: CommandQueue,
) {
    set_up_owner(on.signal().entity, &areas, &children, &spacers, &mut cmd);
}

pub(crate) fn update_lists(
    lists: Query<(
        Entity,
        &UIList,
        &mut UIScrollArea,
        Option<&UILayout>,
        Option<&mut Children>,
    )>,
    nodes: Query<&mut UINode>,
    parts: Query<&UIWidgetPart>,
    spacers: Query<&mut UIIntrinsicHeight, With<UIRowSpacer>>,
    items: Query<&UIItem>,
    culled: Query<&UICulled>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, list, mut area, layout, children) in lists.iter() {
        let mut rows = Vec::new();
        let mut spacer = None;
        if let Some(mut children) = children {
            keep_parts_last(&mut children, &parts);
            for &child in children.iter() {
                if !parts.contains_entity(child) && nodes.contains_entity(child) {
                    rows.push(child);
                } else if spacer.is_none() {
                    spacer = spacers.get_entity(child);
                }
            }
        }
        place_rows(
            &rows,
            entity,
            list.row_height.unwrap_or(theme.row_height),
            layout.map(|layout| layout.content_rect.size.y),
            &mut area,
            &nodes,
            &items,
            &culled,
            &mut cmd,
        );
        fit_owner(entity, spacer, area.content_extent, &nodes);
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        IntoSetConfig, IntoSystemConfig, Schedule, World,
        entity::{
            Entity,
            hierarchy::{ChildOf, Children},
        },
        system::{executor::multi_thread::MultiThreadedExecutor, schedule::CompiledSchedule},
    };
    use concerto_window::plugin::Window;

    use super::*;
    use crate::{
        anchor::UIPanelStack,
        interaction::{HoveredNode, Interactable, UIInputState, update_ui_interaction},
        items::UIItem,
        node::{
            FlexDirection, UICulled, UILayout, UILayoutDiagnostics, UILayoutEngine, UINode,
            UITextMeasure, blank_culled_layouts, compute_ui_nodes,
        },
        scroll::{
            UIScrollArea, UIScrollBar, UIScrollThumb, setup_scrollbars, sync_scrollbar_thumbs,
            sync_scrollbar_tracks,
        },
        sets::UiSet,
        text::fonts::{UIFonts, build_font_system},
        theme::UITheme,
        transform::UIValue,
    };

    struct Harness {
        world: World,
        schedule: CompiledSchedule,
    }

    impl Harness {
        fn new() -> Self {
            let mut world = World::new();
            world.insert_resource(Window::new((800, 600), 1.0));
            world.insert_resource(UITheme::default());
            world.insert_resource(UILayoutEngine::default());
            world.insert_resource(UILayoutDiagnostics::default());
            world.insert_resource(UIPanelStack::default());
            world.insert_resource(UITextMeasure::new(build_font_system(&UIFonts::default())));
            world.add_listener(blank_culled_layouts);
            world.add_listener(set_up_list);
            let mut schedule = Schedule::new();
            schedule.configure_sets(
                (
                    UiSet::Setup,
                    UiSet::Project,
                    UiSet::Layout,
                    UiSet::PostLayout,
                )
                    .chain(),
            );
            schedule.add_system(setup_scrollbars.in_set(UiSet::Setup));
            schedule.add_system(update_lists.in_set(UiSet::Project));
            schedule.add_system(compute_ui_nodes.in_set(UiSet::Layout));
            schedule.add_system(
                (sync_scrollbar_tracks, sync_scrollbar_thumbs).in_set(UiSet::PostLayout),
            );
            let schedule = schedule.compile::<MultiThreadedExecutor>(&mut world);
            Self { world, schedule }
        }

        fn frame(&mut self) {
            self.schedule.run(&mut self.world);
            self.world.tick();
        }

        fn settle(&mut self) {
            for _ in 0..3 {
                self.frame();
            }
        }

        fn root(&mut self) -> Entity {
            self.world.spawn(
                UINode::default()
                    .with_size(UIValue::Px(800.0), UIValue::Px(600.0))
                    .with_flex_direction(FlexDirection::Column),
            )
        }

        fn node(&mut self, node: UINode, parent: Entity) -> Entity {
            self.world.spawn((node, ChildOf::new(parent)))
        }

        fn list(&mut self, list: UIList, node: UINode, parent: Entity) -> Entity {
            self.world.spawn((node, list, ChildOf::new(parent)))
        }

        fn rows(&mut self, list: Entity, count: usize) -> Vec<Entity> {
            (0..count)
                .map(|_| self.node(UINode::default(), list))
                .collect()
        }

        fn layout(&self, entity: Entity) -> &UILayout {
            self.world
                .get_component_for_entity::<UILayout>(entity)
                .unwrap_or_else(|| panic!("{entity:?} has no layout"))
        }

        fn area(&self, list: Entity) -> &UIScrollArea {
            self.world
                .get_component_for_entity::<UIScrollArea>(list)
                .expect("a list is its own scroll area")
        }

        fn set_offset(&mut self, list: Entity, offset: f32) {
            self.world
                .get_component_for_entity_mut::<UIScrollArea>(list)
                .unwrap()
                .offset = offset;
        }

        fn unculled(&self, rows: &[Entity]) -> Vec<usize> {
            rows.iter()
                .enumerate()
                .filter(|(_, row)| {
                    self.world
                        .get_component_for_entity::<UICulled>(**row)
                        .is_none()
                })
                .map(|(index, _)| index)
                .collect()
        }
    }

    fn rows_of(row_height: f32) -> UIList {
        UIList {
            row_height: Some(row_height),
            ..Default::default()
        }
    }

    fn tall(height: f32) -> UINode {
        UINode::default()
            .with_height(UIValue::Px(height))
            .with_flex_shrink(0.0)
    }

    fn long_list(harness: &mut Harness, offset: f32) -> (Entity, Vec<Entity>) {
        let root = harness.root();
        let list = harness.world.spawn((
            tall(100.0),
            rows_of(20.0),
            UIScrollArea {
                offset,
                ..Default::default()
            },
            ChildOf::new(root),
        ));
        let rows = harness.rows(list, 1_000);
        harness.settle();
        (list, rows)
    }

    #[test]
    fn children_are_rows_in_child_order() {
        let mut harness = Harness::new();
        let root = harness.root();
        harness.node(tall(30.0), root);
        let list = harness.list(rows_of(20.0), tall(200.0), root);
        let rows = harness.rows(list, 5);

        harness.settle();

        let top = harness.layout(list).rect.min.y;
        assert_eq!(top, 30.0);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                harness.layout(*row).rect.min.y,
                top + 20.0 * index as f32,
                "row {index}"
            );
            assert_eq!(harness.layout(*row).rect.size.y, 20.0);
        }
    }

    #[test]
    fn the_extent_comes_from_the_children() {
        let mut harness = Harness::new();
        let root = harness.root();
        let list = harness.list(UIList::default(), tall(200.0), root);
        let rows = harness.rows(list, 50);

        harness.settle();
        assert_eq!(harness.area(list).content_extent, 1300.0);

        for row in &rows[..10] {
            harness.world.despawn(*row);
        }
        harness.settle();
        assert_eq!(harness.area(list).content_extent, 1040.0);
    }

    #[test]
    fn the_scrollbar_is_not_a_row() {
        let mut harness = Harness::new();
        let root = harness.root();
        let list = harness.list(UIList::default(), tall(200.0), root);
        let rows = harness.rows(list, 5);

        harness.settle();

        let children = harness
            .world
            .get_component_for_entity::<Children>(list)
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let track = children
            .iter()
            .copied()
            .find(|child| {
                harness
                    .world
                    .get_component_for_entity::<UIScrollBar>(*child)
                    .is_some()
            })
            .expect("the list has a scrollbar track");
        assert!(
            harness
                .world
                .get_component_for_entity::<UIItem>(track)
                .is_none()
        );
        for (index, row) in rows.iter().enumerate() {
            let item = harness
                .world
                .get_component_for_entity::<UIItem>(*row)
                .expect("every child is an item");
            assert_eq!((item.owner(), item.index(), item.depth()), (list, index, 0));
        }
        assert_eq!(harness.area(list).content_extent, 5.0 * 26.0);
    }

    #[test]
    fn rows_far_off_screen_are_culled() {
        let mut harness = Harness::new();
        let (_, rows) = long_list(&mut harness, 400.0);

        assert_eq!(harness.unculled(&rows), (18..=26).collect::<Vec<_>>());
    }

    #[test]
    fn scrolling_moves_the_unculled_window() {
        let mut harness = Harness::new();
        let (list, rows) = long_list(&mut harness, 400.0);

        harness.set_offset(list, 0.0);
        harness.settle();

        assert_eq!(harness.unculled(&rows), (0..=6).collect::<Vec<_>>());
    }

    #[test]
    fn a_row_scrolled_out_is_not_shown_after_one_frame() {
        let mut harness = Harness::new();
        let (list, rows) = long_list(&mut harness, 0.0);
        assert!(harness.layout(rows[0]).is_shown());

        harness.set_offset(list, 400.0);
        harness.frame();

        assert!(!harness.layout(rows[0]).is_shown());
        assert!(harness.layout(rows[20]).is_shown());
        assert_eq!(
            harness.layout(rows[20]).rect.min.y,
            harness.layout(list).rect.min.y
        );
    }

    #[test]
    fn a_list_in_a_grown_column_does_not_take_its_height_from_its_rows() {
        let mut harness = Harness::new();
        let rail = harness.world.spawn(
            UINode::default()
                .with_size(UIValue::Px(238.0), UIValue::Px(600.0))
                .with_flex_direction(FlexDirection::Column),
        );
        let mut cards = Vec::new();
        let mut lists = Vec::new();
        for _ in 0..2 {
            let card = harness.node(
                UINode::default()
                    .with_flex_grow(1.0)
                    .with_flex_shrink(1.0)
                    .with_flex_direction(FlexDirection::Column),
                rail,
            );
            let list = harness.list(
                rows_of(32.0),
                UINode::default()
                    .with_flex_grow(1.0)
                    .with_flex_direction(FlexDirection::Column),
                card,
            );
            harness.rows(list, 60);
            cards.push(card);
            lists.push(list);
        }

        harness.settle();

        for (card, list) in cards.into_iter().zip(lists) {
            assert_eq!(harness.layout(card).rect.size.y, 300.0);
            assert_eq!(harness.layout(list).rect.size.y, 300.0);
        }
    }

    #[test]
    fn a_list_with_auto_height_grows_up_to_max_height() {
        let mut harness = Harness::new();
        let root = harness.root();
        let list = harness.list(
            rows_of(30.0),
            UINode::default().with_max_height(UIValue::Px(300.0)),
            root,
        );
        harness.rows(list, 3);

        harness.settle();
        assert_eq!(harness.layout(list).rect.size.y, 90.0);

        harness.rows(list, 17);
        harness.settle();
        assert_eq!(harness.layout(list).rect.size.y, 300.0);
    }

    #[test]
    fn a_growing_list_in_an_auto_height_column_is_as_tall_as_its_rows() {
        let mut harness = Harness::new();
        let root = harness.root();
        let column = harness.node(
            UINode::default().with_flex_direction(FlexDirection::Column),
            root,
        );
        let list = harness.list(rows_of(20.0), UINode::default().with_flex_grow(1.0), column);
        harness.rows(list, 5);

        harness.settle();

        assert_eq!(harness.layout(list).rect.size.y, 100.0);
        assert_eq!(harness.layout(column).rect.size.y, 100.0);
    }

    #[test]
    fn a_growing_list_in_a_row_does_not_push_its_card_to_the_rows_height() {
        let mut harness = Harness::new();
        let root = harness.root();
        let card = harness.node(
            UINode::default()
                .with_height(UIValue::Px(200.0))
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Column),
            root,
        );
        let row = harness.node(
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_direction(FlexDirection::Row),
            card,
        );
        let list = harness.list(rows_of(32.0), UINode::default().with_flex_grow(1.0), row);
        harness.rows(list, 60);

        harness.settle();

        assert_eq!(harness.layout(row).rect.size.y, 200.0);
        assert_eq!(harness.layout(list).rect.size.y, 200.0);
    }

    #[test]
    fn re_adding_the_list_keeps_one_spacer() {
        let mut harness = Harness::new();
        let root = harness.root();
        let list = harness.list(UIList::default(), tall(100.0), root);
        harness.world.insert(UIList::default(), list);

        let spacers = harness
            .world
            .get_component_for_entity::<Children>(list)
            .unwrap()
            .iter()
            .filter(|child| {
                harness
                    .world
                    .get_component_for_entity::<crate::items::UIRowSpacer>(**child)
                    .is_some()
            })
            .count();
        assert_eq!(spacers, 1);
    }

    #[test]
    fn rows_added_later_stay_under_the_scrollbar() {
        let mut harness = Harness::new();
        harness
            .world
            .insert_resource(concerto_window::input::Input::new());
        harness.world.insert_resource(HoveredNode::default());
        harness.world.insert_resource(UIInputState::default());
        let root = harness.root();
        let list = harness.list(rows_of(20.0), tall(100.0), root);
        for _ in 0..10 {
            harness.node(UINode::default(), list);
        }
        harness.settle();
        for _ in 0..10 {
            harness
                .world
                .spawn((UINode::default(), Interactable, ChildOf::new(list)));
        }
        harness.set_offset(list, 300.0);
        harness.settle();

        let thumb = harness
            .world
            .get_component_for_entity::<Children>(list)
            .unwrap()
            .iter()
            .find_map(|child| {
                harness
                    .world
                    .get_component_for_entity::<Children>(*child)?
                    .iter()
                    .copied()
                    .find(|grandchild| {
                        harness
                            .world
                            .get_component_for_entity::<UIScrollThumb>(*grandchild)
                            .is_some()
                    })
            })
            .expect("the list has a scrollbar thumb");
        let rect = harness.layout(thumb).rect;
        let centre = rect.min + rect.size / 2.0;
        harness
            .world
            .get_resource_mut::<concerto_window::input::Input>()
            .unwrap()
            .update_mouse_position(centre.x as f64, centre.y as f64);
        let mut system = concerto_ecs::IntoSystem::into_system(update_ui_interaction);
        concerto_ecs::System::initialize(&mut system, &mut harness.world);
        concerto_ecs::System::run_and_apply(&mut system, (), &mut harness.world);

        assert_eq!(
            **harness.world.get_resource::<HoveredNode>().unwrap(),
            Some(thumb)
        );
    }
}

use std::collections::{HashMap, HashSet};

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::{CommandQueue, EntityCommandQueue},
    component::{name::Name, scene::SceneEntityRef},
    entity::hierarchy::{ChildOf, Children},
    events::event_reader::EventReader,
    query::filter::{With, Without},
    signal::{
        On,
        lifecycle::Remove,
        listener::{GlobalListener, IntoListener, Listener},
    },
};
use concerto_ui::elements::{Listening, Pressable, Text, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::UIClick,
    node::{UINode, UIRect},
    scroll::UIScrollArea,
    text::UIText,
    text_input::UITextInputChanged,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use crate::asset_editor::ActiveEditor;
use crate::asset_picker::{
    ITEM_HEIGHT, LIST_MAX_HEIGHT, MENU_WIDTH, SCROLLBAR_GUTTER, menu_panel, picker_button,
    picker_caret, picker_label,
};
use crate::fonts::{glyph, icon};
use crate::hierarchy::{HierarchyState, ListRoot, ListedEntity, listed_entities};
use crate::hosted::{EditTarget, EditWorld, EditWorldSignals, PlayState};
use crate::inspector::{
    EditContext, EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};
use crate::tools::{scene::SceneRoot, selection::Select};

const NOT_IN_SCENE: &str = "(not in this scene)";
const INDENT: f32 = 14.0;

/// Picks the entity a [`SceneEntityRef`] names from the entities of the scene being edited.
#[derive(Default)]
pub struct EntityPicker;

/// Keeps every entity picker row naming the entity it holds, and runs the menu the rows share.
pub struct EntityPickerPlugin;

impl Plugin for EntityPickerPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, refresh_entity_picker_labels)
            .add_system(LateUpdate, filter_entity_picker)
            .add_listener(forget_scene);
    }
}

/// The text inside an [`EntityPicker`] row's button, naming the row's entity.
#[derive(Component)]
pub(crate) struct EntityPickerLabel {
    row: Entity,
}

pub(crate) fn register_defaults(registry: &mut crate::inspector::InspectorRegistry) {
    registry.register_property_editor::<SceneEntityRef, EntityPicker>();
}

impl PropertyEditor<SceneEntityRef> for EntityPicker {
    type Snapshot = SceneEntityRef;
    type Edit = SceneEntityRef;

    fn snapshot(&self, value: &SceneEntityRef) -> SceneEntityRef {
        *value
    }

    fn build(&self, row: &mut EntityCommandQueue, _: &SceneEntityRef, theme: &UITheme) {
        let entity = row.entity();
        row.add_child_with(
            picker_button(theme).on_click(toggle_entity_picker(entity)),
            |mut button| {
                button
                    .add_child((picker_label(theme), EntityPickerLabel { row: entity }))
                    .add_child(picker_caret(theme));
            },
        )
        .add_child(select_button(theme, entity));
    }

    fn apply(
        &self,
        value: &mut SceneEntityRef,
        edit: &SceneEntityRef,
        _: &EditContext,
    ) -> Result<(), EditError> {
        if edit.entity().is_none() && *edit != SceneEntityRef::default() {
            return Err(EditError::Rejected);
        }
        *value = *edit;
        Ok(())
    }

    fn edit_to(&self, value: &SceneEntityRef) -> Option<SceneEntityRef> {
        Some(*value)
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

fn select_button(theme: &UITheme, row: Entity) -> (Listening<Pressable, UIClick>, Text) {
    let size = theme.font_size_md;
    let vertical = (theme.control_height - theme.line_height(size)) / 2.0;
    let horizontal = (theme.control_height - size) / 2.0;
    (
        theme
            .pressable()
            .size(
                UIValue::Px(theme.control_height),
                UIValue::Px(theme.control_height),
            )
            .padding(UIRect::axes(vertical, horizontal))
            .fixed()
            .on_click(select_referenced(row)),
        icon(theme, glyph::CROSSHAIR, size).muted(),
    )
}

/// The menu every entity picker shares, holding the text its filter field narrows the list by.
#[derive(Component)]
pub struct EntityPickerMenu {
    filter: String,
}

#[derive(Component)]
struct EntityPickerFilter;

#[derive(Component)]
struct EntityPickerView;

#[derive(Component, Clone)]
struct EntityPickerItem {
    entity: Option<Entity>,
    parent: Option<Entity>,
    label: String,
}

fn held(values: &Query<&PropertyRowValue>, row: Entity) -> Option<SceneEntityRef> {
    values
        .get_entity(row)
        .and_then(|value| value.downcast::<SceneEntityRef>().copied())
}

fn toggle_entity_picker(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           menus: Query<(Entity, &mut EntityPickerMenu, &mut UIAnchoredPanel)>,
           roots: EditWorld<Query<(Entity, &SceneRoot)>>,
           top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
           children: EditWorld<Query<&Children>>,
           names: EditWorld<Query<Option<&Name>>>,
           active: Res<ActiveEditor>,
           states: Query<&PlayState>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let button = on.entity();
        let mut menu = match menus.iter().next() {
            Some((entity, mut menu, mut panel)) => {
                if panel.open && panel.owner == Some(button) {
                    panel.open = false;
                    return;
                }
                if !menu.filter.is_empty() {
                    menu.filter.clear();
                }
                **panel = menu_panel(button);
                cmd.entity(entity).despawn_children()
            }
            None => cmd.spawn((
                theme
                    .popup()
                    .width(UIValue::Px(MENU_WIDTH))
                    .padding(theme.spacing_xs)
                    .gap(theme.spacing_xs),
                menu_panel(button),
                EntityPickerMenu {
                    filter: String::new(),
                },
            )),
        };
        let candidates = listed_entities(
            &roots,
            &top_level,
            &children,
            &active,
            &states,
            ListRoot::Exclude,
        );
        let current = held(&values, row).unwrap_or_default();
        fill_menu(&mut menu, row, current, &candidates, &names, &theme);
    })
    .into_listener()
}

fn fill_menu(
    menu: &mut EntityCommandQueue,
    row: Entity,
    current: SceneEntityRef,
    candidates: &[ListedEntity],
    names: &Query<Option<&Name>>,
    theme: &UITheme,
) {
    menu.add_child((theme.text_field("Search entities…"), EntityPickerFilter));
    let items: Vec<EntityPickerItem> = std::iter::once(EntityPickerItem {
        entity: None,
        parent: None,
        label: "None".into(),
    })
    .chain(candidates.iter().map(|listed| EntityPickerItem {
        entity: Some(listed.entity),
        parent: listed.parent,
        label: entity_label(listed.entity, names.get_entity(listed.entity).flatten()),
    }))
    .collect();
    let depths: Vec<usize> = std::iter::once(0)
        .chain(candidates.iter().map(|listed| listed.depth))
        .collect();
    let extent = items.len() as f32 * ITEM_HEIGHT;
    menu.add_child_with(
        (
            theme
                .column()
                .fixed()
                .height(UIValue::Px(extent.min(LIST_MAX_HEIGHT)))
                .clipped(),
            EntityPickerView,
        ),
        |mut view| {
            let mut list = view.spawn_child(theme.column().gap(0.0).padding(UIRect {
                right: SCROLLBAR_GUTTER,
                ..UIRect::all(0.0)
            }));
            for (item, depth) in items.iter().zip(&depths) {
                let value = item
                    .entity
                    .map_or_else(SceneEntityRef::default, SceneEntityRef::Entity);
                let label = item.label.clone();
                list.add_child_with(
                    (
                        theme
                            .pressable()
                            .selected(current == value)
                            .row()
                            .fixed()
                            .height(UIValue::Px(ITEM_HEIGHT))
                            .padding(UIRect {
                                left: 8.0 + *depth as f32 * INDENT,
                                right: 8.0,
                                ..UIRect::all(0.0)
                            })
                            .on_click(pick_entity(row, value)),
                        item.clone(),
                    ),
                    |mut item| {
                        item.add_child(
                            theme
                                .label(label.clone())
                                .font_size(12.5)
                                .single_line()
                                .grow()
                                .shrink(1.0)
                                .min_width(UIValue::Px(0.0)),
                        );
                    },
                );
            }
            let list = list.entity();
            view.insert(UIScrollArea {
                content: Some(list),
                content_extent: extent,
                ..Default::default()
            });
        },
    );
}

fn pick_entity(row: Entity, value: SceneEntityRef) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<EntityPickerMenu>>,
           rows: Query<(&PropertyRow, &PropertyRowValue)>,
           mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        if let Some((property, held)) = rows.get_entity(row)
            && held.downcast::<SceneEntityRef>() != Some(&value)
            && let Err(error) = commits.push::<SceneEntityRef, EntityPicker>(property, value)
        {
            log::warn!("Entity pick dropped: {error}");
        }
        for mut panel in menus.iter() {
            panel.open = false;
        }
    })
    .into_listener()
}

fn select_referenced(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           roots: EditWorld<Query<(Entity, &SceneRoot)>>,
           top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
           children: EditWorld<Query<&Children>>,
           active: Res<ActiveEditor>,
           states: Query<&PlayState>,
           mut hierarchy: ResMut<HierarchyState>,
           mut scene: EditWorldSignals| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(entity) = held(&values, row).and_then(|value| value.entity()) else {
            return;
        };
        let listed = listed_entities(
            &roots,
            &top_level,
            &children,
            &active,
            &states,
            ListRoot::Exclude,
        )
        .iter()
        .any(|listed| listed.entity == entity);
        if listed {
            scene.trigger(Select(Some(entity)));
            hierarchy.reveal_entity(entity);
        }
    })
    .into_listener()
}

fn filter_entity_picker(
    mut events: EventReader<UITextInputChanged>,
    filters: Query<&EntityPickerFilter>,
    menus: Query<&mut EntityPickerMenu>,
    items: Query<(&EntityPickerItem, &mut UINode)>,
    views: Query<
        (&mut UIScrollArea, &mut UINode),
        (With<EntityPickerView>, Without<EntityPickerItem>),
    >,
) {
    let Some(text) = events
        .read()
        .filter(|event| filters.get_entity(event.entity).is_some())
        .last()
        .map(|event| event.value.clone())
    else {
        return;
    };
    for mut menu in menus.iter() {
        if menu.filter != text {
            menu.filter = text.clone();
        }
    }
    let filter = text.to_lowercase();
    let parents: HashMap<Entity, Option<Entity>> = items
        .iter()
        .filter_map(|(item, _)| item.entity.map(|entity| (entity, item.parent)))
        .collect();
    let mut shown = HashSet::new();
    for (item, _) in items.iter() {
        let Some(entity) = item.entity else {
            continue;
        };
        if !item.label.to_lowercase().contains(&filter) {
            continue;
        }
        let mut next = Some(entity);
        while let Some(entity) = next {
            if !shown.insert(entity) {
                break;
            }
            next = parents.get(&entity).copied().flatten();
        }
    }
    let mut count = 0;
    for (item, mut node) in items.iter() {
        let visible = item.entity.is_none_or(|entity| shown.contains(&entity));
        count += usize::from(visible);
        if node.visible != visible {
            node.visible = visible;
        }
    }
    let extent = count as f32 * ITEM_HEIGHT;
    let height = UIValue::Px(extent.min(LIST_MAX_HEIGHT));
    for (mut area, mut node) in views.iter() {
        if area.content_extent != extent || area.offset != 0.0 {
            area.content_extent = extent;
            area.offset = 0.0;
        }
        if node.height != height {
            node.height = height;
        }
    }
}

fn forget_scene(
    _: On<Remove<EditTarget>>,
    menus: Query<&mut UIAnchoredPanel, With<EntityPickerMenu>>,
) {
    for mut panel in menus.iter() {
        if panel.open {
            panel.open = false;
        }
    }
}

fn entity_label(entity: Entity, name: Option<&Name>) -> String {
    match name {
        Some(name) if !name.as_str().is_empty() => name.as_str().to_owned(),
        _ => format!("Entity {}", entity.index()),
    }
}

/// Writes the name of each picker row's entity into its label.
pub(crate) fn refresh_entity_picker_labels(
    values: Query<&PropertyRowValue>,
    labels: Query<(&EntityPickerLabel, &mut UIText)>,
    names: EditWorld<Query<Option<&Name>>>,
    roots: EditWorld<Query<(Entity, &SceneRoot)>>,
    top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
    children: EditWorld<Query<&Children>>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
) {
    if labels.iter().next().is_none() {
        return;
    }
    let listed: HashSet<Entity> = listed_entities(
        &roots,
        &top_level,
        &children,
        &active,
        &states,
        ListRoot::Exclude,
    )
    .into_iter()
    .map(|listed| listed.entity)
    .collect();
    for (label, mut text) in labels.iter() {
        let Some(reference) = held(&values, label.row) else {
            continue;
        };
        let wanted = match reference.entity() {
            Some(entity) if listed.contains(&entity) => {
                entity_label(entity, names.get_entity(entity).flatten())
            }
            _ => NOT_IN_SCENE.to_owned(),
        };
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hierarchy::HierarchyState,
        hosted::testing::SceneFixture,
        inspector::{InspectorRegistry, PropertyRow, row_systems, spawn_panel},
        tools::selection::Select,
    };
    use concerto_ecs::{
        IntoSystem, System, World, command::CommandQueue, component::bundle::IntoBundle,
        events::event_channel::EventChannel, reflect::ReflectComponent, system::BoxedSystem,
    };
    use concerto_foundation::{
        assets::{asset_server::AssetServer, registry::AssetRegistry},
        transform::Transform,
    };
    use concerto_reflect::{PathSegment, PropertyPath, Reflect, ReflectDefault};
    use concerto_ui::{
        anchor::UIAnchoredPanel,
        interaction::UIInteractionStyle,
        node::UINode,
        scroll::UIScrollArea,
        text_input::{UITextInput, UITextInputChanged},
    };
    use concerto_window::input::MouseButton;
    use std::any::TypeId;

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct Links {
        target: SceneEntityRef,
        maybe: Option<SceneEntityRef>,
        many: Vec<SceneEntityRef>,
    }

    struct Picking {
        f: SceneFixture,
        document: Entity,
        holder: Entity,
        capsule: Entity,
        skeleton: Entity,
        hips: Entity,
        unnamed: Entity,
        blank: Entity,
        root: Entity,
        systems: Vec<BoxedSystem>,
    }

    fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
        let parent = cmd.spawn(UINode::default()).entity();
        spawn_panel(&mut cmd, parent, &theme);
    }

    fn spawn_under(scene: &mut World, root: Entity, bundle: impl IntoBundle) -> Entity {
        let entity = scene.spawn(bundle);
        scene.entity_mut(root).add_child(entity);
        entity
    }

    fn picking(links: impl Fn(&Picking) -> Links) -> Picking {
        let mut f = SceneFixture::new();
        f.world.insert_resource(InspectorRegistry::default());
        let document = f.open();
        let root = f.scene_root(document);
        let scene = f.scene_world_mut(document);
        scene.register_type::<Links>();
        let capsule = spawn_under(scene, root, (Transform::IDENTITY, Name::new("capsule")));
        let unnamed = spawn_under(scene, root, Transform::IDENTITY);
        let blank = spawn_under(scene, root, (Transform::IDENTITY, Name::new("")));
        let holder = spawn_under(scene, root, (Transform::IDENTITY, Name::new("holder")));
        let skeleton = spawn_under(scene, capsule, (Transform::IDENTITY, Name::new("skeleton")));
        let hips = spawn_under(scene, skeleton, (Transform::IDENTITY, Name::new("Hips")));
        f.world.add_listener(forget_scene);
        f.world
            .insert_resource(EventChannel::<UITextInputChanged>::new());

        let mut panel = build_inspector.into_system();
        panel.initialize(&mut f.world);
        panel.run_and_apply((), &mut f.world);
        let mut systems = row_systems();
        systems.push(refresh_entity_picker_labels.into_boxed_system());
        systems.push(filter_entity_picker.into_boxed_system());
        for system in &mut systems {
            system.initialize(&mut f.world);
        }
        let mut p = Picking {
            f,
            document,
            holder,
            capsule,
            skeleton,
            hips,
            unnamed,
            blank,
            root,
            systems,
        };
        let links = links(&p);
        let scene = p.scene();
        scene.insert(links, holder);
        scene.trigger(Select(Some(holder)));
        p.update();
        p.update();
        p
    }

    impl Picking {
        fn scene(&mut self) -> &mut World {
            self.f.scene_world_mut(self.document)
        }
        fn update(&mut self) {
            for system in &mut self.systems {
                system.run_and_apply((), &mut self.f.world);
            }
            self.f.world.tick();
            self.scene().tick();
        }
        fn set(&mut self, target: SceneEntityRef) {
            let holder = self.holder;
            self.scene()
                .get_component_for_entity_mut::<Links>(holder)
                .unwrap()
                .target = target;
            self.update();
            self.update();
        }
        fn row(&mut self, path: &[PathSegment]) -> Option<(Entity, PropertyRow)> {
            let path = PropertyPath::new(path.iter().copied());
            self.f
                .world
                .query::<(Entity, &PropertyRow), ()>()
                .iter(&mut self.f.world)
                .find(|(_, row)| row.address.path == path)
                .map(|(entity, row)| (entity, row.clone()))
        }
        fn label(&mut self, path: &[PathSegment]) -> String {
            let (row, _) = self.row(path).expect("the inspector built the row");
            self.f
                .world
                .query::<(&EntityPickerLabel, &UIText), ()>()
                .iter(&mut self.f.world)
                .find(|(label, _)| label.row == row)
                .map(|(_, text)| text.text.clone())
                .expect("the row has a picker label")
        }
        fn target(&mut self) -> SceneEntityRef {
            let holder = self.holder;
            self.scene()
                .get_component_for_entity::<Links>(holder)
                .unwrap()
                .target
        }
        fn children(&self, entity: Entity) -> Vec<Entity> {
            self.f
                .world
                .get_component_for_entity::<Children>(entity)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }
        fn text(&self, entity: Entity) -> String {
            self.f
                .world
                .get_component_for_entity::<UIText>(entity)
                .unwrap()
                .text
                .clone()
        }
        fn click(&mut self, entity: Entity) {
            self.f.world.trigger_on(
                entity,
                UIClick {
                    position: Default::default(),
                    button: MouseButton::Left,
                },
            );
        }
        fn button(&mut self) -> Entity {
            let (row, _) = self.row(&[TARGET]).expect("the inspector built the row");
            let label = self
                .f
                .world
                .query::<(Entity, &EntityPickerLabel), ()>()
                .iter(&mut self.f.world)
                .find(|(_, label)| label.row == row)
                .map(|(entity, _)| entity)
                .expect("the row has a picker label");
            self.f
                .world
                .get_component_for_entity::<ChildOf>(label)
                .unwrap()
                .parent()
        }
        fn select_button(&mut self) -> Entity {
            let (row, _) = self.row(&[TARGET]).expect("the inspector built the row");
            self.children(row)
                .into_iter()
                .find(|&child| {
                    self.f
                        .world
                        .get_component_for_entity::<UIText>(child)
                        .is_some_and(|text| text.text == glyph::CROSSHAIR.to_string())
                })
                .expect("the row has a select button")
        }
        fn menu(&mut self) -> Option<Entity> {
            self.f
                .world
                .query::<(Entity, &EntityPickerMenu), ()>()
                .iter(&mut self.f.world)
                .map(|(entity, _)| entity)
                .next()
        }
        fn menu_open(&mut self) -> bool {
            self.f
                .world
                .query::<(&EntityPickerMenu, &UIAnchoredPanel), ()>()
                .iter(&mut self.f.world)
                .any(|(_, panel)| panel.open)
        }
        fn filter_field(&mut self) -> Entity {
            let menu = self.menu().expect("the menu was built");
            self.children(menu)
                .into_iter()
                .find(|&child| {
                    self.f
                        .world
                        .get_component_for_entity::<UITextInput>(child)
                        .is_some()
                })
                .expect("the menu has a filter field")
        }
        fn type_filter(&mut self, value: &str) {
            let field = self.filter_field();
            let channel = self
                .f
                .world
                .get_resource_mut::<EventChannel<UITextInputChanged>>()
                .unwrap();
            channel.update();
            channel.update();
            channel.push_event(UITextInputChanged {
                entity: field,
                value: value.into(),
            });
            self.update();
        }
        fn items(&mut self) -> Vec<Item> {
            let menu = self.menu().expect("the menu was built");
            let list = self
                .children(menu)
                .into_iter()
                .find_map(|view| {
                    self.f
                        .world
                        .get_component_for_entity::<UIScrollArea>(view)?
                        .content
                })
                .expect("the menu has a scrolled list");
            let selection = UITheme::default().selection();
            self.children(list)
                .into_iter()
                .map(|item| {
                    let node = self
                        .f
                        .world
                        .get_component_for_entity::<UINode>(item)
                        .unwrap();
                    Item {
                        entity: item,
                        label: self.text(self.children(item)[0]),
                        indent: node.padding.left,
                        shown: node.visible,
                        selected: self
                            .f
                            .world
                            .get_component_for_entity::<UIInteractionStyle>(item)
                            .unwrap()
                            .normal
                            == selection,
                    }
                })
                .collect()
        }
        fn item(&mut self, label: &str) -> Entity {
            self.items()
                .into_iter()
                .find(|item| item.label == label)
                .map(|item| item.entity)
                .unwrap_or_else(|| panic!("no item {label:?}"))
        }
        fn shown(&mut self) -> Vec<String> {
            self.items()
                .into_iter()
                .filter(|item| item.shown)
                .map(|item| item.label)
                .collect()
        }
    }

    struct Item {
        entity: Entity,
        label: String,
        indent: f32,
        shown: bool,
        selected: bool,
    }

    const TARGET: PathSegment = PathSegment::Field("target");

    fn unlinked(_: &Picking) -> Links {
        Links::default()
    }

    #[test]
    fn an_entity_with_no_name_or_an_empty_one_is_labelled_by_its_index() {
        let mut p = picking(unlinked);
        let unnamed = p.unnamed;
        p.set(SceneEntityRef::Entity(unnamed));
        assert_eq!(p.label(&[TARGET]), format!("Entity {}", unnamed.index()));

        let blank = p.blank;
        p.set(SceneEntityRef::Entity(blank));
        assert_eq!(p.label(&[TARGET]), format!("Entity {}", blank.index()));
    }

    #[test]
    fn an_unset_value_reads_not_in_this_scene() {
        let mut p = picking(unlinked);
        assert_eq!(p.label(&[TARGET]), NOT_IN_SCENE);
    }

    #[test]
    fn a_deleted_entity_reads_not_in_this_scene() {
        let mut p = picking(|p| Links {
            target: SceneEntityRef::Entity(p.capsule),
            ..Links::default()
        });
        let capsule = p.capsule;
        assert_eq!(p.label(&[TARGET]), "capsule");
        p.scene().despawn(capsule);
        p.update();
        assert_eq!(p.label(&[TARGET]), NOT_IN_SCENE);
    }

    #[test]
    fn the_scene_root_reads_not_in_this_scene() {
        let mut p = picking(|p| Links {
            target: SceneEntityRef::Entity(p.root),
            ..Links::default()
        });
        assert_eq!(p.label(&[TARGET]), NOT_IN_SCENE);
    }

    fn apply(value: &mut SceneEntityRef, edit: SceneEntityRef) -> Result<(), EditError> {
        EntityPicker.apply(
            value,
            &edit,
            &EditContext {
                assets: &AssetServer::new(),
                registry: &AssetRegistry::default(),
            },
        )
    }

    #[test]
    fn apply_takes_an_entity_or_the_unset_value_and_refuses_any_other_index() {
        let entity = Entity::parse_id("7v2").unwrap();
        let mut value = SceneEntityRef::default();

        let edit = EntityPicker
            .edit_to(&SceneEntityRef::Entity(entity))
            .unwrap();
        apply(&mut value, edit).unwrap();
        assert_eq!(value, SceneEntityRef::Entity(entity));

        assert_eq!(
            apply(&mut value, SceneEntityRef::Index(3)),
            Err(EditError::Rejected)
        );
        assert_eq!(value, SceneEntityRef::Entity(entity));

        apply(&mut value, SceneEntityRef::default()).unwrap();
        assert_eq!(value, SceneEntityRef::default());
    }

    #[test]
    fn option_and_list_rows_wrap_the_picker() {
        let mut p = picking(|p| Links {
            maybe: Some(SceneEntityRef::Entity(p.capsule)),
            many: vec![SceneEntityRef::Entity(p.unnamed), SceneEntityRef::default()],
            ..Links::default()
        });
        let unnamed = p.unnamed;
        let maybe = [PathSegment::Field("maybe"), PathSegment::Field("0")];
        let first = [PathSegment::Field("many"), PathSegment::Index(0)];
        let second = [PathSegment::Field("many"), PathSegment::Index(1)];
        for path in [&maybe[..], &first, &second] {
            let (_, row) = p.row(path).expect("the inspector built the row");
            assert_eq!(row.editor_type, TypeId::of::<EntityPicker>());
        }
        assert_eq!(p.label(&maybe), "capsule");
        assert_eq!(p.label(&first), format!("Entity {}", unnamed.index()));
        assert_eq!(p.label(&second), NOT_IN_SCENE);
    }

    fn linked_to_capsule(p: &Picking) -> Links {
        Links {
            target: SceneEntityRef::Entity(p.capsule),
            ..Links::default()
        }
    }

    #[test]
    fn the_popup_lists_none_then_the_candidates_indented_by_depth() {
        let mut p = picking(linked_to_capsule);
        assert!(!p.menu_open());
        let button = p.button();
        p.click(button);

        assert!(p.menu_open());
        let unnamed = format!("Entity {}", p.unnamed.index());
        let blank = format!("Entity {}", p.blank.index());
        let items = p.items();
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "None",
                "first",
                "second",
                "capsule",
                "skeleton",
                "Hips",
                unnamed.as_str(),
                blank.as_str(),
                "holder"
            ]
        );
        let indent = |label: &str| {
            items
                .iter()
                .find(|item| item.label == label)
                .map(|item| item.indent)
                .unwrap()
        };
        assert_eq!(indent("None"), indent("capsule"));
        assert!(indent("skeleton") > indent("capsule"));
        assert_eq!(
            indent("Hips") - indent("skeleton"),
            indent("skeleton") - indent("capsule")
        );
        assert_eq!(indent(&unnamed), indent("capsule"));
        let selected: Vec<&str> = items
            .iter()
            .filter(|item| item.selected)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(selected, ["capsule"]);
    }

    #[test]
    fn an_unset_value_shows_none_selected_and_the_button_closes_its_menu() {
        let mut p = picking(unlinked);
        let button = p.button();
        p.click(button);
        let selected: Vec<String> = p
            .items()
            .into_iter()
            .filter(|item| item.selected)
            .map(|item| item.label)
            .collect();
        assert_eq!(selected, ["None"]);

        p.click(button);
        assert!(!p.menu_open());
        p.click(button);
        assert_eq!(p.items().len(), 9);
    }

    #[test]
    fn picking_an_entity_commits_it_and_closes_the_menu() {
        let mut p = picking(unlinked);
        let button = p.button();
        p.click(button);
        let item = p.item("skeleton");

        p.click(item);
        p.update();
        p.update();

        assert!(!p.menu_open());
        assert_eq!(p.target(), SceneEntityRef::Entity(p.skeleton));
        assert_eq!(p.label(&[TARGET]), "skeleton");
    }

    #[test]
    fn picking_none_commits_the_unset_value() {
        let mut p = picking(linked_to_capsule);
        let button = p.button();
        p.click(button);
        let item = p.item("None");

        p.click(item);
        p.update();
        p.update();

        assert!(!p.menu_open());
        assert_eq!(p.target(), SceneEntityRef::default());
        assert_eq!(p.label(&[TARGET]), NOT_IN_SCENE);
    }

    #[test]
    fn the_filter_keeps_matches_their_ancestors_and_none() {
        let mut p = picking(unlinked);
        let button = p.button();
        p.click(button);

        p.type_filter("hip");

        assert_eq!(p.shown(), ["None", "capsule", "skeleton", "Hips"]);
        let menu = p.menu().unwrap();
        assert_eq!(
            p.f.world
                .get_component_for_entity::<EntityPickerMenu>(menu)
                .unwrap()
                .filter,
            "hip"
        );

        p.type_filter("ENTITY");
        let unnamed = format!("Entity {}", p.unnamed.index());
        let blank = format!("Entity {}", p.blank.index());
        assert_eq!(p.shown(), ["None", unnamed.as_str(), blank.as_str()]);
    }

    #[test]
    fn reopening_the_menu_empties_its_filter() {
        let mut p = picking(unlinked);
        let button = p.button();
        p.click(button);
        p.type_filter("hip");

        p.click(button);
        p.click(button);

        let menu = p.menu().unwrap();
        assert!(
            p.f.world
                .get_component_for_entity::<EntityPickerMenu>(menu)
                .unwrap()
                .filter
                .is_empty()
        );
        let field = p.filter_field();
        assert!(
            p.f.world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value
                .is_empty()
        );
        assert_eq!(p.shown().len(), 9);
    }

    #[test]
    fn the_select_button_selects_the_referenced_entity_and_reveals_it() {
        let mut p = picking(|p| Links {
            target: SceneEntityRef::Entity(p.hips),
            ..Links::default()
        });
        let hips = p.hips;
        let select = p.select_button();

        p.click(select);

        assert_eq!(p.f.selected(p.document), Some(hips));
        assert_eq!(
            p.f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .pending_scene_reveal(),
            Some(hips)
        );
    }

    #[test]
    fn the_select_button_does_nothing_for_an_unset_value_or_a_value_that_is_no_candidate() {
        let mut p = picking(unlinked);
        let holder = p.holder;
        let select = p.select_button();
        p.click(select);
        assert_eq!(p.f.selected(p.document), Some(holder));

        let root = p.root;
        p.set(SceneEntityRef::Entity(root));
        let select = p.select_button();
        p.click(select);
        assert_eq!(p.f.selected(p.document), Some(holder));
        assert_eq!(
            p.f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .pending_scene_reveal(),
            None
        );
    }

    #[test]
    fn the_menu_closes_when_its_edit_target_goes() {
        let mut p = picking(unlinked);
        let button = p.button();
        p.click(button);
        assert!(p.menu_open());

        p.f.activate(None);

        assert!(!p.menu_open());
    }
}

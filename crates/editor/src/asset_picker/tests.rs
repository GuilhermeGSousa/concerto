use super::*;
use crate::inspector::PropertyEditor;
use crate::{
    hosted::testing::SceneFixture,
    inspector::{InspectorRegistry, row_systems, spawn_panel},
    tools::selection::Select,
};
use concerto_ecs::{
    Entity, IntoSystem, Resource, System, World,
    entity::hierarchy::{ChildOf, Children},
    reflect::AppTypeRegistry,
    system::BoxedSystem,
};
use concerto_foundation::assets::{
    Asset,
    asset_server::AssetServer,
    asset_store::AssetStore,
    handle::{AssetHandle, ReflectAssetHandle},
    registry::AssetEntry,
};
use concerto_mesh::{Mesh, MeshComponent};
use concerto_reflect::TypeRegistry;
use concerto_ui::{interaction::UIInteractionStyle, material::UIMaterial, node::UINode};
use std::any::TypeId;

struct Picking {
    f: SceneFixture,
    document: Entity,
    entity: Entity,
    cube: AssetEntry,
    sphere: AssetEntry,
    systems: Vec<BoxedSystem>,
}

fn mesh_entry(path: &str) -> AssetEntry {
    AssetEntry {
        id: AssetId::new(),
        address: format!("content/{path}.gasset"),
        kind: Mesh::name().into(),
        provenance: None,
    }
}

fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let parent = cmd.spawn(UINode::default()).entity();
    spawn_panel(&mut cmd, parent, &theme);
}

fn picking() -> Picking {
    let mut f = SceneFixture::new();
    f.world.register_component::<ChildOf>();
    f.world.register_component::<Children>();
    f.world.insert_resource(InspectorRegistry::default());
    let cube = mesh_entry("props/cube");
    let sphere = mesh_entry("props/sphere");
    f.add_to_project(&cube);
    f.add_to_project(&sphere);
    let document = f.open();
    let root = f.scene_root(document);

    let scene = f.scene_world_mut(document);
    scene.register_type::<MeshComponent>();
    let store = AssetStore::<Mesh>::new();
    scene
        .get_resource_mut::<AssetServer>()
        .unwrap()
        .register_asset(&store);
    scene.insert_resource(store);
    let entity = scene.spawn(MeshComponent::default());
    scene.entity_mut(root).add_child(entity);
    scene.trigger(Select(Some(entity)));

    let mut panel = build_inspector.into_system();
    panel.initialize(&mut f.world);
    panel.run_and_apply((), &mut f.world);
    let mut systems = row_systems();
    systems.push(refresh_asset_picker_labels.into_boxed_system());
    for system in &mut systems {
        system.initialize(&mut f.world);
    }
    let mut p = Picking {
        f,
        document,
        entity,
        cube,
        sphere,
        systems,
    };
    p.update();
    p.update();
    p
}

impl Picking {
    fn scene(&self) -> &World {
        self.f.scene_world(self.document)
    }
    fn update(&mut self) {
        for system in &mut self.systems {
            system.run_and_apply((), &mut self.f.world);
        }
        self.f.world.tick();
        self.f.scene_world_mut(self.document).tick();
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
    fn button(&mut self) -> Entity {
        let label = self
            .f
            .world
            .query::<(Entity, &AssetPickerLabel), ()>()
            .iter(&mut self.f.world)
            .map(|(entity, _)| entity)
            .next()
            .expect("the inspector built a picker row");
        self.f
            .world
            .get_component_for_entity::<ChildOf>(label)
            .unwrap()
            .parent()
    }
    fn label(&mut self) -> String {
        self.f
            .world
            .query::<(&AssetPickerLabel, &UIText), ()>()
            .iter(&mut self.f.world)
            .map(|(_, text)| text.text.clone())
            .next()
            .expect("the picker row has a label")
    }
    fn menu_open(&mut self) -> bool {
        let overlay = UITheme::default().surface_overlay.to_linear();
        self.f
            .world
            .query::<(&AssetPickerMenu, &UIAnchoredPanel, &UIMaterial), ()>()
            .iter(&mut self.f.world)
            .any(|(_, panel, material)| {
                if panel.open {
                    assert_eq!(material.color, overlay);
                }
                panel.open
            })
    }
    fn items(&mut self) -> Vec<(Entity, String)> {
        let menus: Vec<Entity> = self
            .f
            .world
            .query::<(Entity, &AssetPickerMenu), ()>()
            .iter(&mut self.f.world)
            .map(|(entity, _)| entity)
            .collect();
        let lists: Vec<Entity> = menus
            .iter()
            .flat_map(|&menu| self.children(menu))
            .filter_map(|view| {
                self.f
                    .world
                    .get_component_for_entity::<UIScrollArea>(view)?
                    .content
            })
            .collect();
        lists
            .iter()
            .flat_map(|&list| self.children(list))
            .map(|item| (item, self.text(self.children(item)[0])))
            .collect()
    }
    fn picker(&self) -> AssetPicker {
        AssetPicker::new(
            self.scene()
                .get_resource::<AppTypeRegistry>()
                .unwrap()
                .get_type_data::<ReflectAssetHandle>(TypeId::of::<AssetHandle<Mesh>>())
                .unwrap(),
        )
    }
    fn held(&self) -> &AssetHandle<Mesh> {
        &self
            .scene()
            .get_component_for_entity::<MeshComponent>(self.entity)
            .unwrap()
            .handle
    }
}

#[test]
fn the_label_names_no_asset_for_an_empty_handle() {
    let mut p = picking();
    assert_eq!(p.label(), "None");
}

#[test]
fn the_buttons_caret_is_a_node_so_it_is_laid_out_and_drawn() {
    let mut p = picking();
    let button = p.button();
    let caret = p.children(button)[1];

    assert_eq!(p.text(caret), glyph::CARET_DOWN.to_string());
    assert!(
        p.f.world
            .get_component_for_entity::<UINode>(caret)
            .is_some(),
        "a text without a node never reaches layout or the draw lists"
    );
}

#[test]
fn the_menu_lists_only_assets_of_the_pickers_type() {
    let mut p = picking();
    assert!(!p.menu_open());

    let button = p.button();
    p.click(button);

    assert!(p.menu_open());
    let names: Vec<_> = p.items().into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["cube", "sphere"]);
}

#[test]
fn clicking_the_button_again_closes_the_menu() {
    let mut p = picking();
    let button = p.button();
    p.click(button);
    p.click(button);
    assert!(!p.menu_open());
}

#[test]
fn picking_an_item_gives_the_scene_a_loaded_handle_to_it_and_closes_the_menu() {
    let mut p = picking();
    let button = p.button();
    p.click(button);
    let sphere = p.sphere.id;
    let (item, _) = p
        .items()
        .into_iter()
        .find(|(_, name)| name == "sphere")
        .unwrap();

    p.click(item);
    p.update();
    p.update();

    assert!(!p.menu_open());
    assert_eq!(p.held().id(), sphere);
    assert!(matches!(p.held(), AssetHandle::Strong(..)));
    assert_eq!(p.label(), "sphere");
}

#[test]
fn reopening_the_menu_rebuilds_its_items_instead_of_adding_to_them() {
    let mut p = picking();
    let button = p.button();
    p.click(button);
    p.click(button);
    p.click(button);
    assert_eq!(p.items().len(), 2);
}

#[test]
fn an_asset_of_another_type_is_rejected_and_leaves_the_handle_alone() {
    let p = picking();
    let mut handle = AssetHandle::<Mesh>::default();
    let scene_asset = p.f.scene.id;

    let result = p.picker().apply(
        &mut handle,
        &scene_asset,
        &EditContext {
            assets: p.scene().get_resource().unwrap(),
            registry: p.scene().get_resource().unwrap(),
        },
    );

    assert_eq!(result, Err(EditError::Rejected));
    assert!(handle.id().is_nil());
}

#[test]
fn an_asset_type_the_scenes_server_never_registered_is_rejected() {
    let p = picking();
    let cube = p.cube.id;
    let mut handle = AssetHandle::<Mesh>::default();

    let result = p.picker().apply(
        &mut handle,
        &cube,
        &EditContext {
            assets: &AssetServer::new(),
            registry: p.scene().get_resource().unwrap(),
        },
    );

    assert_eq!(result, Err(EditError::Rejected));
    assert!(handle.id().is_nil());
}

#[test]
fn a_value_edit_to_a_handle_picks_the_asset_it_names() {
    let p = picking();
    let picker = p.picker();
    let sphere = p.sphere.id;
    let mut handle = AssetHandle::<Mesh>::default();

    let edit = picker
        .edit_to(&AssetHandle::<Mesh>::weak(sphere))
        .unwrap()
        .expect("a picker offers a value edit");
    picker
        .apply(
            &mut handle,
            edit.as_ref(),
            &EditContext {
                assets: p.scene().get_resource().unwrap(),
                registry: p.scene().get_resource().unwrap(),
            },
        )
        .unwrap();

    assert_eq!(handle.id(), sphere);
    assert!(matches!(handle, AssetHandle::Strong(..)));
    assert_eq!(picker.edit_to(&7u32).err(), Some(EditError::TypeMismatch));
}

#[derive(Resource, Default)]
struct Picks(Vec<AssetId>);

fn note_pick(id: AssetId) -> Listener<UIClick> {
    (move |_: On<UIClick>, mut picks: ResMut<Picks>| picks.0.push(id)).into_listener()
}

fn open_with_no_row(current: Option<AssetId>) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
           registry: Res<AssetRegistry>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        open_asset_menu(
            on.entity(),
            Mesh::name(),
            None,
            current,
            note_pick,
            &menus,
            &registry,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn rowless_button(p: &mut Picking, current: Option<AssetId>) -> Entity {
    p.f.world.insert_resource(Picks::default());
    p.f.world.spawn(open_with_no_row(current))
}

#[test]
fn a_menu_with_no_row_lists_the_kind_and_runs_the_pick_it_was_given() {
    let mut p = picking();
    let cube = p.cube.id;
    let sphere = p.sphere.id;
    let button = rowless_button(&mut p, Some(cube));

    p.click(button);

    assert!(p.menu_open());
    let menu_row =
        p.f.world
            .query::<&AssetPickerMenu, ()>()
            .iter(&mut p.f.world)
            .map(|menu| menu.row)
            .next();
    assert_eq!(menu_row, Some(None));
    let items = p.items();
    let names: Vec<_> = items.iter().map(|(_, name)| name.as_str()).collect();
    assert_eq!(names, ["cube", "sphere"]);
    let selection = UITheme::default().selection();
    let fill = |p: &Picking, item: Entity| {
        p.f.world
            .get_component_for_entity::<UIInteractionStyle>(item)
            .unwrap()
            .normal
    };
    assert_eq!(fill(&p, items[0].0), selection);
    assert_ne!(fill(&p, items[1].0), selection);

    p.click(items[1].0);

    assert_eq!(p.f.world.get_resource::<Picks>().unwrap().0, [sphere]);
    assert!(
        p.f.world
            .get_resource::<PropertyCommits>()
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn the_button_that_opened_a_menu_with_no_row_closes_it() {
    let mut p = picking();
    let button = rowless_button(&mut p, None);

    p.click(button);
    p.click(button);

    assert!(!p.menu_open());
}

#[test]
fn the_inspector_takes_the_menu_back_from_a_menu_with_no_row() {
    let mut p = picking();
    let rowless = rowless_button(&mut p, None);
    let button = p.button();
    let sphere = p.sphere.id;
    p.click(rowless);

    p.click(button);
    let (item, _) = p
        .items()
        .into_iter()
        .find(|(_, name)| name == "sphere")
        .unwrap();
    p.click(item);
    p.update();
    p.update();

    assert!(!p.menu_open());
    assert_eq!(p.held().id(), sphere);
    assert!(p.f.world.get_resource::<Picks>().unwrap().0.is_empty());
}

#[derive(Default)]
struct IdOnly;

impl PropertyEditor<AssetHandle<Mesh>> for IdOnly {
    type Snapshot = AssetId;
    type Edit = ();

    fn snapshot(&self, value: &AssetHandle<Mesh>) -> AssetId {
        value.id()
    }

    fn build(&self, _: &mut EntityCommandQueue, _: &AssetId, _: &UITheme) {}

    fn apply(&self, _: &mut AssetHandle<Mesh>, _: &(), _: &EditContext) -> Result<(), EditError> {
        Ok(())
    }
}

#[test]
fn an_editor_registered_for_the_handle_type_wins_over_the_picker() {
    let mut types = TypeRegistry::default();
    types.register::<MeshComponent>();
    let mut registry = InspectorRegistry::default();
    let editor = |registry: &InspectorRegistry| {
        let handle = registry.collect(&MeshComponent::default(), &types)[0].type_id;
        registry
            .editor(handle, Some(&types))
            .map(|editor| editor.editor_type)
    };
    assert_eq!(editor(&registry), Some(TypeId::of::<AssetPicker>()));

    registry.register_property_editor::<AssetHandle<Mesh>, IdOnly>();

    assert_eq!(editor(&registry), Some(TypeId::of::<IdOnly>()));
}

#[test]
fn a_handle_type_the_scene_never_registered_has_no_picker() {
    let types = TypeRegistry::default();
    let properties = InspectorRegistry::default().collect(&MeshComponent::default(), &types);
    assert!(properties[0].value().is_none());
}

#[test]
fn the_menus_rows_sit_edge_to_edge_so_its_height_fits_them() {
    let mut p = picking();
    let button = p.button();
    p.click(button);
    let (item, _) = p.items()[0].clone();

    let list =
        p.f.world
            .get_component_for_entity::<ChildOf>(item)
            .unwrap()
            .parent();

    assert_eq!(
        p.f.world
            .get_component_for_entity::<UINode>(list)
            .unwrap()
            .gap,
        glam::Vec2::ZERO
    );
}

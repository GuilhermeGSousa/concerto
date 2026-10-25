use super::*;
use crate::{
    hosted::testing::SceneFixture,
    inspector::{InspectorRegistry, row_systems, spawn_panel},
    tools::selection::Select,
};
use concerto_ecs::{
    Entity, IntoSystem, System, World,
    entity::hierarchy::{ChildOf, Children},
    system::BoxedSystem,
};
use concerto_foundation::assets::{
    Asset, asset_server::AssetServer, asset_store::AssetStore, registry::AssetEntry,
};
use concerto_mesh::{Mesh, MeshComponent};
use concerto_ui::node::UINode;

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
    let mut registry = InspectorRegistry::default();
    registry.register_property_editor::<AssetHandle<Mesh>, AssetPicker<Mesh>>();
    f.world.insert_resource(registry);
    let cube = mesh_entry("props/cube");
    let sphere = mesh_entry("props/sphere");
    f.add_to_project(&cube);
    f.add_to_project(&sphere);
    let document = f.open();

    let scene = f.scene_world_mut(document);
    scene.register_type::<MeshComponent>();
    scene.register_type::<AssetHandle<Mesh>>();
    let store = AssetStore::<Mesh>::new();
    scene
        .get_resource_mut::<AssetServer>()
        .unwrap()
        .register_asset(&store);
    scene.insert_resource(store);
    let entity = scene.spawn(MeshComponent::default());
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
        self.f
            .world
            .query::<(&AssetPickerMenu, &UIAnchoredPanel), ()>()
            .iter(&mut self.f.world)
            .any(|(_, panel)| panel.open)
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

    let result = AssetPicker::<Mesh>::default().apply(
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

    let result = AssetPicker::<Mesh>::default().apply(
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

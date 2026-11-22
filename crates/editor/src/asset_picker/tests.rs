use super::*;
use concerto_ecs::{
    Entity, Resource, World,
    entity::hierarchy::{ChildOf, Children},
};
use concerto_foundation::assets::{Asset, registry::AssetEntry};
use concerto_mesh::Mesh;

fn entry(path: &str, kind: &str) -> AssetEntry {
    AssetEntry {
        embedded_in: None,
        id: AssetId::new(),
        address: format!("content/{path}.gasset"),
        kind: kind.into(),
        kind_version: None,
        provenance: None,
    }
}

#[derive(Resource, Default)]
struct Picks(Vec<AssetId>);

fn note_pick(id: AssetId) -> Listener<UIClick> {
    (move |_: On<UIClick>, mut picks: ResMut<Picks>| picks.0.push(id)).into_listener()
}

fn open_with_no_row(
    on: On<UIClick>,
    mut menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
    registry: Res<AssetRegistry>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    open_asset_menu(
        on.entity(),
        Mesh::name(),
        None,
        None,
        note_pick,
        &mut menus,
        &registry,
        &theme,
        &mut cmd,
    );
}

fn click(world: &mut World, entity: Entity) {
    world.trigger_on(
        entity,
        UIClick {
            position: Default::default(),
            button: MouseButton::Left,
        },
    );
}

fn children(world: &World, entity: Entity) -> Vec<Entity> {
    world
        .get_component_for_entity::<Children>(entity)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default()
}

struct Item {
    entity: Entity,
    name: String,
}

fn items(world: &mut World) -> Vec<Item> {
    let menus: Vec<Entity> = world
        .query::<(Entity, &AssetPickerMenu), ()>()
        .iter(world)
        .map(|(entity, _)| entity)
        .collect();
    let lists: Vec<Entity> = menus
        .iter()
        .flat_map(|&menu| children(world, menu))
        .filter_map(|view| {
            world
                .get_component_for_entity::<UIScrollArea>(view)?
                .content
        })
        .collect();
    lists
        .iter()
        .flat_map(|&list| children(world, list))
        .map(|item| {
            let label = children(world, item)[0];
            let text = world.get_component_for_entity::<UIText>(label).unwrap();
            Item {
                entity: item,
                name: text.text.clone(),
            }
        })
        .collect()
}

#[test]
fn a_menu_with_no_row_lists_the_kind_and_runs_the_pick_it_was_given() {
    let mut world = World::new();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world.insert_resource(UITheme::default());
    world.insert_resource(Picks::default());
    let sphere = entry("props/sphere", Mesh::name());
    let mut registry = AssetRegistry::default();
    registry.insert(entry("props/cube", Mesh::name()));
    registry.insert(sphere.clone());
    registry.insert(entry("levels/start", "Scene"));
    world.insert_resource(registry);
    let button = world.spawn(open_with_no_row.into_listener());

    click(&mut world, button);

    let items = items(&mut world);
    let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["cube", "sphere"]);

    click(&mut world, items[1].entity);

    assert_eq!(world.get_resource::<Picks>().unwrap().0, [sphere.id]);
}

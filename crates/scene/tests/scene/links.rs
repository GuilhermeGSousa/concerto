use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::LateUpdate,
    App,
};
use concerto_ecs::{
    component::{name::Name, scene::SceneEntityRef},
    entity::hierarchy::ChildOf,
    events::event_reader::EventReader,
    reflect::{ReflectComponent, ReflectSceneComponent},
    resource::{ResMut, Resource},
    Component, Entity,
};
use concerto_foundation::assets::{asset_server::AssetServer, handle::AssetHandle, AssetId};
use concerto_foundation::transform::Transform;
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_scene::{
    capture::{capture_scene, capture_subtree},
    link::{
        owning_link, ChangeOutcome, LinkParent, LinkRoot, LinkedNode, OwnedPath, SceneLink,
        SceneLinkSpawned, SceneLinkState,
    },
    link_path::node_paths,
    plugin::ScenePlugin,
    scene::{LinkChange, Scene, SceneLinkData, SceneNode, SerializedComponent},
    spawner::{spawn_scene_in_world, SpawnFilter},
};

#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
struct Label {
    text: String,
}

#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default, SceneComponent)]
struct Prop {
    art: AssetHandle<Scene>,
    facing: SceneEntityRef,
}

#[derive(Resource, Default)]
struct Heard(Vec<Entity>);

fn hear(mut spawned: EventReader<SceneLinkSpawned>, mut heard: ResMut<Heard>) {
    heard.0.extend(spawned.read().map(|event| event.link));
}

fn node(name: &str, children: Vec<usize>) -> SceneNode {
    let mut node = SceneNode {
        name: name.into(),
        children,
        components: vec![],
        link: None,
    };
    node.push_component(&Transform::IDENTITY).unwrap();
    node
}

fn label(text: &str) -> SerializedComponent {
    SerializedComponent {
        type_name: std::any::type_name::<Label>().into(),
        value: concerto_reflect::to_value(&Label { text: text.into() }).unwrap(),
    }
}

fn prop(art: AssetId) -> SerializedComponent {
    SerializedComponent {
        type_name: std::any::type_name::<Prop>().into(),
        value: concerto_reflect::to_value(&Prop {
            art: AssetHandle::weak(art),
            facing: SceneEntityRef::default(),
        })
        .unwrap(),
    }
}

fn link_parent(path: &str) -> SerializedComponent {
    SerializedComponent {
        type_name: std::any::type_name::<LinkParent>().into(),
        value: concerto_reflect::to_value(&LinkParent { path: path.into() }).unwrap(),
    }
}

fn room() -> Scene {
    Scene {
        nodes: vec![
            node("room", vec![1, 2, 3, 4]),
            node("lamp", vec![]),
            node("helper", vec![]),
            node("chair", vec![]),
            node("chair", vec![]),
        ],
        referenced_assets: vec![],
    }
}

fn room_with_shelf() -> Scene {
    Scene {
        nodes: vec![
            node("room", vec![2, 3, 4, 5]),
            node("lamp", vec![]),
            node("shelf", vec![1]),
            node("helper", vec![]),
            node("chair", vec![]),
            node("chair", vec![]),
        ],
        referenced_assets: vec![],
    }
}

fn app() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(ScenePlugin)
        .register_type::<Label>()
        .register_type::<Prop>()
        .insert_resource(Heard::default())
        .add_system(LateUpdate, hear);
    app.finish_plugin_build();
    app
}

fn settle(app: &mut App, link: Entity) -> &SceneLinkState {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while matches!(state(app, link), SceneLinkState::Waiting) {
        assert!(
            std::time::Instant::now() < deadline,
            "the link never spawned"
        );
        app.update();
    }
    app.update();
    state(app, link)
}

fn state(app: &App, link: Entity) -> &SceneLinkState {
    app.main()
        .world()
        .get_component_for_entity::<SceneLinkState>(link)
        .unwrap()
}

fn linked(app: &mut App, link: Entity, path: &str) -> Entity {
    linked_nodes(app, link)
        .into_iter()
        .find(|node| node.path == path)
        .unwrap_or_else(|| panic!("no linked node at {path}"))
        .entity
}

struct Linked {
    entity: Entity,
    path: String,
}

fn linked_nodes(app: &mut App, link: Entity) -> Vec<Linked> {
    let world = app.main_mut().world_mut();
    world
        .query::<(Entity, &LinkedNode), ()>()
        .iter(world)
        .filter(|(_, node)| node.link == link)
        .map(|(entity, node)| Linked {
            entity,
            path: node.path.clone(),
        })
        .collect()
}

fn parent(app: &App, entity: Entity) -> Option<Entity> {
    app.main()
        .world()
        .get_component_for_entity::<ChildOf>(entity)
        .map(ChildOf::parent)
}

fn label_of(app: &App, entity: Entity) -> Option<String> {
    app.main()
        .world()
        .get_component_for_entity::<Label>(entity)
        .map(|label| label.text.clone())
}

#[test]
fn a_link_spawns_its_scene_applies_its_changes_and_respawns_keeping_own_children() {
    let mut app = app();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let s = AssetId::new();
    let handle = server.insert_loaded(s, room());
    let art = AssetId::new();

    let changes = vec![
        LinkChange::RemoveNode {
            path: "room/helper".into(),
        },
        LinkChange::RemoveComponent {
            path: "room/lamp".into(),
            type_name: std::any::type_name::<Transform>().into(),
        },
        LinkChange::AddComponent {
            path: "room/chair#1".into(),
            component: label("added"),
        },
        LinkChange::AddComponent {
            path: "room/gone".into(),
            component: prop(art),
        },
    ];
    let link = app.main_mut().world_mut().spawn(
        changes
            .iter()
            .cloned()
            .fold(SceneLink::new(handle.clone()), SceneLink::with_change),
    );
    let own = app.main_mut().world_mut().spawn((
        Name::new("own"),
        Transform::IDENTITY,
        LinkParent {
            path: "room/lamp".into(),
        },
    ));
    app.main_mut().world_mut().entity_mut(link).add_child(own);

    assert!(matches!(settle(&mut app, link), SceneLinkState::Spawned));
    assert_eq!(app.get_resource::<Heard>().unwrap().0, [link]);
    let world = app.main_mut().world_mut();
    assert!(!world
        .query::<&Name, ()>()
        .iter(world)
        .any(|name| name.as_str() == "helper"));
    let lamp = linked(&mut app, link, "room/lamp");
    assert!(app
        .main()
        .world()
        .get_component_for_entity::<Transform>(lamp)
        .is_none());
    let chairs = [
        linked(&mut app, link, "room/chair"),
        linked(&mut app, link, "room/chair#1"),
    ];
    assert_eq!(
        chairs.map(|chair| label_of(&app, chair)),
        [None, Some("added".to_owned())]
    );
    let scene_link = app
        .main()
        .world()
        .get_component_for_entity::<SceneLink>(link)
        .unwrap();
    assert_eq!(
        scene_link.outcomes(),
        [
            ChangeOutcome::Applied,
            ChangeOutcome::Applied,
            ChangeOutcome::Applied,
            ChangeOutcome::Unapplied(format!("no node at room/gone in {}", s.simple_hex())),
        ]
    );
    let mut paths: Vec<String> = linked_nodes(&mut app, link)
        .into_iter()
        .map(|node| node.path)
        .collect();
    paths.sort();
    let mut expected: Vec<String> = node_paths(&room())
        .into_iter()
        .filter(|path| path != "room/helper")
        .collect();
    expected.sort();
    assert_eq!(paths, expected);
    assert_eq!(parent(&app, own), Some(lamp));

    app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<Label>(chairs[1])
        .unwrap()
        .text = "edited".into();
    let captured = capture_subtree(app.main().world(), link).unwrap();
    let mut saved = changes.clone();
    saved[2] = LinkChange::AddComponent {
        path: "room/chair#1".into(),
        component: label("edited"),
    };
    assert_eq!(captured.nodes.len(), 2);
    assert_eq!(
        captured.nodes[0].link,
        Some(SceneLinkData {
            scene: s,
            changes: saved,
        })
    );
    assert_eq!(captured.nodes[0].children, [1]);
    assert!(captured.nodes[1]
        .components
        .contains(&link_parent("room/lamp")));
    assert!(captured.referenced_assets.contains(&s));
    assert!(captured.referenced_assets.contains(&art));
    let mut copy = self::app();
    copy.get_resource::<AssetServer>()
        .unwrap()
        .insert_loaded(s, room());
    let world = copy.main_mut().world_mut();
    let holder = world.spawn(Transform::IDENTITY);
    let copied =
        spawn_scene_in_world(world, &captured, holder, &SpawnFilter::default()).node_entities[0];
    assert!(matches!(settle(&mut copy, copied), SceneLinkState::Spawned));
    assert_eq!(
        bincode::serialize(&capture_scene(copy.main().world(), holder).unwrap()).unwrap(),
        bincode::serialize(&captured).unwrap()
    );
    let world = app.main_mut().world_mut();
    world.insert(
        Prop {
            art: AssetHandle::default(),
            facing: SceneEntityRef::Entity(lamp),
        },
        own,
    );
    let refused = capture_subtree(world, link).unwrap_err().to_string();
    assert!(
        refused.contains(
            "references the linked node room/lamp; references into a link are not supported yet"
        ),
        "{refused}"
    );
    world.remove_component::<Prop>(own);

    app.main_mut()
        .world_mut()
        .insert(Label { text: "own".into() }, own);
    server.insert_loaded(s, room_with_shelf());
    app.update();
    settle(&mut app, link);
    assert!(matches!(state(&app, link), SceneLinkState::Spawned));
    assert_eq!(app.get_resource::<Heard>().unwrap().0, [link, link]);
    linked(&mut app, link, "room/shelf/lamp");
    assert_eq!(parent(&app, own), Some(link));
    assert_eq!(label_of(&app, own).as_deref(), Some("own"));

    let m = AssetId::new();
    let mut inner = node("inner", vec![]);
    let manor = LinkChange::AddComponent {
        path: "room/helper".into(),
        component: label("manor"),
    };
    inner.link = Some(SceneLinkData {
        scene: s,
        changes: vec![manor.clone()],
    });
    let outer_handle = server.insert_loaded(
        m,
        Scene {
            nodes: vec![inner],
            referenced_assets: vec![],
        },
    );
    let world = app.main_mut().world_mut();
    let outer = world.spawn(
        SceneLink::new(outer_handle)
            .with_change(LinkChange::AddComponent {
                path: "inner/room/chair".into(),
                component: label("deep"),
            })
            .with_change(LinkChange::RemoveComponent {
                path: "inner/room/helper".into(),
                type_name: std::any::type_name::<Label>().into(),
            }),
    );
    let guest = world.spawn((
        Name::new("guest"),
        Transform::IDENTITY,
        LinkParent {
            path: "inner/room/chair#1".into(),
        },
    ));
    world.entity_mut(outer).add_child(guest);
    assert!(matches!(settle(&mut app, outer), SceneLinkState::Spawned));
    let inner = linked(&mut app, outer, "inner");
    assert!(matches!(state(&app, inner), SceneLinkState::Spawned));
    assert_eq!(
        app.main()
            .world()
            .get_component_for_entity::<SceneLink>(inner)
            .unwrap()
            .changes(),
        [manor]
    );
    let deep_chair = linked(&mut app, inner, "room/chair");
    assert_eq!(label_of(&app, deep_chair).as_deref(), Some("deep"));
    assert_eq!(
        owning_link(app.main().world(), deep_chair),
        Some(OwnedPath {
            link: outer,
            path: "inner/room/chair".into()
        })
    );
    assert_eq!(
        parent(&app, guest),
        Some(linked(&mut app, inner, "room/chair#1"))
    );
    let deep_helper = linked(&mut app, inner, "room/helper");
    assert_eq!(label_of(&app, deep_helper), None);
    assert_eq!(
        app.main()
            .world()
            .get_component_for_entity::<SceneLink>(outer)
            .unwrap()
            .outcomes(),
        [ChangeOutcome::Applied, ChangeOutcome::Applied]
    );

    app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<Label>(deep_chair)
        .unwrap()
        .text = "edited".into();
    let mut one_chair = room_with_shelf();
    one_chair.nodes[0].children.pop();
    one_chair.nodes.pop();
    server.insert_loaded(s, one_chair);
    app.update();
    settle(&mut app, inner);
    let deep_chair = linked(&mut app, inner, "room/chair");
    assert_eq!(label_of(&app, deep_chair).as_deref(), Some("edited"));
    let captured = capture_subtree(app.main().world(), outer).unwrap();
    assert_eq!(
        captured.nodes[0].link.as_ref().unwrap().changes[0],
        LinkChange::AddComponent {
            path: "inner/room/chair".into(),
            component: label("edited"),
        }
    );
    assert!(captured
        .nodes
        .iter()
        .find(|node| node.name == "guest")
        .unwrap()
        .components
        .contains(&link_parent("inner/room/chair#1")));

    let cold = AssetId::new();
    let mut late = node("late", vec![]);
    late.link = Some(SceneLinkData {
        scene: cold,
        changes: vec![],
    });
    let early_handle = server.insert_loaded(
        AssetId::new(),
        Scene {
            nodes: vec![late],
            referenced_assets: vec![],
        },
    );
    let world = app.main_mut().world_mut();
    let early = world.spawn(
        SceneLink::new(early_handle).with_change(LinkChange::AddComponent {
            path: "late/room/chair".into(),
            component: label("late"),
        }),
    );
    let lodger = world.spawn((
        Name::new("lodger"),
        Transform::IDENTITY,
        LinkParent {
            path: "late/room/lamp".into(),
        },
    ));
    world.entity_mut(early).add_child(lodger);
    assert!(matches!(settle(&mut app, early), SceneLinkState::Spawned));
    let outcomes = |app: &App| {
        app.main()
            .world()
            .get_component_for_entity::<SceneLink>(early)
            .unwrap()
            .outcomes()
            .to_vec()
    };
    assert_eq!(
        outcomes(&app),
        [ChangeOutcome::Unapplied("waiting for late to load".into())]
    );
    let late = linked(&mut app, early, "late");
    server.insert_loaded(cold, room());
    assert!(matches!(settle(&mut app, late), SceneLinkState::Spawned));
    assert_eq!(outcomes(&app), [ChangeOutcome::Applied]);
    let late_chair = linked(&mut app, late, "room/chair");
    assert_eq!(label_of(&app, late_chair).as_deref(), Some("late"));
    assert_eq!(
        parent(&app, lodger),
        Some(linked(&mut app, late, "room/lamp"))
    );
    let captured = capture_subtree(app.main().world(), early).unwrap();
    assert_eq!(captured.nodes.len(), 2);
    assert!(captured.nodes[1]
        .components
        .contains(&link_parent("late/room/lamp")));

    let mut copy = node("copy", vec![]);
    let stored = vec![LinkChange::RemoveNode {
        path: "room/helper".into(),
    }];
    copy.link = Some(SceneLinkData {
        scene: s,
        changes: stored.clone(),
    });
    let world = app.main_mut().world_mut();
    let holder = world.spawn(Transform::IDENTITY);
    let opened = spawn_scene_in_world(
        world,
        &Scene {
            nodes: vec![copy],
            referenced_assets: vec![],
        },
        holder,
        &SpawnFilter::default(),
    )
    .node_entities[0];
    assert!(matches!(settle(&mut app, opened), SceneLinkState::Spawned));
    assert_eq!(
        app.main()
            .world()
            .get_component_for_entity::<SceneLink>(opened)
            .unwrap()
            .changes(),
        stored
    );
    linked(&mut app, opened, "room/shelf/lamp");
    assert!(!linked_nodes(&mut app, opened)
        .iter()
        .any(|node| node.path == "room/helper"));

    let mut mid = node("mid", vec![]);
    mid.link = Some(SceneLinkData {
        scene: s,
        changes: vec![],
    });
    let s2 = AssetId::new();
    server.insert_loaded(
        s2,
        Scene {
            nodes: vec![mid],
            referenced_assets: vec![],
        },
    );
    let mut middle = node("middle", vec![]);
    middle.link = Some(SceneLinkData {
        scene: s2,
        changes: vec![LinkChange::AddComponent {
            path: "mid/room/chair".into(),
            component: label("manor"),
        }],
    });
    let level_handle = server.insert_loaded(
        AssetId::new(),
        Scene {
            nodes: vec![middle],
            referenced_assets: vec![],
        },
    );
    let level = app
        .main_mut()
        .world_mut()
        .spawn(
            SceneLink::new(level_handle).with_change(LinkChange::RemoveComponent {
                path: "middle/mid/room/chair".into(),
                type_name: std::any::type_name::<Label>().into(),
            }),
        );
    assert!(matches!(settle(&mut app, level), SceneLinkState::Spawned));
    let middle = linked(&mut app, level, "middle");
    let mid = linked(&mut app, middle, "mid");
    settle(&mut app, mid);
    let far_chair = linked(&mut app, mid, "room/chair");
    assert_eq!(label_of(&app, far_chair), None);
    assert_eq!(
        app.main()
            .world()
            .get_component_for_entity::<SceneLink>(level)
            .unwrap()
            .outcomes(),
        [ChangeOutcome::Applied]
    );

    app.main_mut().world_mut().insert_resource(LinkRoot(s));
    let looped = app
        .main_mut()
        .world_mut()
        .spawn(SceneLink::new(handle.clone()));
    assert_eq!(
        settle(&mut app, looped),
        &SceneLinkState::Refused(format!(
            "{} links itself through {}",
            s.simple_hex(),
            s.simple_hex()
        ))
    );
    assert!(linked_nodes(&mut app, looped).is_empty());
}

use concerto_ecs::{
    Changed, IntoSystem, Query, ResMut, Resource, System, World,
    entity::Entity,
    entity::hierarchy::{ChildOf, Children},
};

fn world() -> World {
    let mut world = World::new();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world
}

fn children_of(world: &World, parent: Entity) -> Option<Vec<Entity>> {
    world
        .get_component_for_entity::<Children>(parent)
        .map(|children| children.iter().copied().collect())
}

#[derive(Clone, Copy, Debug)]
enum Edit {
    AddChild { parent: usize, child: usize },
    InsertChildOf { parent: usize, child: usize },
    RemoveChildOf(usize),
    Despawn(usize),
}

struct Family {
    parent: usize,
    children: &'static [usize],
}

struct HierarchyCase {
    name: &'static str,
    nodes: usize,
    edits: &'static [Edit],
    families: &'static [Family],
    despawned: &'static [usize],
}

#[test]
fn edits_keep_children_and_child_of_in_step() {
    use Edit::*;
    let cases = [
        HierarchyCase {
            name: "despawning a leaf keeps its siblings",
            nodes: 3,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 1,
                },
                AddChild {
                    parent: 0,
                    child: 2,
                },
                Despawn(1),
            ],
            families: &[Family {
                parent: 0,
                children: &[2],
            }],
            despawned: &[1],
        },
        HierarchyCase {
            name: "despawning a parent takes its children",
            nodes: 3,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 1,
                },
                AddChild {
                    parent: 0,
                    child: 2,
                },
                Despawn(1),
                Despawn(0),
            ],
            families: &[],
            despawned: &[0, 1, 2],
        },
        HierarchyCase {
            name: "despawning the last leaf removes the parent's Children",
            nodes: 2,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 1,
                },
                Despawn(1),
            ],
            families: &[],
            despawned: &[1],
        },
        HierarchyCase {
            name: "despawning a subtree detaches it from its surviving parent",
            nodes: 3,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 1,
                },
                AddChild {
                    parent: 1,
                    child: 2,
                },
                Despawn(1),
            ],
            families: &[],
            despawned: &[1, 2],
        },
        HierarchyCase {
            name: "reparenting through the builder",
            nodes: 3,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 2,
                },
                AddChild {
                    parent: 1,
                    child: 2,
                },
            ],
            families: &[Family {
                parent: 1,
                children: &[2],
            }],
            despawned: &[],
        },
        HierarchyCase {
            name: "reparenting by inserting ChildOf",
            nodes: 3,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 2,
                },
                InsertChildOf {
                    parent: 1,
                    child: 2,
                },
            ],
            families: &[Family {
                parent: 1,
                children: &[2],
            }],
            despawned: &[],
        },
        HierarchyCase {
            name: "re-adding a child to its current parent",
            nodes: 2,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 1,
                },
                AddChild {
                    parent: 0,
                    child: 1,
                },
                InsertChildOf {
                    parent: 0,
                    child: 1,
                },
            ],
            families: &[Family {
                parent: 0,
                children: &[1],
            }],
            despawned: &[],
        },
        HierarchyCase {
            name: "inserting ChildOf attaches in order",
            nodes: 3,
            edits: &[
                InsertChildOf {
                    parent: 0,
                    child: 1,
                },
                InsertChildOf {
                    parent: 0,
                    child: 2,
                },
            ],
            families: &[Family {
                parent: 0,
                children: &[1, 2],
            }],
            despawned: &[],
        },
        HierarchyCase {
            name: "detaching one of several children",
            nodes: 3,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 1,
                },
                AddChild {
                    parent: 0,
                    child: 2,
                },
                RemoveChildOf(1),
            ],
            families: &[Family {
                parent: 0,
                children: &[2],
            }],
            despawned: &[],
        },
        HierarchyCase {
            name: "detaching the last child removes the parent's Children",
            nodes: 2,
            edits: &[
                AddChild {
                    parent: 0,
                    child: 1,
                },
                RemoveChildOf(1),
            ],
            families: &[],
            despawned: &[],
        },
    ];
    for case in cases {
        let mut world = world();
        let nodes: Vec<Entity> = (0..case.nodes).map(|_| world.spawn(())).collect();

        for edit in case.edits {
            match *edit {
                AddChild { parent, child } => {
                    world.entity_mut(nodes[parent]).add_child(nodes[child]);
                }
                InsertChildOf { parent, child } => {
                    world.insert(ChildOf::new(nodes[parent]), nodes[child]);
                }
                RemoveChildOf(child) => world.remove_component::<ChildOf>(nodes[child]),
                Despawn(node) => world.despawn(nodes[node]),
            }
        }

        for (index, node) in nodes.iter().enumerate() {
            let name = case.name;
            let alive = !case.despawned.contains(&index);
            assert_eq!(world.entity_is_valid(*node), alive, "{name}: node {index}");
            if !alive {
                continue;
            }
            let family = case.families.iter().find(|family| family.parent == index);
            let expected = family.map(|family| family.children.iter().map(|i| nodes[*i]).collect());
            assert_eq!(children_of(&world, *node), expected, "{name}: node {index}");
            let parent = case
                .families
                .iter()
                .find(|family| family.children.contains(&index))
                .map(|family| nodes[family.parent]);
            let child_of = world
                .get_component_for_entity::<ChildOf>(*node)
                .map(ChildOf::parent);
            assert_eq!(child_of, parent, "{name}: node {index}");
        }
    }
}

#[test]
fn despawn_takes_a_deep_hierarchy_without_recursing() {
    let mut world = world();
    let root = world.spawn(());
    let mut entities = vec![root];
    let mut parent = root;
    for _ in 0..2_048 {
        let child = world.spawn(());
        world.entity_mut(parent).add_child(child);
        entities.push(child);
        parent = child;
    }

    world.despawn(root);

    assert!(
        entities
            .into_iter()
            .all(|entity| !world.entity_is_valid(entity))
    );
}

#[derive(Resource, Default)]
struct Looks(Vec<usize>);

fn count_changed_children(parents: Query<Entity, Changed<Children>>, mut looks: ResMut<Looks>) {
    looks.0.push(parents.iter().count());
}

#[test]
fn attaching_or_detaching_a_child_changes_the_parents_children() {
    let mut world = world();
    world.insert_resource(Looks::default());
    let parent = world.spawn(());
    let mut watcher = count_changed_children.into_system();
    watcher.initialize(&mut world);
    watcher.run_and_apply((), &mut world);

    let first = world.spawn(ChildOf::new(parent));
    watcher.run_and_apply((), &mut world);
    world.spawn(ChildOf::new(parent));
    watcher.run_and_apply((), &mut world);
    watcher.run_and_apply((), &mut world);
    world.remove_component::<ChildOf>(first);
    watcher.run_and_apply((), &mut world);
    watcher.run_and_apply((), &mut world);

    assert_eq!(world.get_resource::<Looks>().unwrap().0, [0, 1, 1, 0, 1, 0]);
}

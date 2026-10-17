//! Create, delete, duplicate and rename the entities of an open scene, or of a running game's world.
use anyhow::{Context, bail};
use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    Entity, World,
    component::name::Name,
    entity::hierarchy::ChildOf,
    signal::{On, Signal},
};
use concerto_foundation::transform::Transform;
use concerto_render::components::render_entity::SyncWithRenderWorld;
use concerto_scene::{capture::capture_subtree, scene::Scene, spawner::spawn_scene_in_world};

use crate::tools::{
    scene::{RevealEntity, SceneEdited, SceneRoot, SceneStatus, write},
    selection::Select,
};

/// A structural change to an open scene, or to a game's world, which has no scene root. Trigger it; one listener applies it.
pub enum EntityEdit {
    /// Adds an empty entity as the last child of `parent`, or at the top level of a world without a scene root.
    Create {
        parent: Option<Entity>,
    },
    /// Removes the entity and its descendants.
    Delete(Entity),
    /// Copies the entity and its descendants beside it.
    Duplicate(Entity),
    Rename {
        entity: Entity,
        name: String,
    },
}

impl Signal for EntityEdit {}

pub(crate) fn build(app: &mut App) {
    app.add_listener(apply_entity_edit.at_edit_time());
}

enum Applied {
    Changed(Option<Entity>),
    Removed(Option<Entity>),
    Unchanged,
}

pub(crate) fn apply_entity_edit(on: On<EntityEdit>, world: &mut World) {
    let operation = match on.signal() {
        EntityEdit::Create { .. } => "Create",
        EntityEdit::Delete(_) => "Delete",
        EntityEdit::Duplicate(_) => "Duplicate",
        EntityEdit::Rename { .. } => "Rename",
    };
    match apply(world, on.signal()) {
        Ok(Applied::Changed(selected)) => {
            if let Some(selected) = selected {
                world.trigger(Select(Some(selected)));
                write(world, RevealEntity(selected));
            }
            write(world, SceneEdited);
        }
        Ok(Applied::Removed(parent)) => {
            world.trigger(Select(parent));
            if let Some(parent) = parent {
                write(world, RevealEntity(parent));
            }
            write(world, SceneEdited);
        }
        Ok(Applied::Unchanged) => {}
        Err(error) => write(world, SceneStatus(format!("{operation} failed: {error:#}"))),
    }
}

fn apply(world: &mut World, edit: &EntityEdit) -> anyhow::Result<Applied> {
    match edit {
        EntityEdit::Create { parent } => {
            new_parent(world, *parent)?;
            let child = world.spawn((
                Name::new("Entity"),
                Transform::IDENTITY,
                SyncWithRenderWorld,
            ));
            if let Some(parent) = parent {
                world.entity_mut(*parent).add_child(child);
            }
            Ok(Applied::Changed(Some(child)))
        }
        EntityEdit::Delete(entity) => {
            let parent = editable(world, *entity)?;
            world.despawn(*entity);
            Ok(Applied::Removed(parent))
        }
        EntityEdit::Duplicate(entity) => {
            let parent = editable(world, *entity)?;
            let scene = capture_subtree(world, *entity)?;
            Ok(Applied::Changed(spawn_beside(world, &scene, parent)))
        }
        EntityEdit::Rename { entity, name } => {
            editable(world, *entity)?;
            let name = name.trim();
            if name.is_empty() {
                bail!("a name cannot be empty");
            }
            match world.get_component_for_entity_mut::<Name>(*entity) {
                Some(current) if current.as_str() == name => return Ok(Applied::Unchanged),
                Some(current) => current.set(name),
                None => world.insert(Name::new(name), *entity),
            }
            Ok(Applied::Changed(None))
        }
    }
}

fn has_scene_root(world: &mut World) -> bool {
    world.query::<&SceneRoot, ()>().iter(world).next().is_some()
}

fn exists(world: &World, entity: Entity) -> anyhow::Result<()> {
    if !world.entity_is_valid(entity) {
        bail!("the entity no longer exists");
    }
    Ok(())
}

fn new_parent(world: &mut World, parent: Option<Entity>) -> anyhow::Result<()> {
    if has_scene_root(world) {
        scene_member(
            world,
            parent.context("a new entity needs a parent in the scene")?,
        )?;
    } else if let Some(parent) = parent {
        exists(world, parent)?;
    }
    Ok(())
}

fn editable(world: &mut World, entity: Entity) -> anyhow::Result<Option<Entity>> {
    if has_scene_root(world) {
        return scene_node(world, entity).map(Some);
    }
    exists(world, entity)?;
    Ok(world
        .get_component_for_entity::<ChildOf>(entity)
        .map(ChildOf::parent))
}

fn spawn_beside(world: &mut World, scene: &Scene, parent: Option<Entity>) -> Option<Entity> {
    let holder = parent.unwrap_or_else(|| world.spawn(()));
    let spawned = spawn_scene_in_world(world, scene, holder);
    if parent.is_none() {
        for root in &spawned.root_entities {
            world.remove_component::<ChildOf>(*root);
        }
        world.despawn(holder);
    }
    spawned.node_entities.first().copied()
}

/// Whether `entity` is the scene root; an error if it is not part of an open scene at all.
fn scene_member(world: &World, entity: Entity) -> anyhow::Result<bool> {
    let mut current = entity;
    let mut is_root = true;
    while world.entity_is_valid(current) {
        if world
            .get_component_for_entity::<SceneRoot>(current)
            .is_some()
        {
            return Ok(is_root);
        }
        let Some(parent) = world.get_component_for_entity::<ChildOf>(current) else {
            break;
        };
        current = parent.parent();
        is_root = false;
    }
    bail!("the entity is not part of an open scene")
}

/// The parent of a scene entity that is not the scene root.
fn scene_node(world: &World, entity: Entity) -> anyhow::Result<Entity> {
    if scene_member(world, entity)? {
        bail!("the scene root cannot be changed");
    }
    world
        .get_component_for_entity::<ChildOf>(entity)
        .map(ChildOf::parent)
        .context("the entity has no parent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        scene::testing::{tools_app, written},
        selection::Selection,
    };
    use concerto_ecs::entity::hierarchy::Children;
    use concerto_foundation::assets::AssetId;
    use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};

    struct Fixture {
        app: App,
        root: Entity,
        node: Entity,
        child: Entity,
    }

    fn fixture() -> Fixture {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        world.register_component_type::<Transform>();
        world.register_component_type::<SyncWithRenderWorld>();
        world.register_component_type::<SkeletonComponent>();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.register_component::<Name>();
        let root = world.spawn((Transform::IDENTITY, SceneRoot));
        let node = world.spawn((Name::new("node"), Transform::IDENTITY));
        let child = world.spawn((Name::new("child"), Transform::IDENTITY));
        world.entity_mut(root).add_child(node);
        world.entity_mut(node).add_child(child);
        Fixture {
            app,
            root,
            node,
            child,
        }
    }

    impl Fixture {
        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }
        fn trigger(&mut self, edit: EntityEdit) {
            self.world().trigger(edit);
        }
        fn edits(&self) -> usize {
            written::<SceneEdited>(&self.app).len()
        }
        fn statuses(&self) -> Vec<String> {
            written::<SceneStatus>(&self.app)
                .into_iter()
                .map(|status| status.0)
                .collect()
        }
        fn selected(&self) -> Option<Entity> {
            self.app.get_resource::<Selection>().unwrap().entity()
        }
        fn children(&mut self, entity: Entity) -> Vec<Entity> {
            self.world()
                .get_component_for_entity::<Children>(entity)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }
        fn name(&mut self, entity: Entity) -> String {
            self.world()
                .get_component_for_entity::<Name>(entity)
                .unwrap()
                .as_str()
                .to_owned()
        }
        fn valid(&mut self, entity: Entity) -> bool {
            self.world().entity_is_valid(entity)
        }
    }

    #[test]
    fn an_applied_edit_writes_scene_edited_and_reveals_what_it_created() {
        let mut f = fixture();

        f.trigger(EntityEdit::Create {
            parent: Some(f.root),
        });

        assert_eq!(f.edits(), 1);
        let revealed = written::<RevealEntity>(&f.app);
        assert_eq!(revealed.len(), 1);
        assert_eq!(f.selected(), Some(revealed[0].0));
    }

    #[test]
    fn a_refused_edit_writes_a_status_and_no_scene_edited() {
        let mut f = fixture();

        f.trigger(EntityEdit::Delete(f.root));

        assert_eq!(f.edits(), 0);
        let status = f.statuses();
        assert_eq!(status.len(), 1);
        assert!(status[0].starts_with("Delete failed"));
    }

    #[test]
    fn create_adds_a_selected_last_child_and_writes_scene_edited() {
        let mut f = fixture();
        f.trigger(EntityEdit::Create {
            parent: Some(f.node),
        });
        let created = f.selected().unwrap();
        assert_eq!(f.children(f.node), [f.child, created]);
        assert_eq!(f.name(created), "Entity");
        assert!(
            f.world()
                .get_component_for_entity::<Transform>(created)
                .is_some()
        );
        assert!(
            f.world()
                .get_component_for_entity::<SyncWithRenderWorld>(created)
                .is_some()
        );
        assert_eq!(f.edits(), 1);

        f.trigger(EntityEdit::Create {
            parent: Some(f.root),
        });
        let second = f.selected().unwrap();
        assert_eq!(f.children(f.root), [f.node, second]);
    }

    #[test]
    fn an_entity_outside_any_scene_is_rejected() {
        let mut f = fixture();
        let stray = f.world().spawn(Name::new("stray"));
        let count = |f: &mut Fixture| {
            let world = f.world();
            world.query::<Entity, ()>().iter(world).count()
        };
        let before = count(&mut f);
        for edit in [
            EntityEdit::Create {
                parent: Some(stray),
            },
            EntityEdit::Delete(stray),
            EntityEdit::Duplicate(stray),
            EntityEdit::Rename {
                entity: stray,
                name: "other".into(),
            },
        ] {
            f.trigger(edit);
        }
        assert_eq!(count(&mut f), before);
        assert_eq!(f.name(stray), "stray");
        assert_eq!(f.edits(), 0);
        assert_eq!(f.statuses().len(), 4);
        assert_eq!(f.selected(), None);
    }

    #[test]
    fn delete_removes_the_subtree_and_selects_the_parent() {
        let mut f = fixture();
        f.trigger(EntityEdit::Delete(f.node));
        assert!(!f.valid(f.node));
        assert!(!f.valid(f.child));
        assert_eq!(f.selected(), Some(f.root));
        assert_eq!(f.edits(), 1);
    }

    #[test]
    fn the_scene_root_cannot_be_deleted_duplicated_or_renamed() {
        let mut f = fixture();
        for (refused, edit) in [
            EntityEdit::Delete(f.root),
            EntityEdit::Duplicate(f.root),
            EntityEdit::Rename {
                entity: f.root,
                name: "other".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            f.trigger(edit);
            let statuses = f.statuses();
            assert_eq!(statuses.len(), refused + 1);
            assert!(
                statuses[refused].contains("failed"),
                "{}",
                statuses[refused]
            );
        }
        assert!(f.valid(f.root));
        assert_eq!(f.children(f.root), [f.node]);
        let root = f.root;
        assert!(f.world().get_component_for_entity::<Name>(root).is_none());
        assert_eq!(f.edits(), 0);
    }

    #[test]
    fn duplicate_copies_the_subtree_beside_the_source_and_selects_the_copy() {
        let mut f = fixture();
        let child = f.child;
        f.world()
            .get_component_for_entity_mut::<Transform>(child)
            .unwrap()
            .translation
            .x = 7.0;
        f.trigger(EntityEdit::Duplicate(f.node));
        let copy = f.selected().unwrap();
        assert_ne!(copy, f.node);
        assert_eq!(f.children(f.root), [f.node, copy]);
        assert_eq!(f.name(copy), "node");
        let copied_child = f.children(copy)[0];
        assert_ne!(copied_child, f.child);
        assert_eq!(f.name(copied_child), "child");
        assert_eq!(
            f.world()
                .get_component_for_entity::<Transform>(copied_child)
                .unwrap()
                .translation
                .x,
            7.0
        );
        assert_eq!(f.edits(), 1);
    }

    #[test]
    fn a_duplicate_that_refers_outside_its_subtree_spawns_nothing_and_says_why() {
        let mut f = fixture();
        let (root, child) = (f.root, f.child);
        let bone = f.world().spawn(Name::new("bone"));
        f.world().entity_mut(root).add_child(bone);
        f.world().insert(
            SkeletonComponent {
                skeleton: concerto_foundation::assets::handle::AssetHandle::<Skeleton>::weak(
                    AssetId::new(),
                ),
                bones: vec![concerto_ecs::component::scene::SceneEntityRef::Entity(bone)],
                bone_ids: vec![],
                root: None,
            },
            child,
        );
        f.trigger(EntityEdit::Duplicate(f.node));
        assert_eq!(f.children(f.root), [f.node, bone]);
        let statuses = f.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(
            statuses[0].starts_with("Duplicate failed: "),
            "{}",
            statuses[0]
        );
        assert_eq!(f.edits(), 0);
        assert_eq!(f.selected(), None);
    }

    #[test]
    fn rename_sets_a_trimmed_name_and_ignores_empty_or_unchanged_ones() {
        let mut f = fixture();
        f.trigger(EntityEdit::Rename {
            entity: f.node,
            name: "node".into(),
        });
        assert_eq!(f.edits(), 0, "an unchanged name is not an edit");
        assert!(f.statuses().is_empty());

        f.trigger(EntityEdit::Rename {
            entity: f.node,
            name: "   ".into(),
        });
        assert_eq!(f.name(f.node), "node");
        assert!(f.statuses()[0].starts_with("Rename failed: "));
        assert_eq!(f.edits(), 0);

        f.trigger(EntityEdit::Rename {
            entity: f.node,
            name: "  lamp ".into(),
        });
        assert_eq!(f.name(f.node), "lamp");
        assert_eq!(f.edits(), 1);

        let root = f.root;
        let unnamed = f.world().spawn(());
        f.world().entity_mut(root).add_child(unnamed);
        f.trigger(EntityEdit::Rename {
            entity: unnamed,
            name: "fresh".into(),
        });
        assert_eq!(f.name(unnamed), "fresh");
    }

    fn play_fixture() -> Fixture {
        let mut f = fixture();
        let root = f.root;
        f.world().remove_component::<SceneRoot>(root);
        f
    }

    fn parent_of(f: &mut Fixture, entity: Entity) -> Option<Entity> {
        f.world()
            .get_component_for_entity::<ChildOf>(entity)
            .map(ChildOf::parent)
    }

    #[test]
    fn in_a_scene_create_without_a_parent_is_refused() {
        let mut f = fixture();

        f.trigger(EntityEdit::Create { parent: None });

        assert_eq!(f.edits(), 0);
        let statuses = f.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].starts_with("Create failed"), "{}", statuses[0]);
        assert_eq!(f.selected(), None);
        assert_eq!(f.children(f.root), [f.node]);
    }

    #[test]
    fn without_a_scene_root_create_without_a_parent_adds_a_selected_top_level_entity() {
        let mut f = play_fixture();

        f.trigger(EntityEdit::Create { parent: None });

        let created = f.selected().unwrap();
        assert_ne!(created, f.root);
        assert_eq!(parent_of(&mut f, created), None);
        assert_eq!(f.name(created), "Entity");
        assert_eq!(f.edits(), 1);

        f.trigger(EntityEdit::Create {
            parent: Some(f.root),
        });
        let child = f.selected().unwrap();
        assert_eq!(f.children(f.root), [f.node, child]);
        assert!(f.statuses().is_empty());
    }

    #[test]
    fn without_a_scene_root_a_top_level_entity_can_be_renamed_duplicated_and_deleted() {
        let mut f = play_fixture();

        f.trigger(EntityEdit::Rename {
            entity: f.root,
            name: "level".into(),
        });
        assert_eq!(f.name(f.root), "level");

        f.trigger(EntityEdit::Duplicate(f.root));
        let copy = f.selected().unwrap();
        assert_ne!(copy, f.root);
        assert_eq!(parent_of(&mut f, copy), None);
        assert_eq!(f.name(copy), "level");
        assert_eq!(f.children(copy).len(), 1);
        assert_eq!(f.children(f.root), [f.node]);

        f.trigger(EntityEdit::Delete(f.root));
        assert!(!f.valid(f.root));
        assert!(!f.valid(f.node));
        assert!(!f.valid(f.child));
        assert!(f.valid(copy));
        assert_eq!(f.selected(), None);
        assert_eq!(f.edits(), 3);
        assert!(f.statuses().is_empty());

        f.trigger(EntityEdit::Rename {
            entity: f.root,
            name: "gone".into(),
        });
        let statuses = f.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].starts_with("Rename failed"), "{}", statuses[0]);
        assert_eq!(f.edits(), 3);
    }

    #[test]
    fn without_a_scene_root_deleting_a_child_selects_its_parent() {
        let mut f = play_fixture();

        f.trigger(EntityEdit::Delete(f.node));

        assert!(!f.valid(f.node));
        assert!(!f.valid(f.child));
        assert_eq!(f.selected(), Some(f.root));
        assert_eq!(f.edits(), 1);
    }

    #[test]
    fn without_a_scene_root_a_top_level_duplicate_adds_only_the_copied_subtree() {
        let mut f = play_fixture();
        let count = |f: &mut Fixture| {
            let world = f.world();
            world.query::<Entity, ()>().iter(world).count()
        };
        let before = count(&mut f);

        f.trigger(EntityEdit::Duplicate(f.root));

        assert_eq!(count(&mut f), before + 3);
        let copy = f.selected().unwrap();
        assert_eq!(parent_of(&mut f, copy), None);
    }
}

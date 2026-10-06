//! Create, delete, duplicate and rename the entities of an open scene.
use anyhow::{Context, bail};
use concerto_ecs::{
    Entity, World,
    component::name::Name,
    entity::hierarchy::ChildOf,
    signal::{On, Signal},
};
use concerto_foundation::transform::Transform;
use concerto_render::components::render_entity::SyncWithRenderWorld;
use concerto_scene::{capture::capture_subtree, spawner::spawn_scene_in_world};

use crate::{
    asset_editor::{EditorDocument, mark_entity_edited, owning_document},
    hierarchy::HierarchyState,
    scene::SceneRoot,
    selection::Selection,
};

/// A structural change to an open scene. Trigger it; one listener applies it.
pub enum EntityEdit {
    /// Adds an empty entity as the last child of `parent`.
    Create {
        parent: Entity,
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

pub(crate) fn apply_entity_edit(on: On<EntityEdit>, world: &mut World) {
    let (operation, target) = match on.signal() {
        EntityEdit::Create { parent } => ("Create", *parent),
        EntityEdit::Delete(entity) => ("Delete", *entity),
        EntityEdit::Duplicate(entity) => ("Duplicate", *entity),
        EntityEdit::Rename { entity, .. } => ("Rename", *entity),
    };
    let document = owning_document(world, target);
    match apply(world, on.signal()) {
        Ok(Some(selected)) => {
            if let Some(selection) = world.get_resource_mut::<Selection>() {
                selection.select_entity(selected);
            }
            if let Some(hierarchy) = world.get_resource_mut::<HierarchyState>() {
                hierarchy.reveal_entity(selected);
            }
        }
        Ok(None) => {}
        Err(error) => {
            let status = format!("{operation} failed: {error:#}");
            match document
                .and_then(|document| world.get_component_for_entity_mut::<EditorDocument>(document))
            {
                Some(document) => document.status = status,
                None => log::warn!("{status}"),
            }
        }
    }
}

fn apply(world: &mut World, edit: &EntityEdit) -> anyhow::Result<Option<Entity>> {
    match edit {
        EntityEdit::Create { parent } => {
            scene_member(world, *parent)?;
            let child = world.spawn((
                Name::new("Entity"),
                Transform::IDENTITY,
                SyncWithRenderWorld,
            ));
            world.entity_mut(*parent).add_child(child);
            mark_entity_edited(world, *parent);
            Ok(Some(child))
        }
        EntityEdit::Delete(entity) => {
            let parent = scene_node(world, *entity)?;
            mark_entity_edited(world, parent);
            world.despawn(*entity);
            Ok(Some(parent))
        }
        EntityEdit::Duplicate(entity) => {
            let parent = scene_node(world, *entity)?;
            let scene = capture_subtree(world, *entity)?;
            let spawned = spawn_scene_in_world(world, &scene, parent);
            mark_entity_edited(world, parent);
            Ok(spawned.node_entities.first().copied())
        }
        EntityEdit::Rename { entity, name } => {
            scene_node(world, *entity)?;
            let name = name.trim();
            if name.is_empty() {
                bail!("a name cannot be empty");
            }
            match world.get_component_for_entity_mut::<Name>(*entity) {
                Some(current) if current.as_str() == name => return Ok(None),
                Some(current) => current.set(name),
                None => world.insert(Name::new(name), *entity),
            }
            mark_entity_edited(world, *entity);
            Ok(None)
        }
    }
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
    use crate::asset_editor::EditorOwned;
    use concerto_ecs::entity::hierarchy::Children;
    use concerto_foundation::assets::AssetId;
    use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};

    struct Fixture {
        world: World,
        document: Entity,
        root: Entity,
        node: Entity,
        child: Entity,
    }

    fn fixture() -> Fixture {
        let mut world = World::new();
        world.register_component_type::<Transform>();
        world.register_component_type::<SyncWithRenderWorld>();
        world.register_component_type::<SkeletonComponent>();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.register_component::<Name>();
        world.insert_resource(Selection::default());
        world.insert_resource(HierarchyState::default());
        world.add_listener(apply_entity_edit);
        let document = world.spawn(EditorDocument {
            asset_type: "Scene",
            title: "level".into(),
            current: None,
            pending: None,
            project_generation: 0,
            request_generation: 0,
            order: 0,
            status: String::new(),
            revision: 0,
            saved_revision: 0,
        });
        let root = world.spawn((
            Transform::IDENTITY,
            SceneRoot {
                asset_id: AssetId::new(),
                address: "level.gasset".into(),
            },
            EditorOwned(document),
        ));
        let node = world.spawn((Name::new("node"), Transform::IDENTITY));
        let child = world.spawn((Name::new("child"), Transform::IDENTITY));
        world.entity_mut(root).add_child(node);
        world.entity_mut(node).add_child(child);
        Fixture {
            world,
            document,
            root,
            node,
            child,
        }
    }

    impl Fixture {
        fn doc(&self) -> &EditorDocument {
            self.world
                .get_component_for_entity::<EditorDocument>(self.document)
                .unwrap()
        }
        fn selected(&self) -> Option<Entity> {
            self.world.get_resource::<Selection>().unwrap().entity()
        }
        fn children(&self, entity: Entity) -> Vec<Entity> {
            self.world
                .get_component_for_entity::<Children>(entity)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }
        fn name(&self, entity: Entity) -> &str {
            self.world
                .get_component_for_entity::<Name>(entity)
                .unwrap()
                .as_str()
        }
    }

    #[test]
    fn create_adds_a_selected_last_child_and_marks_the_document() {
        let mut f = fixture();
        f.world.trigger(EntityEdit::Create { parent: f.node });
        let created = f.selected().unwrap();
        assert_eq!(f.children(f.node), [f.child, created]);
        assert_eq!(f.name(created), "Entity");
        assert!(
            f.world
                .get_component_for_entity::<Transform>(created)
                .is_some()
        );
        assert!(
            f.world
                .get_component_for_entity::<SyncWithRenderWorld>(created)
                .is_some()
        );
        assert!(f.doc().is_dirty());

        f.world.trigger(EntityEdit::Create { parent: f.root });
        assert_eq!(f.children(f.root), [f.node, f.selected().unwrap()]);
    }

    #[test]
    fn an_entity_outside_any_scene_is_rejected() {
        let mut f = fixture();
        let stray = f.world.spawn(Name::new("stray"));
        let count = f.world.query::<Entity, ()>().iter(&mut f.world).count();
        for edit in [
            EntityEdit::Create { parent: stray },
            EntityEdit::Delete(stray),
            EntityEdit::Duplicate(stray),
            EntityEdit::Rename {
                entity: stray,
                name: "other".into(),
            },
        ] {
            f.world.trigger(edit);
        }
        assert_eq!(
            f.world.query::<Entity, ()>().iter(&mut f.world).count(),
            count
        );
        assert_eq!(f.name(stray), "stray");
        assert!(!f.doc().is_dirty());
        assert_eq!(f.selected(), None);
    }

    #[test]
    fn delete_removes_the_subtree_and_selects_the_parent() {
        let mut f = fixture();
        f.world.trigger(EntityEdit::Delete(f.node));
        assert!(!f.world.entity_is_valid(f.node));
        assert!(!f.world.entity_is_valid(f.child));
        assert_eq!(f.selected(), Some(f.root));
        assert!(f.doc().is_dirty());
    }

    #[test]
    fn the_scene_root_cannot_be_deleted_duplicated_or_renamed() {
        let mut f = fixture();
        for edit in [
            EntityEdit::Delete(f.root),
            EntityEdit::Duplicate(f.root),
            EntityEdit::Rename {
                entity: f.root,
                name: "other".into(),
            },
        ] {
            f.world.trigger(edit);
            assert!(f.doc().status.contains("failed"), "{}", f.doc().status);
        }
        assert!(f.world.entity_is_valid(f.root));
        assert_eq!(f.children(f.root), [f.node]);
        assert!(f.world.get_component_for_entity::<Name>(f.root).is_none());
        assert!(!f.doc().is_dirty());
    }

    #[test]
    fn duplicate_copies_the_subtree_beside_the_source_and_selects_the_copy() {
        let mut f = fixture();
        f.world
            .get_component_for_entity_mut::<Transform>(f.child)
            .unwrap()
            .translation
            .x = 7.0;
        f.world.trigger(EntityEdit::Duplicate(f.node));
        let copy = f.selected().unwrap();
        assert_ne!(copy, f.node);
        assert_eq!(f.children(f.root), [f.node, copy]);
        assert_eq!(f.name(copy), "node");
        let copied_child = f.children(copy)[0];
        assert_ne!(copied_child, f.child);
        assert_eq!(f.name(copied_child), "child");
        assert_eq!(
            f.world
                .get_component_for_entity::<Transform>(copied_child)
                .unwrap()
                .translation
                .x,
            7.0
        );
        assert!(f.doc().is_dirty());
    }

    #[test]
    fn a_duplicate_that_refers_outside_its_subtree_spawns_nothing_and_says_why() {
        let mut f = fixture();
        let bone = f.world.spawn(Name::new("bone"));
        f.world.entity_mut(f.root).add_child(bone);
        f.world.insert(
            SkeletonComponent {
                skeleton: concerto_foundation::assets::handle::AssetHandle::<Skeleton>::weak(
                    AssetId::new(),
                ),
                bones: vec![concerto_ecs::component::scene::SceneEntityRef::Entity(bone)],
                bone_ids: vec![],
                root: None,
            },
            f.child,
        );
        f.world.trigger(EntityEdit::Duplicate(f.node));
        assert_eq!(f.children(f.root), [f.node, bone]);
        assert!(
            f.doc().status.starts_with("Duplicate failed: "),
            "{}",
            f.doc().status
        );
        assert!(!f.doc().is_dirty());
        assert_eq!(f.selected(), None);
    }

    #[test]
    fn rename_sets_a_trimmed_name_and_ignores_empty_or_unchanged_ones() {
        let mut f = fixture();
        f.world.trigger(EntityEdit::Rename {
            entity: f.node,
            name: "node".into(),
        });
        assert!(!f.doc().is_dirty(), "an unchanged name is not an edit");

        f.world.trigger(EntityEdit::Rename {
            entity: f.node,
            name: "   ".into(),
        });
        assert_eq!(f.name(f.node), "node");
        assert!(f.doc().status.starts_with("Rename failed: "));
        assert!(!f.doc().is_dirty());

        f.world.trigger(EntityEdit::Rename {
            entity: f.node,
            name: "  lamp ".into(),
        });
        assert_eq!(f.name(f.node), "lamp");
        assert!(f.doc().is_dirty());

        let unnamed = f.world.spawn(());
        f.world.entity_mut(f.root).add_child(unnamed);
        f.world.trigger(EntityEdit::Rename {
            entity: unnamed,
            name: "fresh".into(),
        });
        assert_eq!(f.name(unnamed), "fresh");
    }
}

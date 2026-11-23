//! Create, delete, duplicate and rename the entities of an open scene, or of a running game's world.
use anyhow::{Context, bail};
use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    Entity, World,
    component::name::Name,
    entity::hierarchy::ChildOf,
    events::Event,
    signal::{On, Signal},
};
use concerto_foundation::transform::Transform;
use concerto_render::components::render_entity::SyncWithRenderWorld;
use concerto_scene::{
    capture::capture_subtree,
    node_id::{NodeId, SceneNodeId},
    scene::Scene,
    spawner::{SpawnFilter, spawn_scene_in_world},
};

use crate::tools::{
    links::{LinkedEdit, record_removal, refusal},
    scene::{RevealEntity, SceneRoot, SceneStatus, report_edit, write},
    selection::Select,
};

/// A structural change to an open scene, or to a game's world, which has no scene root. Trigger it; one listener applies it.
#[derive(Clone, Debug, PartialEq)]
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

/// What one `EntityEdit` did: the entity it created or duplicated, or the status text saying why it was refused.
#[derive(Event, Clone)]
pub struct EntityEdited {
    pub edit: EntityEdit,
    pub result: Result<Option<Entity>, String>,
}

pub(crate) fn build(app: &mut App) {
    app.register_event::<EntityEdited>()
        .add_listener(apply_entity_edit.at_edit_time());
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
    let result = match apply(world, on.signal()) {
        Ok(Applied::Changed(selected)) => {
            if let Some(selected) = selected {
                world.trigger(Select::only(selected));
                write(world, RevealEntity(selected));
            }
            report_edit(world);
            Ok(selected)
        }
        Ok(Applied::Removed(parent)) => {
            world.trigger(Select::Replace(parent.into_iter().collect()));
            if let Some(parent) = parent {
                write(world, RevealEntity(parent));
            }
            report_edit(world);
            Ok(None)
        }
        Ok(Applied::Unchanged) => Ok(None),
        Err(error) => {
            let status = format!("{operation} failed: {error:#}");
            write(world, SceneStatus(status.clone()));
            Err(status)
        }
    };
    write(
        world,
        EntityEdited {
            edit: on.signal().clone(),
            result,
        },
    );
}

fn apply(world: &mut World, edit: &EntityEdit) -> anyhow::Result<Applied> {
    match edit {
        EntityEdit::Create { parent } => {
            new_parent(world, *parent)?;
            let child = world.spawn((
                Name::new("Entity"),
                Transform::IDENTITY,
                SyncWithRenderWorld,
                SceneNodeId(NodeId::fresh()),
            ));
            if let Some(parent) = parent {
                world.entity_mut(*parent).add_child(child);
            }
            Ok(Applied::Changed(Some(child)))
        }
        EntityEdit::Delete(entity) => {
            let parent = editable(world, *entity)?;
            record_removal(world, *entity);
            world.despawn(*entity);
            Ok(Applied::Removed(parent))
        }
        EntityEdit::Duplicate(entity) => {
            let parent = editable(world, *entity)?;
            refuse_linked(world, *entity, LinkedEdit::Duplicate)?;
            let mut scene = capture_subtree(world, *entity)?;
            for node in &mut scene.nodes {
                node.id = NodeId::fresh();
            }
            Ok(Applied::Changed(spawn_beside(world, &scene, parent)))
        }
        EntityEdit::Rename { entity, name } => {
            editable(world, *entity)?;
            refuse_linked(world, *entity, LinkedEdit::Rename)?;
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

fn refuse_linked(world: &World, entity: Entity, edit: LinkedEdit) -> anyhow::Result<()> {
    match refusal(world, entity, edit) {
        Some(reason) => bail!(reason),
        None => Ok(()),
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

pub(crate) fn new_parent(world: &mut World, parent: Option<Entity>) -> anyhow::Result<()> {
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
    let spawned = spawn_scene_in_world(world, scene, holder, &SpawnFilter::default());
    if parent.is_none() {
        for root in &spawned.root_entities {
            world.remove_component::<ChildOf>(*root);
        }
        world.despawn(holder);
    }
    spawned.node_entities.first().copied()
}

/// Refuses a live entity outside the open scene; in a world without a scene root, as a running game's, every entity passes.
pub(crate) fn in_open_scene(world: &mut World, entity: Entity) -> anyhow::Result<()> {
    if world.entity_is_valid(entity) && has_scene_root(world) {
        scene_member(world, entity)?;
    }
    Ok(())
}

/// Whether `entity` is the scene root; an error if it is not part of an open scene at all.
pub(crate) fn scene_member(world: &World, entity: Entity) -> anyhow::Result<bool> {
    scene_member_by(entity, |entity| {
        world.entity_is_valid(entity).then(|| {
            (
                world
                    .get_component_for_entity::<SceneRoot>(entity)
                    .is_some(),
                world
                    .get_component_for_entity::<ChildOf>(entity)
                    .map(ChildOf::parent),
            )
        })
    })
}

/// `scene_member` over `node`, which gives a live entity's root mark and parent, and `None` for a dead one.
pub(crate) fn scene_member_by(
    entity: Entity,
    node: impl Fn(Entity) -> Option<(bool, Option<Entity>)>,
) -> anyhow::Result<bool> {
    let mut current = entity;
    let mut is_root = true;
    while let Some((root, parent)) = node(current) {
        if root {
            return Ok(is_root);
        }
        let Some(parent) = parent else {
            break;
        };
        current = parent;
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
        scene::{
            SceneEdited,
            testing::{tools_app, written},
        },
        selection::Selection,
    };
    use concerto_ecs::entity::hierarchy::Children;

    struct Fixture {
        app: App,
        root: Entity,
        node: Entity,
        child: Entity,
    }

    fn fixture() -> Fixture {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        world.register_type::<Transform>();
        world.register_type::<SyncWithRenderWorld>();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.register_component::<Name>();
        let root = world.spawn((Transform::IDENTITY, SceneRoot));
        let node = world.spawn((
            Name::new("node"),
            Transform::IDENTITY,
            SceneNodeId(NodeId(1)),
        ));
        let child = world.spawn((
            Name::new("child"),
            Transform::from_translation(glam::Vec3::X * 7.0),
            SceneNodeId(NodeId(2)),
        ));
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
        fn edit(&mut self, edit: EntityEdit) -> Result<Option<Entity>, String> {
            self.world().trigger(edit);
            written::<EntityEdited>(&self.app).pop().unwrap().result
        }
        fn edits(&self) -> usize {
            written::<SceneEdited>(&self.app).len()
        }
        fn selected(&self) -> Option<Entity> {
            self.app.get_resource::<Selection>().unwrap().active()
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
        fn node_id(&mut self, entity: Entity) -> Option<NodeId> {
            self.world()
                .get_component_for_entity::<SceneNodeId>(entity)
                .map(|id| id.0)
        }
    }

    #[test]
    fn entities_are_created_duplicated_renamed_and_deleted_in_a_scene_and_in_a_game_world() {
        let mut f = fixture();
        let (root, node, child) = (f.root, f.node, f.child);

        let created = f
            .edit(EntityEdit::Create { parent: Some(node) })
            .unwrap()
            .unwrap();
        assert_eq!(f.children(node), [child, created]);
        assert_eq!(f.name(created), "Entity");
        assert!(
            f.node_id(created)
                .is_some_and(|id| id != NodeId(1) && id != NodeId(2))
        );
        assert!(
            f.world()
                .get_component_for_entity::<SyncWithRenderWorld>(created)
                .is_some()
        );
        assert_eq!(f.selected(), Some(created));
        assert_eq!(
            written::<RevealEntity>(&f.app)
                .into_iter()
                .map(|reveal| reveal.0)
                .collect::<Vec<_>>(),
            [created]
        );

        let copy = f.edit(EntityEdit::Duplicate(node)).unwrap().unwrap();
        assert_eq!(f.children(root), [node, copy]);
        assert_eq!(f.selected(), Some(copy));
        let copied_child = f.children(copy)[0];
        assert_ne!(copied_child, child);
        assert_eq!(f.name(copied_child), "child");
        let copied = [f.node_id(copy), f.node_id(copied_child)];
        assert!(
            copied
                .iter()
                .all(|id| id.is_some_and(|id| id != NodeId(1) && id != NodeId(2)))
        );
        assert_ne!(copied[0], copied[1]);
        assert_eq!(
            f.world()
                .get_component_for_entity::<Transform>(copied_child)
                .unwrap()
                .translation
                .x,
            7.0
        );

        f.edit(EntityEdit::Rename {
            entity: copy,
            name: "  lamp ".into(),
        })
        .unwrap();
        assert_eq!(f.name(copy), "lamp");

        f.edit(EntityEdit::Delete(node)).unwrap();
        assert!(!f.valid(node) && !f.valid(child) && !f.valid(created));
        assert_eq!(f.selected(), Some(root));
        assert_eq!(f.edits(), 4);

        assert!(f.edit(EntityEdit::Delete(root)).is_err());
        assert!(f.edit(EntityEdit::Create { parent: None }).is_err());
        assert!(f.valid(root));
        assert_eq!(f.edits(), 4);

        f.world().remove_component::<SceneRoot>(root);
        let top = f
            .edit(EntityEdit::Create { parent: None })
            .unwrap()
            .unwrap();
        assert!(f.world().get_component_for_entity::<ChildOf>(top).is_none());
        f.edit(EntityEdit::Delete(root)).unwrap();
        assert!(!f.valid(root) && !f.valid(copy));
        assert!(f.valid(top));
        assert_eq!(f.edits(), 6);
    }
}

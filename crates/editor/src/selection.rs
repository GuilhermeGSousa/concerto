//! The one thing every panel agrees on.
//!
//! Panels never reference each other: the hierarchy and viewport write here,
//! the inspector and viewport read. That is what makes a panel removable.
use ecs::{entity::Entity, resource::Resource};
use essential::assets::AssetId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// A live entity in the world.
    Entity(Entity),
    /// An asset in the project catalogue, which may not be spawned at all.
    Asset(AssetId),
}

#[derive(Resource, Default)]
pub struct Selection {
    current: Option<SelectionKind>,
    revision: u64,
}

impl Selection {
    pub fn current(&self) -> Option<SelectionKind> {
        self.current
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn entity(&self) -> Option<Entity> {
        match self.current {
            Some(SelectionKind::Entity(entity)) => Some(entity),
            _ => None,
        }
    }

    pub fn asset(&self) -> Option<AssetId> {
        match self.current {
            Some(SelectionKind::Asset(id)) => Some(id),
            _ => None,
        }
    }

    pub fn is(&self, kind: SelectionKind) -> bool {
        self.current == Some(kind)
    }

    pub fn set(&mut self, kind: SelectionKind) {
        self.replace(Some(kind));
    }

    pub fn select_entity(&mut self, entity: Entity) {
        self.set(SelectionKind::Entity(entity));
    }

    pub fn select_asset(&mut self, id: AssetId) {
        self.set(SelectionKind::Asset(id));
    }

    pub fn clear(&mut self) {
        self.replace(None);
    }

    /// Clears the selection only if it names `entity` — for use when an entity
    /// is about to be despawned, so the selection never dangles.
    pub fn clear_entity(&mut self, entity: Entity) {
        if self.entity() == Some(entity) {
            self.clear();
        }
    }

    fn replace(&mut self, next: Option<SelectionKind>) {
        if self.current != next {
            self.current = next;
            self.revision += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecs::World;

    #[test]
    fn revision_only_moves_on_a_real_change() {
        let mut world = World::default();
        let entity = world.spawn(());
        let mut selection = Selection::default();

        selection.select_entity(entity);
        let after_first = selection.revision();
        assert!(
            after_first > 0,
            "selecting something must bump the revision"
        );

        selection.select_entity(entity);
        assert_eq!(
            selection.revision(),
            after_first,
            "re-selecting the same thing must not invalidate every panel"
        );
    }

    #[test]
    fn clearing_a_different_entity_leaves_the_selection_alone() {
        let mut world = World::default();
        let kept = world.spawn(());
        let other = world.spawn(());
        let mut selection = Selection::default();
        selection.select_entity(kept);

        selection.clear_entity(other);

        assert_eq!(
            selection.entity(),
            Some(kept),
            "despawning an unrelated entity must not clear the selection"
        );
    }

    #[test]
    fn clearing_the_selected_entity_drops_it() {
        let mut world = World::default();
        let doomed = world.spawn(());
        let mut selection = Selection::default();
        selection.select_entity(doomed);

        selection.clear_entity(doomed);

        assert_eq!(
            selection.current(),
            None,
            "a selection must never outlive the entity it names"
        );
    }
}

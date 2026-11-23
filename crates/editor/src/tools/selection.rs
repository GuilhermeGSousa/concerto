//! The one thing every panel agrees on.
use concerto_app::{App, edit_time::AtEditTime, schedule_groups::Update};
use concerto_ecs::{
    Query, ResMut,
    entity::{Entity, hierarchy::ChildOf},
    resource::Resource,
    signal::{On, Signal},
};

use crate::tools::scene::SceneRoot;

/// The selected entities of the scene in the order they were selected; the last is the active one, which stands for the selection where one entity is needed.
#[derive(Resource, Default)]
pub struct Selection {
    entities: Vec<Entity>,
}

impl Selection {
    /// The last entity selected: the one the gizmo sits on and Rename and Add child act on.
    pub fn active(&self) -> Option<Entity> {
        self.entities.last().copied()
    }

    /// Every selected entity, in the order selected.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(&entity)
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// The selected entities with no selected ancestor, in the order selected.
    pub fn roots(&self, parents: &Query<&ChildOf>) -> Vec<Entity> {
        self.entities
            .iter()
            .copied()
            .filter(|&entity| {
                let mut at = entity;
                while let Some(parent) = parents.get_entity(at) {
                    at = parent.parent();
                    if self.contains(at) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }
}

/// Changes the selection of the scene.
pub enum Select {
    /// Selects exactly these entities, the last one active; an empty list clears the selection.
    Replace(Vec<Entity>),
    /// Adds the entity as the active one, or removes it when it is selected already.
    Toggle(Entity),
}

impl Select {
    /// Selects this entity alone.
    pub fn only(entity: Entity) -> Self {
        Select::Replace(vec![entity])
    }

    /// Clears the selection.
    pub fn none() -> Self {
        Select::Replace(Vec::new())
    }
}

impl Signal for Select {}

fn selected_after(
    current: &[Entity],
    select: &Select,
    is_root: impl Fn(Entity) -> bool,
) -> Vec<Entity> {
    let mut list = match select {
        Select::Replace(entities) => entities.clone(),
        Select::Toggle(entity) => {
            let mut list = current.to_vec();
            match list.iter().position(|selected| selected == entity) {
                Some(at) => {
                    list.remove(at);
                }
                None => list.push(*entity),
            }
            list
        }
    };
    let mut kept = Vec::with_capacity(list.len());
    while let Some(entity) = list.pop() {
        if !kept.contains(&entity) {
            kept.push(entity);
        }
    }
    kept.reverse();
    if kept.len() > 1 {
        kept.retain(|&entity| !is_root(entity));
    }
    kept
}

pub(crate) fn select(on: On<Select>, mut selection: ResMut<Selection>, roots: Query<&SceneRoot>) {
    let next = selected_after(&selection.entities, on.signal(), |entity| {
        roots.contains_entity(entity)
    });
    if next != selection.entities {
        selection.entities = next;
    }
}

fn forget_despawned_selection(mut selection: ResMut<Selection>, alive: Query<Entity>) {
    if selection
        .entities
        .iter()
        .any(|&entity| !alive.contains_entity(entity))
    {
        selection
            .entities
            .retain(|&entity| alive.contains_entity(entity));
    }
}

pub(crate) fn build(app: &mut App) {
    app.insert_resource(Selection::default())
        .add_listener(select.at_edit_time())
        .add_system(Update, forget_despawned_selection.at_edit_time());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::scene::testing::tools_app;
    use concerto_ecs::{IntoSystem, Res, System};

    struct Case {
        name: &'static str,
        start: Vec<usize>,
        signal: fn(&[Entity]) -> Select,
        expected: Vec<usize>,
    }

    fn pick(all: &[Entity], at: &[usize]) -> Vec<Entity> {
        at.iter().map(|&index| all[index]).collect()
    }

    #[test]
    fn selection_follows_replace_and_toggle_drops_the_root_from_several_and_forgets_despawned_entities()
     {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        let root = world.spawn(SceneRoot);
        let a = world.spawn(());
        let b = world.spawn(());
        let c = world.spawn(());
        let all = [root, a, b, c];

        let cases = [
            Case {
                name: "replace sets the list, last active",
                start: vec![],
                signal: |all| Select::Replace(vec![all[1], all[2]]),
                expected: vec![1, 2],
            },
            Case {
                name: "replace with a duplicate keeps the later position",
                start: vec![],
                signal: |all| Select::Replace(vec![all[1], all[2], all[1]]),
                expected: vec![2, 1],
            },
            Case {
                name: "toggle adds as active",
                start: vec![1],
                signal: |all| Select::Toggle(all[2]),
                expected: vec![1, 2],
            },
            Case {
                name: "toggle of a member removes it",
                start: vec![1, 2, 3],
                signal: |all| Select::Toggle(all[3]),
                expected: vec![1, 2],
            },
            Case {
                name: "replace with root and another drops the root",
                start: vec![],
                signal: |all| Select::Replace(vec![all[0], all[1]]),
                expected: vec![1],
            },
            Case {
                name: "replace with the root alone keeps it",
                start: vec![],
                signal: |all| Select::only(all[0]),
                expected: vec![0],
            },
            Case {
                name: "toggle onto the root drops the root",
                start: vec![0],
                signal: |all| Select::Toggle(all[1]),
                expected: vec![1],
            },
            Case {
                name: "replace with nothing clears",
                start: vec![1, 2],
                signal: |_| Select::none(),
                expected: vec![],
            },
        ];
        for case in cases {
            world.trigger(Select::Replace(pick(&all, &case.start)));
            world.trigger((case.signal)(&all));
            assert_eq!(
                world.get_resource::<Selection>().unwrap().entities(),
                pick(&all, &case.expected),
                "{}",
                case.name
            );
        }

        world.trigger(Select::Replace(vec![a, b, c]));
        world.despawn(b);
        app.update();
        let world = app.main_mut().world_mut();
        let selection = world.get_resource::<Selection>().unwrap();
        assert_eq!(selection.entities(), [a, c]);
        assert_eq!(selection.active(), Some(c));

        let parent = world.spawn(());
        let child = world.spawn(ChildOf::new(parent));
        let other = world.spawn(());
        world.trigger(Select::Replace(vec![parent, child, other]));
        let mut system = (move |selection: Res<Selection>, parents: Query<&ChildOf>| {
            assert_eq!(selection.roots(&parents), [parent, other]);
        })
        .into_system();
        system.initialize(world);
        system.run_and_apply((), world);
        let selection = world.get_resource::<Selection>().unwrap();
        assert_eq!(selection.entities(), [parent, child, other]);
    }
}

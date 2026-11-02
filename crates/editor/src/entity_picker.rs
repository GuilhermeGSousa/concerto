use std::collections::HashSet;

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res,
    command::EntityCommandQueue,
    component::{name::Name, scene::SceneEntityRef},
    entity::hierarchy::{ChildOf, Children},
    query::filter::Without,
    signal::{
        On,
        listener::{GlobalListener, IntoListener, Listener},
    },
};
use concerto_ui::elements::{Listening, Pressable, Text, prelude::*};
use concerto_ui::{
    interaction::UIClick, node::UIRect, text::UIText, theme::UITheme, transform::UIValue,
};

use crate::asset_editor::ActiveEditor;
use crate::asset_picker::{picker_button, picker_caret, picker_label};
use crate::fonts::{glyph, icon};
use crate::hierarchy::{ListRoot, listed_entities};
use crate::hosted::{EditWorld, PlayState};
use crate::inspector::{EditContext, EditError, PropertyEditor, PropertyRowValue};
use crate::tools::scene::SceneRoot;

const NOT_IN_SCENE: &str = "(not in this scene)";

/// Picks the entity a [`SceneEntityRef`] names from the entities of the scene being edited.
#[derive(Default)]
pub struct EntityPicker;

/// Keeps every entity picker row naming the entity it holds.
pub struct EntityPickerPlugin;

impl Plugin for EntityPickerPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, refresh_entity_picker_labels);
    }
}

/// The text inside an [`EntityPicker`] row's button, naming the row's entity.
#[derive(Component)]
pub(crate) struct EntityPickerLabel {
    row: Entity,
}

pub(crate) fn register_defaults(registry: &mut crate::inspector::InspectorRegistry) {
    registry.register_property_editor::<SceneEntityRef, EntityPicker>();
}

impl PropertyEditor<SceneEntityRef> for EntityPicker {
    type Snapshot = SceneEntityRef;
    type Edit = SceneEntityRef;

    fn snapshot(&self, value: &SceneEntityRef) -> SceneEntityRef {
        *value
    }

    fn build(&self, row: &mut EntityCommandQueue, _: &SceneEntityRef, theme: &UITheme) {
        let entity = row.entity();
        row.add_child_with(
            picker_button(theme).on_click(toggle_entity_picker(entity)),
            |mut button| {
                button
                    .add_child((picker_label(theme), EntityPickerLabel { row: entity }))
                    .add_child(picker_caret(theme));
            },
        )
        .add_child(select_button(theme, entity));
    }

    fn apply(
        &self,
        value: &mut SceneEntityRef,
        edit: &SceneEntityRef,
        _: &EditContext,
    ) -> Result<(), EditError> {
        if edit.entity().is_none() && *edit != SceneEntityRef::default() {
            return Err(EditError::Rejected);
        }
        *value = *edit;
        Ok(())
    }

    fn edit_to(&self, value: &SceneEntityRef) -> Option<SceneEntityRef> {
        Some(*value)
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

fn select_button(theme: &UITheme, row: Entity) -> (Listening<Pressable, UIClick>, Text) {
    let size = theme.font_size_md;
    let vertical = (theme.control_height - theme.line_height(size)) / 2.0;
    let horizontal = (theme.control_height - size) / 2.0;
    (
        theme
            .pressable()
            .size(
                UIValue::Px(theme.control_height),
                UIValue::Px(theme.control_height),
            )
            .padding(UIRect::axes(vertical, horizontal))
            .fixed()
            .on_click(select_referenced(row)),
        icon(theme, glyph::CROSSHAIR, size).muted(),
    )
}

fn toggle_entity_picker(_row: Entity) -> Listener<UIClick> {
    (|_: On<UIClick>| {}).into_listener()
}

fn select_referenced(_row: Entity) -> Listener<UIClick> {
    (|_: On<UIClick>| {}).into_listener()
}

fn entity_label(entity: Entity, name: Option<&Name>) -> String {
    match name {
        Some(name) if !name.as_str().is_empty() => name.as_str().to_owned(),
        _ => format!("Entity {}", entity.index()),
    }
}

/// Writes the name of each picker row's entity into its label.
pub(crate) fn refresh_entity_picker_labels(
    values: Query<&PropertyRowValue>,
    labels: Query<(&EntityPickerLabel, &mut UIText)>,
    names: EditWorld<Query<Option<&Name>>>,
    roots: EditWorld<Query<(Entity, &SceneRoot)>>,
    top_level: EditWorld<Query<Entity, (Without<ChildOf>, Without<GlobalListener>)>>,
    children: EditWorld<Query<&Children>>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
) {
    if labels.iter().next().is_none() {
        return;
    }
    let listed: HashSet<Entity> = listed_entities(
        &roots,
        &top_level,
        &children,
        &active,
        &states,
        ListRoot::Exclude,
    )
    .into_iter()
    .map(|listed| listed.entity)
    .collect();
    for (label, mut text) in labels.iter() {
        let Some(reference) = values
            .get_entity(label.row)
            .and_then(|value| value.downcast::<SceneEntityRef>().copied())
        else {
            continue;
        };
        let wanted = match reference.entity() {
            Some(entity) if listed.contains(&entity) => {
                entity_label(entity, names.get_entity(entity).flatten())
            }
            _ => NOT_IN_SCENE.to_owned(),
        };
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hosted::testing::SceneFixture,
        inspector::{InspectorRegistry, PropertyRow, row_systems, spawn_panel},
        tools::selection::Select,
    };
    use concerto_ecs::{
        IntoSystem, System, World, command::CommandQueue, component::bundle::IntoBundle,
        reflect::ReflectComponent, system::BoxedSystem,
    };
    use concerto_foundation::{
        assets::{asset_server::AssetServer, registry::AssetRegistry},
        transform::Transform,
    };
    use concerto_reflect::{PathSegment, PropertyPath, Reflect, ReflectDefault};
    use concerto_ui::node::UINode;
    use std::any::TypeId;

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct Links {
        target: SceneEntityRef,
        maybe: Option<SceneEntityRef>,
        many: Vec<SceneEntityRef>,
    }

    struct Picking {
        f: SceneFixture,
        document: Entity,
        holder: Entity,
        capsule: Entity,
        unnamed: Entity,
        blank: Entity,
        root: Entity,
        systems: Vec<BoxedSystem>,
    }

    fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
        let parent = cmd.spawn(UINode::default()).entity();
        spawn_panel(&mut cmd, parent, &theme);
    }

    fn spawn_under(scene: &mut World, root: Entity, bundle: impl IntoBundle) -> Entity {
        let entity = scene.spawn(bundle);
        scene.entity_mut(root).add_child(entity);
        entity
    }

    fn picking(links: impl Fn(&Picking) -> Links) -> Picking {
        let mut f = SceneFixture::new();
        f.world.insert_resource(InspectorRegistry::default());
        let document = f.open();
        let root = f.scene_root(document);
        let scene = f.scene_world_mut(document);
        scene.register_type::<Links>();
        let capsule = spawn_under(scene, root, (Transform::IDENTITY, Name::new("capsule")));
        let unnamed = spawn_under(scene, root, Transform::IDENTITY);
        let blank = spawn_under(scene, root, (Transform::IDENTITY, Name::new("")));
        let holder = spawn_under(scene, root, (Transform::IDENTITY, Name::new("holder")));

        let mut panel = build_inspector.into_system();
        panel.initialize(&mut f.world);
        panel.run_and_apply((), &mut f.world);
        let mut systems = row_systems();
        systems.push(refresh_entity_picker_labels.into_boxed_system());
        for system in &mut systems {
            system.initialize(&mut f.world);
        }
        let mut p = Picking {
            f,
            document,
            holder,
            capsule,
            unnamed,
            blank,
            root,
            systems,
        };
        let links = links(&p);
        let scene = p.scene();
        scene.insert(links, holder);
        scene.trigger(Select(Some(holder)));
        p.update();
        p.update();
        p
    }

    impl Picking {
        fn scene(&mut self) -> &mut World {
            self.f.scene_world_mut(self.document)
        }
        fn update(&mut self) {
            for system in &mut self.systems {
                system.run_and_apply((), &mut self.f.world);
            }
            self.f.world.tick();
            self.scene().tick();
        }
        fn set(&mut self, target: SceneEntityRef) {
            let holder = self.holder;
            self.scene()
                .get_component_for_entity_mut::<Links>(holder)
                .unwrap()
                .target = target;
            self.update();
            self.update();
        }
        fn row(&mut self, path: &[PathSegment]) -> Option<(Entity, PropertyRow)> {
            let path = PropertyPath::new(path.iter().copied());
            self.f
                .world
                .query::<(Entity, &PropertyRow), ()>()
                .iter(&mut self.f.world)
                .find(|(_, row)| row.address.path == path)
                .map(|(entity, row)| (entity, row.clone()))
        }
        fn label(&mut self, path: &[PathSegment]) -> String {
            let (row, _) = self.row(path).expect("the inspector built the row");
            self.f
                .world
                .query::<(&EntityPickerLabel, &UIText), ()>()
                .iter(&mut self.f.world)
                .find(|(label, _)| label.row == row)
                .map(|(_, text)| text.text.clone())
                .expect("the row has a picker label")
        }
    }

    const TARGET: PathSegment = PathSegment::Field("target");

    fn unlinked(_: &Picking) -> Links {
        Links::default()
    }

    #[test]
    fn the_label_names_the_referenced_entity() {
        let mut p = picking(|p| Links {
            target: SceneEntityRef::Entity(p.capsule),
            ..Links::default()
        });
        assert_eq!(p.label(&[TARGET]), "capsule");
    }

    #[test]
    fn an_entity_with_no_name_or_an_empty_one_is_labelled_by_its_index() {
        let mut p = picking(unlinked);
        let unnamed = p.unnamed;
        p.set(SceneEntityRef::Entity(unnamed));
        assert_eq!(p.label(&[TARGET]), format!("Entity {}", unnamed.index()));

        let blank = p.blank;
        p.set(SceneEntityRef::Entity(blank));
        assert_eq!(p.label(&[TARGET]), format!("Entity {}", blank.index()));
    }

    #[test]
    fn an_unset_value_reads_not_in_this_scene() {
        let mut p = picking(unlinked);
        assert_eq!(p.label(&[TARGET]), NOT_IN_SCENE);
    }

    #[test]
    fn a_deleted_entity_reads_not_in_this_scene() {
        let mut p = picking(|p| Links {
            target: SceneEntityRef::Entity(p.capsule),
            ..Links::default()
        });
        let capsule = p.capsule;
        assert_eq!(p.label(&[TARGET]), "capsule");
        p.scene().despawn(capsule);
        p.update();
        assert_eq!(p.label(&[TARGET]), NOT_IN_SCENE);
    }

    #[test]
    fn the_scene_root_reads_not_in_this_scene() {
        let mut p = picking(|p| Links {
            target: SceneEntityRef::Entity(p.root),
            ..Links::default()
        });
        assert_eq!(p.label(&[TARGET]), NOT_IN_SCENE);
    }

    fn apply(value: &mut SceneEntityRef, edit: SceneEntityRef) -> Result<(), EditError> {
        EntityPicker.apply(
            value,
            &edit,
            &EditContext {
                assets: &AssetServer::new(),
                registry: &AssetRegistry::default(),
            },
        )
    }

    #[test]
    fn apply_takes_an_entity_or_the_unset_value_and_refuses_any_other_index() {
        let entity = Entity::parse_id("7v2").unwrap();
        let mut value = SceneEntityRef::default();

        let edit = EntityPicker
            .edit_to(&SceneEntityRef::Entity(entity))
            .unwrap();
        apply(&mut value, edit).unwrap();
        assert_eq!(value, SceneEntityRef::Entity(entity));

        assert_eq!(
            apply(&mut value, SceneEntityRef::Index(3)),
            Err(EditError::Rejected)
        );
        assert_eq!(value, SceneEntityRef::Entity(entity));

        apply(&mut value, SceneEntityRef::default()).unwrap();
        assert_eq!(value, SceneEntityRef::default());
    }

    #[test]
    fn option_and_list_rows_wrap_the_picker() {
        let mut p = picking(|p| Links {
            maybe: Some(SceneEntityRef::Entity(p.capsule)),
            many: vec![SceneEntityRef::Entity(p.unnamed), SceneEntityRef::default()],
            ..Links::default()
        });
        let unnamed = p.unnamed;
        let maybe = [PathSegment::Field("maybe"), PathSegment::Field("0")];
        let first = [PathSegment::Field("many"), PathSegment::Index(0)];
        let second = [PathSegment::Field("many"), PathSegment::Index(1)];
        for path in [&maybe[..], &first, &second] {
            let (_, row) = p.row(path).expect("the inspector built the row");
            assert_eq!(row.editor_type, TypeId::of::<EntityPicker>());
        }
        assert_eq!(p.label(&maybe), "capsule");
        assert_eq!(p.label(&first), format!("Entity {}", unnamed.index()));
        assert_eq!(p.label(&second), NOT_IN_SCENE);
    }
}

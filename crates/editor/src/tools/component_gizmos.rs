//! Gizmos that draw a component while its entity, or one of its ancestors, is selected.
use std::any::{TypeId, type_name};

use concerto_app::{App, edit_time::AtEditTime, schedule_groups::LateUpdate};
use concerto_debug_gizmos::{
    ComponentGizmo, DebugGizmos, DebugGizmosPlugin,
    component::{BoundsGizmo, CameraGizmo, LightGizmo, SkeletonGizmo},
};
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, Res, ResMut, Resource, Without,
    entity::hierarchy::ChildOf,
    signal::{On, Signal},
    system::input::StaticSystemInput,
};
use concerto_foundation::transform::{GlobalTransform, systems::propagate_global_transforms};
use concerto_render::plugin::RenderPlugin;

use crate::tools::{camera::EditorHelper, selection::Selection};

/// A registered gizmo kind and whether it is drawn whatever is selected.
#[derive(Clone, Debug)]
pub struct GizmoKind {
    /// The gizmo's `NAME`.
    pub name: &'static str,
    /// Drawn for every entity, not only for the selection.
    pub always: bool,
    gizmo: TypeId,
    type_name: &'static str,
}

/// The gizmo kinds registered in this `App`, each drawn for the selection or always.
#[derive(Resource, Default)]
pub struct ComponentGizmos {
    kinds: Vec<GizmoKind>,
}

impl ComponentGizmos {
    /// The registered kinds, in the order they were registered.
    pub fn kinds(&self) -> &[GizmoKind] {
        &self.kinds
    }

    /// Whether the kind `name` is drawn for every entity.
    pub fn always(&self, name: &str) -> bool {
        self.kinds
            .iter()
            .any(|kind| kind.name == name && kind.always)
    }

    pub(crate) fn add<G: ComponentGizmo>(&mut self) -> bool {
        let gizmo = TypeId::of::<G>();
        if let Some(kind) = self.kinds.iter().find(|kind| kind.name == G::NAME) {
            assert!(
                kind.gizmo == gizmo,
                "the gizmo kind `{}` is registered by both `{}` and `{}`",
                G::NAME,
                kind.type_name,
                type_name::<G>()
            );
            return false;
        }
        self.kinds.push(GizmoKind {
            name: G::NAME,
            always: false,
            gizmo,
            type_name: type_name::<G>(),
        });
        true
    }
}

/// Draws the gizmo kind `kind` for every entity, or only for the selection; an unknown kind changes nothing.
pub struct SetGizmoAlways {
    pub kind: String,
    pub always: bool,
}

impl Signal for SetGizmoAlways {}

pub(crate) fn set_gizmo_always(on: On<SetGizmoAlways>, mut gizmos: ResMut<ComponentGizmos>) {
    let signal = on.signal();
    let Some(index) = gizmos
        .kinds
        .iter()
        .position(|kind| kind.name == signal.kind && kind.always != signal.always)
    else {
        return;
    };
    gizmos.kinds[index].always = signal.always;
}

/// Registers component gizmos in an `App` the editor hosts.
pub trait ComponentGizmosAppExt {
    /// Draws `G` for the selection and its descendants, and lists its kind; panics when another gizmo type has its `NAME`.
    fn register_component_gizmo<G: ComponentGizmo>(&mut self) -> &mut Self;
}

impl ComponentGizmosAppExt for App {
    fn register_component_gizmo<G: ComponentGizmo>(&mut self) -> &mut Self {
        if self.get_resource::<ComponentGizmos>().is_none() {
            self.insert_resource(ComponentGizmos::default());
            self.add_listener(set_gizmo_always.at_edit_time());
        }
        let added = self
            .get_resource_mut::<ComponentGizmos>()
            .expect("ComponentGizmos was just inserted")
            .add::<G>();
        if added && self.is_plugin_added::<DebugGizmosPlugin>() {
            self.add_system(
                LateUpdate,
                draw_component_gizmo::<G>
                    .after(propagate_global_transforms)
                    .at_edit_time(),
            );
        }
        self
    }
}

fn draw_component_gizmo<G: ComponentGizmo>(
    drawn: Query<(Entity, &G::Component, &GlobalTransform), Without<EditorHelper>>,
    parents: Query<&ChildOf>,
    selection: Option<Res<Selection>>,
    kinds: Res<ComponentGizmos>,
    input: StaticSystemInput<G::Input>,
    mut gizmos: DebugGizmos,
) {
    let always = kinds.always(G::NAME);
    let selected = selection.and_then(|selection| selection.entity());
    if !always && selected.is_none() {
        return;
    }
    for (entity, component, transform) in drawn.iter() {
        if always || selected.is_some_and(|selected| within(entity, selected, &parents)) {
            G::draw(entity, component, transform, &input, &mut gizmos);
        }
    }
}

fn within(entity: Entity, ancestor: Entity, parents: &Query<&ChildOf>) -> bool {
    let mut current = Some(entity);
    while let Some(at) = current {
        if at == ancestor {
            return true;
        }
        current = parents.get_entity(at).map(|child_of| child_of.parent());
    }
    false
}

pub(crate) fn build(app: &mut App) {
    if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
        app.register_plugin(DebugGizmosPlugin);
    }
    app.register_component_gizmo::<BoundsGizmo>()
        .register_component_gizmo::<SkeletonGizmo>()
        .register_component_gizmo::<LightGizmo>()
        .register_component_gizmo::<CameraGizmo>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_debug_gizmos::GizmoStorage;
    use concerto_ecs::{
        IntoSystem, System, World, component::Component, entity::hierarchy::Children,
    };
    use concerto_mesh::Aabb;
    use concerto_render::components::{Camera, Light};
    use glam::{Mat4, Vec3};

    use crate::tools::selection::{Select, select};

    fn run<M>(system: impl IntoSystem<(), M>, world: &mut World) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn drawn(world: &mut World) -> usize {
        world.insert_resource(GizmoStorage::default());
        run(draw_component_gizmo::<BoundsGizmo>, world);
        run(draw_component_gizmo::<LightGizmo>, world);
        run(draw_component_gizmo::<CameraGizmo>, world);
        world
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }

    fn draw_every<G: ComponentGizmo>(
        drawn: Query<(Entity, &G::Component, &GlobalTransform)>,
        input: StaticSystemInput<G::Input>,
        mut gizmos: DebugGizmos,
    ) {
        for (entity, component, transform) in drawn.iter() {
            G::draw(entity, component, transform, &input, &mut gizmos);
        }
    }

    fn alone<G: ComponentGizmo>(component: G::Component) -> usize {
        let mut world = World::new();
        world.insert_resource(GizmoStorage::default());
        world.spawn((component, GlobalTransform::new(Mat4::IDENTITY)));
        run(draw_every::<G>, &mut world);
        world
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }

    fn placed(world: &mut World, component: impl Component, at: Vec3) -> Entity {
        world.spawn((component, GlobalTransform::new(Mat4::from_translation(at))))
    }

    fn set(world: &mut World, kind: &str, always: bool) {
        world.trigger(SetGizmoAlways {
            kind: kind.to_string(),
            always,
        });
    }

    #[test]
    fn a_gizmo_draws_for_the_selection_and_its_descendants_or_always_when_set() {
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(Selection::default());
        world.add_listener(select);
        world.add_listener(set_gizmo_always);
        let mut kinds = ComponentGizmos::default();
        kinds.add::<BoundsGizmo>();
        kinds.add::<LightGizmo>();
        kinds.add::<CameraGizmo>();
        world.insert_resource(kinds);

        let aabb = Aabb {
            min: Vec3::splat(-0.5),
            max: Vec3::splat(0.5),
        };
        let light = || Light::point_light().with_range(2.0);
        let parent = placed(&mut world, light(), Vec3::ZERO);
        let child = placed(&mut world, aabb, Vec3::X);
        world.entity_mut(parent).add_child(child);
        placed(&mut world, Camera::default(), Vec3::new(0.0, 0.0, 10.0));
        world.spawn((
            Camera::default(),
            GlobalTransform::new(Mat4::from_translation(Vec3::new(0.0, 5.0, 10.0))),
            EditorHelper,
        ));
        let bounds = alone::<BoundsGizmo>(aabb);
        let lit = alone::<LightGizmo>(light());
        let camera = alone::<CameraGizmo>(Camera::default());

        assert_eq!(drawn(&mut world), 0, "nothing is selected");
        world.trigger(Select(Some(parent)));
        assert_eq!(drawn(&mut world), lit + bounds, "the parent and its child");
        world.trigger(Select(Some(child)));
        assert_eq!(drawn(&mut world), bounds, "the child alone");
        world.trigger(Select(None));
        set(&mut world, "Camera", true);
        assert_eq!(
            drawn(&mut world),
            camera,
            "the scene's camera, not the helper"
        );
        set(&mut world, "Camera", false);
        set(&mut world, "Bounds", true);
        assert_eq!(drawn(&mut world), bounds, "bounds with nothing selected");
    }
}

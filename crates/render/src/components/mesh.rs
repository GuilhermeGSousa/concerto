use bitflags::bitflags;
use concerto_app::extractor::Extracted;
use concerto_ecs::{
    component::removed::RemovedComponents,
    entity::EntityHashSet,
    query::Query,
    resource::{ResMut, Resource},
    Changed, Entity, Or,
};
use concerto_foundation::{
    assets::AssetId,
    transform::{GlobalTransform, GlobalTransformRaw},
};
use concerto_mesh::{mesh::MeshComponent, Aabb, SkeletonComponent};
use derive_more::{Deref, DerefMut};
use glam::Mat4;

use crate::{
    batching::{BatchedInstanceBuffers, InstanceInputUniformBuffer},
    components::{
        render_entity::{MainEntity, MainEntityHashMap},
        visibility::{NoFrustumCulling, NotShadowCaster},
    },
    frustum::Frustum,
};

pub(crate) struct RenderMeshInstance {
    pub(crate) mesh_asset_id: AssetId,
    pub(crate) flags: RenderMeshInstanceFlags,
    pub(crate) current_uniform_index: u32,
    pub(crate) world_aabb: Option<Aabb>,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) struct RenderMeshInstanceFlags: u8 {
        const SKINNED = 1;
        const NO_FRUSTUM_CULLING = 1 << 1;
        const SHADOW_CASTER = 1 << 2;
    }
}

impl RenderMeshInstance {
    pub(crate) fn is_visible_in(&self, frustum: &Frustum) -> bool {
        if self.flags.intersects(
            RenderMeshInstanceFlags::SKINNED | RenderMeshInstanceFlags::NO_FRUSTUM_CULLING,
        ) {
            return true;
        }
        self.world_aabb
            .is_none_or(|world_aabb| frustum.intersects_aabb(&world_aabb))
    }
}

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct RenderMeshInstances(MainEntityHashMap<RenderMeshInstance>);

type MeshExtractionQuery = (
    Entity,
    &'static MeshComponent,
    &'static GlobalTransform,
    Option<&'static SkeletonComponent>,
    Option<&'static Aabb>,
    Option<&'static NoFrustumCulling>,
    Option<&'static NotShadowCaster>,
);

type ChangedMeshFilter = Or<(
    Changed<GlobalTransform>,
    Changed<MeshComponent>,
    Changed<SkeletonComponent>,
    Changed<Aabb>,
    Changed<NoFrustumCulling>,
    Changed<NotShadowCaster>,
)>;

fn extract_mesh(
    (entity, mesh, transform, skeleton, aabb, no_frustum_culling, not_shadow_caster): (
        Entity,
        &MeshComponent,
        &GlobalTransform,
        Option<&SkeletonComponent>,
        Option<&Aabb>,
        Option<&NoFrustumCulling>,
        Option<&NotShadowCaster>,
    ),
    render_mesh_instances: &mut RenderMeshInstances,
    input: &mut InstanceInputUniformBuffer<GlobalTransformRaw>,
) {
    let mut flags = RenderMeshInstanceFlags::empty();
    flags.set(RenderMeshInstanceFlags::SKINNED, skeleton.is_some());
    flags.set(
        RenderMeshInstanceFlags::NO_FRUSTUM_CULLING,
        no_frustum_culling.is_some(),
    );
    flags.set(
        RenderMeshInstanceFlags::SHADOW_CASTER,
        not_shadow_caster.is_none(),
    );
    let world_aabb = aabb.map(|aabb| aabb.transformed(transform.matrix()));
    let raw_transform = if flags.contains(RenderMeshInstanceFlags::SKINNED) {
        GlobalTransform::new(Mat4::IDENTITY).to_raw()
    } else {
        transform.to_raw()
    };

    let main_entity = MainEntity::new(entity);
    if let Some(instance) = render_mesh_instances.get_mut(&main_entity) {
        instance.mesh_asset_id = mesh.handle.id();
        instance.world_aabb = world_aabb;
        instance.flags = flags;
        input.set(instance.current_uniform_index, raw_transform);
        return;
    }

    let current_uniform_index = input.add(raw_transform);
    render_mesh_instances.insert(
        main_entity,
        RenderMeshInstance {
            mesh_asset_id: mesh.handle.id(),
            flags,
            current_uniform_index,
            world_aabb,
        },
    );
}

pub(crate) fn extract_meshes(
    changed_meshes: Extracted<Query<MeshExtractionQuery, ChangedMeshFilter>>,
    all_meshes: Extracted<Query<MeshExtractionQuery>>,
    mut removed_meshes: Extracted<RemovedComponents<MeshComponent>>,
    mut removed_skeletons: Extracted<RemovedComponents<SkeletonComponent>>,
    mut removed_aabbs: Extracted<RemovedComponents<Aabb>>,
    mut removed_no_frustum_culling: Extracted<RemovedComponents<NoFrustumCulling>>,
    mut removed_not_shadow_casters: Extracted<RemovedComponents<NotShadowCaster>>,
    mut render_mesh_instances: ResMut<RenderMeshInstances>,
    mut buffers: ResMut<BatchedInstanceBuffers>,
) {
    let input = &mut buffers.current_input_buffer;

    for row in changed_meshes.iter() {
        extract_mesh(row, &mut render_mesh_instances, input);
    }

    let removed: EntityHashSet = removed_skeletons
        .read()
        .chain(removed_aabbs.read())
        .chain(removed_no_frustum_culling.read())
        .chain(removed_not_shadow_casters.read())
        .collect();
    for entity in removed {
        if changed_meshes.contains_entity(entity) {
            continue;
        }
        if let Some(row) = all_meshes.get_entity(entity) {
            extract_mesh(row, &mut render_mesh_instances, input);
        }
    }

    for entity in removed_meshes.read() {
        if all_meshes.contains_entity(entity) {
            continue;
        }
        if let Some(instance) = render_mesh_instances.remove(&MainEntity::new(entity)) {
            input.remove(instance.current_uniform_index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        components::render_entity::MainEntity, draw_target::DrawTarget, gpu::Gpu,
        plugin::RenderPlugin,
    };
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
        App,
    };
    use concerto_ecs::Entity;
    use concerto_foundation::{assets::handle::AssetHandle, transform::Transform};
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::Vec3;

    fn app(gpu: &Gpu, mesh: AssetId, translations: &[Vec3]) -> (App, Vec<Entity>) {
        let mut app = App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::None)
            .insert_resource(Window::new((64, 48), 1.0));
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(RenderPlugin);
        app.main_mut().world_mut().register_component::<Transform>();
        let entities = translations
            .iter()
            .map(|&translation| spawn_mesh(&mut app, mesh, translation))
            .collect();
        app.finish_plugin_build();
        (app, entities)
    }

    fn spawn_mesh(app: &mut App, mesh: AssetId, translation: Vec3) -> Entity {
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: AssetHandle::weak(mesh),
            },
            Transform::from_translation(translation),
        ))
    }

    fn instances(app: &App) -> &RenderMeshInstances {
        app.render().get_resource::<RenderMeshInstances>().unwrap()
    }

    fn drawn(app: &mut App) -> Vec<AssetId> {
        instances(app)
            .values()
            .map(|instance| instance.mesh_asset_id)
            .collect()
    }

    #[test]
    fn swapping_a_mesh_handle_changes_the_mesh_its_render_entity_draws() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (first, second) = (AssetId::new(), AssetId::new());
        let (mut app, entities) = app(&gpu, first, &[Vec3::ZERO]);
        app.update();
        assert_eq!(drawn(&mut app), [first]);

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<MeshComponent>(entities[0])
            .unwrap()
            .handle = AssetHandle::weak(second);
        app.update();

        assert_eq!(drawn(&mut app), [second]);
    }

    fn slot(app: &mut App, entity: Entity) -> u32 {
        instances(app)
            .get(&MainEntity::new(entity))
            .unwrap()
            .current_uniform_index
    }

    fn stored(app: &App, slot: u32) -> Mat4 {
        let value = app
            .render()
            .get_resource::<BatchedInstanceBuffers>()
            .unwrap()
            .current_input_buffer
            .get(slot);
        Mat4::from_cols_slice(bytemuck::cast_slice(&[value]))
    }

    fn overwrite(app: &mut App, slot: u32, matrix: Mat4) {
        app.render_mut()
            .get_resource_mut::<BatchedInstanceBuffers>()
            .unwrap()
            .current_input_buffer
            .set(slot, GlobalTransform::new(matrix).to_raw());
    }

    #[test]
    fn a_moved_mesh_stores_its_new_transform() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::ZERO]);
        let entity = entities[0];
        app.update();
        app.update();
        let slot = slot(&mut app, entity);

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = Vec3::new(1.0, 2.0, 3.0);
        app.update();

        assert_eq!(
            stored(&app, slot),
            Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn an_unmoved_mesh_in_a_hierarchy_is_not_extracted_again() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::ZERO]);
        let entity = entities[0];
        let world = app.main_mut().world_mut();
        let parent = world.spawn(Transform::from_translation(Vec3::X));
        world.entity_mut(parent).add_child(entity);
        app.update();
        app.update();
        let slot = slot(&mut app, entity);
        let sentinel = Mat4::from_scale(Vec3::splat(7.0));

        overwrite(&mut app, slot, sentinel);
        app.update();
        app.update();

        assert_eq!(stored(&app, slot), sentinel);
    }

    #[test]
    fn a_mesh_that_gains_a_skeleton_stores_the_identity() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::X]);
        let entity = entities[0];
        app.update();
        app.update();
        let slot = slot(&mut app, entity);
        assert_eq!(stored(&app, slot), Mat4::from_translation(Vec3::X));

        app.main_mut()
            .world_mut()
            .insert(SkeletonComponent::default(), entity);
        app.update();

        assert_eq!(stored(&app, slot), Mat4::IDENTITY);
    }

    #[test]
    fn a_mesh_that_loses_its_skeleton_stores_its_world_transform() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::X]);
        let entity = entities[0];
        app.main_mut()
            .world_mut()
            .insert(SkeletonComponent::default(), entity);
        app.update();
        app.update();
        let slot = slot(&mut app, entity);
        assert_eq!(stored(&app, slot), Mat4::IDENTITY);

        app.main_mut()
            .world_mut()
            .remove_component::<SkeletonComponent>(entity);
        app.update();

        assert_eq!(stored(&app, slot), Mat4::from_translation(Vec3::X));
    }

    #[test]
    fn a_mesh_readded_after_moving_without_one_stores_where_it_now_is() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mesh = AssetId::new();
        let (mut app, entities) = app(&gpu, mesh, &[Vec3::ZERO]);
        let entity = entities[0];
        app.update();
        app.update();
        let slot = slot(&mut app, entity);

        let world = app.main_mut().world_mut();
        world.remove_component::<MeshComponent>(entity);
        world
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = Vec3::Y;
        app.update();
        app.main_mut().world_mut().insert(
            MeshComponent {
                handle: AssetHandle::weak(mesh),
            },
            entity,
        );
        app.update();

        assert_eq!(stored(&app, slot), Mat4::from_translation(Vec3::Y));
    }

    #[test]
    fn a_despawned_meshs_slot_is_reused() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mesh = AssetId::new();
        let (mut app, entities) = app(&gpu, mesh, &[Vec3::ZERO]);
        app.update();
        app.update();
        let old_slot = slot(&mut app, entities[0]);

        app.main_mut().world_mut().despawn(entities[0]);
        app.update();
        let replacement = spawn_mesh(&mut app, mesh, Vec3::X);
        app.update();

        assert_eq!(slot(&mut app, replacement), old_slot);
    }

    #[test]
    fn a_mesh_removed_and_readded_in_one_frame_keeps_its_slot() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mesh = AssetId::new();
        let (mut app, entities) = app(&gpu, mesh, &[Vec3::ZERO]);
        let entity = entities[0];
        app.update();
        app.update();
        let old_slot = slot(&mut app, entity);

        let world = app.main_mut().world_mut();
        world.remove_component::<MeshComponent>(entity);
        world.insert(
            MeshComponent {
                handle: AssetHandle::weak(mesh),
            },
            entity,
        );
        app.update();

        assert_eq!(slot(&mut app, entity), old_slot);
    }

    #[test]
    fn a_mesh_that_loses_its_mesh_component_leaves_render_mesh_instances() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mesh = AssetId::new();
        let (mut app, entities) = app(&gpu, mesh, &[Vec3::ZERO]);
        let entity = entities[0];
        app.update();
        app.update();
        let old_slot = slot(&mut app, entity);

        app.main_mut()
            .world_mut()
            .remove_component::<MeshComponent>(entity);
        app.update();

        assert!(instances(&app).get(&MainEntity::new(entity)).is_none());
        let replacement = spawn_mesh(&mut app, mesh, Vec3::X);
        app.update();
        assert_eq!(slot(&mut app, replacement), old_slot);
    }

    fn frustum() -> Frustum {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        Frustum::from_clip_from_world(proj * view)
    }

    fn instance_with(
        flags: RenderMeshInstanceFlags,
        world_aabb: Option<Aabb>,
    ) -> RenderMeshInstance {
        RenderMeshInstance {
            mesh_asset_id: AssetId::new(),
            flags,
            current_uniform_index: 0,
            world_aabb,
        }
    }

    fn box_behind() -> Option<Aabb> {
        Some(Aabb {
            min: Vec3::new(-1.0, -1.0, 9.0),
            max: Vec3::new(1.0, 1.0, 11.0),
        })
    }

    #[test]
    fn a_skinned_instance_is_visible_outside_the_frustum() {
        let instance = instance_with(RenderMeshInstanceFlags::SKINNED, box_behind());

        assert!(instance.is_visible_in(&frustum()));
    }

    #[test]
    fn an_instance_with_no_frustum_culling_is_visible_outside_the_frustum() {
        let instance = instance_with(RenderMeshInstanceFlags::NO_FRUSTUM_CULLING, box_behind());

        assert!(instance.is_visible_in(&frustum()));
    }

    #[test]
    fn an_instance_with_no_bounds_is_visible() {
        let instance = instance_with(RenderMeshInstanceFlags::empty(), None);

        assert!(instance.is_visible_in(&frustum()));
    }

    #[test]
    fn an_instance_outside_the_frustum_is_not_visible() {
        let instance = instance_with(RenderMeshInstanceFlags::empty(), box_behind());

        assert!(!instance.is_visible_in(&frustum()));
    }

    #[test]
    fn a_mesh_gaining_no_frustum_culling_is_extracted_with_the_flag() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::ZERO]);
        let entity = entities[0];
        app.update();
        app.update();
        let flags = |app: &App| instances(app).get(&MainEntity::new(entity)).unwrap().flags;
        assert!(!flags(&app).contains(RenderMeshInstanceFlags::NO_FRUSTUM_CULLING));

        app.main_mut().world_mut().insert(NoFrustumCulling, entity);
        app.update();

        assert!(flags(&app).contains(RenderMeshInstanceFlags::NO_FRUSTUM_CULLING));
    }

    #[test]
    fn a_mesh_losing_not_shadow_caster_becomes_a_shadow_caster_again() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::ZERO]);
        let entity = entities[0];
        app.main_mut().world_mut().insert(NotShadowCaster, entity);
        app.update();
        app.update();
        let flags = |app: &App| instances(app).get(&MainEntity::new(entity)).unwrap().flags;
        assert!(!flags(&app).contains(RenderMeshInstanceFlags::SHADOW_CASTER));

        app.main_mut()
            .world_mut()
            .remove_component::<NotShadowCaster>(entity);
        app.update();

        assert!(flags(&app).contains(RenderMeshInstanceFlags::SHADOW_CASTER));
    }
}

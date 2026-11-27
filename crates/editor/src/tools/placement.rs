//! Placing an asset in the open scene as an entity.
use anyhow::{Context, bail};
use concerto_app::{
    App, PlaceableAssets, PlaceableKind, edit_time::AtEditTime, schedule_groups::Update,
};
use concerto_ecs::{
    Entity, Res, Resource, World,
    command::CommandQueue,
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    events::{Event, event_channel::EventChannel, event_cursor::EventCursor},
    signal::{On, Signal},
    system::input::SystemLocal,
};
use concerto_foundation::{
    assets::{
        AssetId,
        asset_server::AssetServer,
        asset_store::AssetStore,
        handle::{AssetHandle, AssetLoadFailed},
    },
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::{Aabb, Mesh, MeshComponent, Ray};
use concerto_render::components::{camera::Camera, render_entity::SyncWithRenderWorld};
use concerto_scene::{
    link::LinkRoot,
    node_id::{NodeId, SceneNodeId},
    scene::Scene,
};
use glam::{Mat4, Vec2, Vec3};

use crate::import_fold::shown_name;
use crate::tools::{
    camera::{EditorCamera, ViewportPointer},
    entity_ops::new_parent,
    picking::{pick_hit, ray_through},
    scene::{RevealEntity, SceneRoot, SceneStatus, report_edit, write},
    selection::Select,
};

const GROUND_REACH: f32 = 200.0;
const AHEAD: f32 = 10.0;
const NO_SCENE: &str = "stop the game and open a scene to place assets";
const LOAD_FAILED: &str = "the asset failed to load; see the log";
const LOAD_CANCELLED: &str = "the asset's load was cancelled; place it again";

/// Places `asset` in the open scene as a new entity under `parent`, or under the scene root.
///
/// Trigger it; [`AssetPlaced`] says how it ended, once the asset has loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceAsset {
    /// The asset to place, of a kind the recipe registered as placeable.
    pub asset: AssetId,
    /// The new entity's parent, or `None` for the scene root.
    pub parent: Option<Entity>,
    /// Where the new entity goes.
    pub at: PlaceAt,
}

impl Signal for PlaceAsset {}

/// Where a placed entity goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaceAt {
    /// A point in world space.
    Position(Vec3),
    /// The origin of the parent, which is the scene root when none is named.
    Origin,
    /// What is under a point of the viewport, in logical pixels from its top-left corner: a mesh of the scene, else the ground plane, else a point ahead of the camera.
    Pointer(Vec2),
}

/// How one `PlaceAsset` ended: the entity it made, or the status text saying why it made none.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct AssetPlaced {
    /// The placement that was asked for.
    pub request: PlaceAsset,
    /// The new entity, or the status text saying why none was made.
    pub result: Result<Entity, String>,
}

struct Placement {
    request: PlaceAsset,
    kind: PlaceableKind,
    name: String,
    parent: Entity,
    position: Vec3,
}

/// The placements waiting for their asset to load.
#[derive(Resource, Default)]
pub(crate) struct PendingPlacements(Vec<Placement>);

struct FinishPlacements {
    failed: Vec<AssetId>,
}

impl Signal for FinishPlacements {}

pub(crate) fn build(app: &mut App) {
    app.register_event::<AssetPlaced>()
        .insert_resource(PendingPlacements::default())
        .add_listener(place_asset.at_edit_time())
        .add_listener(finish_placements.at_edit_time())
        .add_system(Update, request_finish.at_edit_time());
}

pub(crate) fn place_asset(on: On<PlaceAsset>, world: &mut World) {
    let request = on.signal();
    match resolve(world, request) {
        Ok(placement) if (placement.kind.is_loaded)(world, request.asset) => {
            finish(world, &placement)
        }
        Ok(placement) => {
            (placement.kind.request_load)(world, request.asset);
            match world.get_resource_mut::<PendingPlacements>() {
                Some(pending) => pending.0.push(placement),
                None => {
                    (placement.kind.release)(world, request.asset);
                    report(world, placement.request, Err(LOAD_FAILED.to_owned()));
                }
            }
        }
        Err(error) => report(world, request.clone(), Err(format!("{error:#}"))),
    }
}

fn request_finish(
    pending: Res<PendingPlacements>,
    failures: Option<Res<EventChannel<AssetLoadFailed>>>,
    mut read: SystemLocal<EventCursor<AssetLoadFailed>>,
    mut cmd: CommandQueue,
) {
    let failed: Vec<AssetId> = failures
        .iter()
        .flat_map(|failures| failures.read_from(&mut read))
        .map(|event| event.id)
        .collect();
    if !pending.0.is_empty() {
        cmd.trigger(FinishPlacements { failed });
    }
}

fn finish_placements(on: On<FinishPlacements>, world: &mut World) {
    let Some(pending) = world.get_resource_mut::<PendingPlacements>() else {
        return;
    };
    let mut waiting = Vec::new();
    for placement in std::mem::take(&mut pending.0) {
        let asset = placement.request.asset;
        let result = if (placement.kind.is_loaded)(world, asset) {
            spawn(world, &placement).map_err(|error| format!("{error:#}"))
        } else if world
            .get_resource::<AssetServer>()
            .is_some_and(|server| server.is_loading_or_loaded(asset))
        {
            waiting.push(placement);
            continue;
        } else if on.signal().failed.contains(&asset) {
            Err(LOAD_FAILED.to_owned())
        } else {
            Err(LOAD_CANCELLED.to_owned())
        };
        if result.is_err() {
            (placement.kind.release)(world, asset);
        }
        report(world, placement.request, result);
    }
    if let Some(pending) = world.get_resource_mut::<PendingPlacements>() {
        pending.0.extend(waiting);
    }
}

fn finish(world: &mut World, placement: &Placement) {
    let result = spawn(world, placement).map_err(|error| format!("{error:#}"));
    report(world, placement.request.clone(), result);
}

fn report(world: &mut World, request: PlaceAsset, result: Result<Entity, String>) {
    match &result {
        Ok(entity) => {
            world.trigger(Select(Some(*entity)));
            write(world, RevealEntity(*entity));
            report_edit(world);
        }
        Err(error) => write(world, SceneStatus(error.clone())),
    }
    write(world, AssetPlaced { request, result });
}

fn scene_root(world: &mut World) -> anyhow::Result<Entity> {
    world
        .query::<(Entity, &SceneRoot), ()>()
        .iter(world)
        .map(|(root, _)| root)
        .next()
        .context(NO_SCENE)
}

fn resolve(world: &mut World, request: &PlaceAsset) -> anyhow::Result<Placement> {
    let root = scene_root(world)?;
    let entry = world
        .get_resource::<AssetServer>()
        .and_then(AssetServer::registry)
        .and_then(|registry| registry.get(request.asset).cloned())
        .with_context(|| {
            format!(
                "no asset with id {}; list_assets shows what exists",
                request.asset.simple_hex()
            )
        })?;
    refuse_linking_the_root(world, request.asset)?;
    let placeable = world.get_resource::<PlaceableAssets>();
    let Some(kind) = placeable.and_then(|kinds| kinds.get(&entry.kind).copied()) else {
        let kinds: Vec<&str> = placeable
            .iter()
            .flat_map(|kinds| kinds.iter())
            .map(|kind| kind.kind)
            .collect();
        bail!(
            "{} assets cannot be placed; placeable kinds: {}",
            entry.kind,
            kinds.join(", ")
        );
    };
    let parent = match request.parent {
        Some(parent) => {
            new_parent(world, Some(parent))?;
            parent
        }
        None => root,
    };
    let position = match request.at {
        PlaceAt::Position(position) => position,
        PlaceAt::Origin => world_matrix(world, parent).transform_point3(Vec3::ZERO),
        PlaceAt::Pointer(pointer) => under_pointer(world, pointer)?,
    };
    Ok(Placement {
        request: request.clone(),
        kind,
        name: shown_name(&entry).into_owned(),
        parent,
        position,
    })
}

fn refuse_linking_the_root(world: &World, asset: AssetId) -> anyhow::Result<()> {
    let Some(root) = world.get_resource::<LinkRoot>().map(|root| root.0) else {
        return Ok(());
    };
    let scenes = world.get_resource::<AssetStore<Scene>>();
    let mut seen = Vec::new();
    let mut pending = vec![asset];
    while let Some(id) = pending.pop() {
        if id == root {
            bail!("a scene cannot link itself");
        }
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let linked = scenes
            .and_then(|scenes| scenes.get(&AssetHandle::weak(id)))
            .into_iter()
            .flat_map(|scene| &scene.nodes)
            .filter_map(|node| node.link.as_ref())
            .map(|link| link.scene);
        pending.extend(linked);
    }
    Ok(())
}

fn spawn(world: &mut World, placement: &Placement) -> anyhow::Result<Entity> {
    scene_root(world)?;
    refuse_linking_the_root(world, placement.request.asset)?;
    new_parent(world, Some(placement.parent))?;
    let parent = world_matrix(world, placement.parent);
    let local = if parent.determinant().abs() < 1e-12 {
        placement.position
    } else {
        parent.inverse().transform_point3(placement.position)
    };
    let entity = world.spawn((
        Name::new(placement.name.clone()),
        Transform::from_translation(local),
        SyncWithRenderWorld,
        SceneNodeId(NodeId::fresh()),
    ));
    world.entity_mut(placement.parent).add_child(entity);
    (placement.kind.fill)(world, placement.request.asset, entity);
    Ok(entity)
}

fn world_matrix(world: &World, entity: Entity) -> Mat4 {
    let mut matrix = Mat4::IDENTITY;
    let mut current = Some(entity);
    while let Some(entity) = current {
        if let Some(transform) = world.get_component_for_entity::<Transform>(entity) {
            matrix = transform.compute_matrix() * matrix;
        }
        current = world
            .get_component_for_entity::<ChildOf>(entity)
            .map(ChildOf::parent);
    }
    matrix
}

fn under_pointer(world: &mut World, pointer: Vec2) -> anyhow::Result<Vec3> {
    let size = world
        .get_resource::<ViewportPointer>()
        .map_or(Vec2::ZERO, |viewport| viewport.size);
    let ray = world
        .query::<(&EditorCamera, &Camera, &Transform), ()>()
        .iter(world)
        .next()
        .and_then(|(_, camera, transform)| {
            ray_through(
                pointer,
                size,
                camera.build_projection_matrix(),
                transform.compute_matrix(),
            )
        })
        .context("the viewport has no camera or no picture yet; place by position instead")?;
    let candidates = scene_meshes(world);
    let hit = world.get_resource::<AssetStore<Mesh>>().and_then(|meshes| {
        pick_hit(
            &ray,
            meshes,
            candidates.iter().map(|candidate| {
                (
                    candidate.entity,
                    &candidate.mesh,
                    &candidate.aabb,
                    &candidate.transform,
                )
            }),
        )
    });
    Ok(landing(&ray, hit.map(|hit| hit.distance)))
}

fn scene_meshes(world: &mut World) -> Vec<Candidate> {
    let mut scene = std::collections::HashSet::new();
    let mut stack: Vec<Entity> = world
        .query::<(Entity, &SceneRoot), ()>()
        .iter(world)
        .map(|(root, _)| root)
        .collect();
    while let Some(entity) = stack.pop() {
        if scene.insert(entity)
            && let Some(children) = world.get_component_for_entity::<Children>(entity)
        {
            stack.extend(children.iter().copied());
        }
    }
    world
        .query::<(Entity, &MeshComponent, &Aabb, &GlobalTransform), ()>()
        .iter(world)
        .filter(|(entity, ..)| scene.contains(entity))
        .map(|(entity, mesh, aabb, transform)| Candidate {
            entity,
            mesh: MeshComponent {
                handle: AssetHandle::weak(mesh.handle.id()),
            },
            aabb: *aabb,
            transform: GlobalTransform::new(transform.matrix()),
        })
        .collect()
}

struct Candidate {
    entity: Entity,
    mesh: MeshComponent,
    aabb: Aabb,
    transform: GlobalTransform,
}

fn landing(ray: &Ray, hit: Option<f32>) -> Vec3 {
    if let Some(distance) = hit {
        return ray.at(distance);
    }
    let ground = -ray.origin.y / ray.direction.y;
    if ground.is_finite() && ground > 0.0 && ground <= GROUND_REACH {
        ray.at(ground)
    } else {
        ray.at(AHEAD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        scene::{SceneDirty, SceneEdited},
        selection::Selection,
    };
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    };
    use concerto_ecs::{Component, ResMut, events::event_reader::EventReader};
    use concerto_foundation::assets::{
        Asset, LoadableAsset, PlaceableAsset, content::save_content_asset_into,
        registry::AssetRegistry,
    };
    use concerto_mesh::Vertex;
    use concerto_scene::{
        node_id::{NodeId, SceneNodeId},
        plugin::ScenePlugin,
        scene::{Scene, SceneNode},
    };
    use glam::Quat;
    use std::path::PathBuf;

    use crate::tools::picking::world_to_viewport;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Prop {
        size: u32,
    }
    impl Asset for Prop {
        fn name() -> &'static str {
            "TestProp"
        }
    }
    impl LoadableAsset for Prop {}
    impl PlaceableAsset for Prop {
        fn place(asset: &Self, _handle: AssetHandle<Self>, entity: Entity, cmd: &mut CommandQueue) {
            cmd.insert(Lamp { size: asset.size }, entity);
        }
    }

    #[derive(Component, PartialEq, Debug)]
    struct Lamp {
        size: u32,
    }

    #[derive(Resource)]
    struct Heard<E: Event + Clone + 'static>(Vec<E>);

    fn hear<E: Event + Clone + 'static>(mut events: EventReader<E>, mut heard: ResMut<Heard<E>>) {
        heard.0.extend(events.read().cloned());
    }

    struct Fixture {
        app: App,
        dir: PathBuf,
        root: Entity,
        anchor: Entity,
        scene: AssetId,
        mesh: AssetId,
        prop: AssetId,
        broken: AssetId,
    }

    fn node(name: &str) -> SceneNode {
        SceneNode {
            id: NodeId::from_name_path(name),
            name: name.into(),
            children: vec![],
            components: vec![],
            link: None,
        }
    }

    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!("placement-{}", AssetId::new().simple_hex()));
        let mut registry = AssetRegistry::new();
        let scene = save_content_asset_into(
            &Scene {
                nodes: vec![node("left"), node("right")],
                referenced_assets: vec![],
            },
            &dir,
            "content/props/pair.gasset",
            &mut registry,
        )
        .unwrap();
        let mesh = save_content_asset_into(
            &Mesh {
                vertices: vec![],
                indices: vec![],
            },
            &dir,
            "content/props/crate.gasset",
            &mut registry,
        )
        .unwrap();
        let prop = save_content_asset_into(
            &Prop { size: 3 },
            &dir,
            "content/props/lamp.gasset",
            &mut registry,
        )
        .unwrap();
        let broken = save_content_asset_into(
            &Prop { size: 4 },
            &dir,
            "content/props/broken.gasset",
            &mut registry,
        )
        .unwrap();
        std::fs::remove_file(dir.join("content/props/broken.gasset")).unwrap();

        let mut app = App::for_editing();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(ScenePlugin)
            .register_asset::<Mesh>()
            .register_placeable_asset::<Prop>();
        crate::tools::build_core(&mut app);
        app.insert_resource(ViewportPointer::default())
            .insert_resource(Heard::<AssetPlaced>(vec![]))
            .insert_resource(Heard::<SceneStatus>(vec![]))
            .insert_resource(Heard::<RevealEntity>(vec![]))
            .insert_resource(Heard::<SceneEdited>(vec![]))
            .add_system(Update, hear::<AssetPlaced>.at_edit_time())
            .add_system(Update, hear::<SceneStatus>.at_edit_time())
            .add_system(Update, hear::<RevealEntity>.at_edit_time())
            .add_system(Update, hear::<SceneEdited>.at_edit_time());
        app.get_resource::<AssetServer>()
            .unwrap()
            .publish_project_content(&dir, registry)
            .unwrap();
        app.finish_plugin_build();

        let world = app.main_mut().world_mut();
        world.register_type::<Transform>();
        world.register_type::<SyncWithRenderWorld>();
        let root = world.spawn((Transform::IDENTITY, SceneRoot));
        let anchor = world.spawn((
            Name::new("anchor"),
            Transform::from_translation_rotation(
                Vec3::new(10.0, 0.0, 0.0),
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ),
        ));
        world.entity_mut(root).add_child(anchor);
        Fixture {
            app,
            dir,
            root,
            anchor,
            scene,
            mesh,
            prop,
            broken,
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Fixture {
        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }
        fn heard<E: Event + Clone + 'static>(&self) -> Vec<E> {
            self.app.get_resource::<Heard<E>>().unwrap().0.clone()
        }
        fn settle(&mut self, outcomes: usize) -> AssetPlaced {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while self.heard::<AssetPlaced>().len() < outcomes {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the placement never ended"
                );
                self.app.update();
            }
            self.app.update();
            let heard = self.heard::<AssetPlaced>();
            assert_eq!(heard.len(), outcomes);
            heard.last().unwrap().clone()
        }
        fn place(&mut self, request: PlaceAsset) -> AssetPlaced {
            let outcomes = self.heard::<AssetPlaced>().len() + 1;
            self.world().trigger(request);
            self.settle(outcomes)
        }
        fn name(&mut self, entity: Entity) -> String {
            self.world()
                .get_component_for_entity::<Name>(entity)
                .unwrap()
                .as_str()
                .to_owned()
        }
        fn parent(&mut self, entity: Entity) -> Option<Entity> {
            self.world()
                .get_component_for_entity::<ChildOf>(entity)
                .map(ChildOf::parent)
        }
        fn children(&mut self, entity: Entity) -> Vec<Entity> {
            self.world()
                .get_component_for_entity::<Children>(entity)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }
        fn local(&mut self, entity: Entity) -> Vec3 {
            self.world()
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation
        }
        fn world_position(&mut self, entity: Entity) -> Vec3 {
            self.world()
                .get_component_for_entity::<GlobalTransform>(entity)
                .unwrap()
                .matrix()
                .transform_point3(Vec3::ZERO)
        }
        fn selected(&self) -> Option<Entity> {
            self.app.get_resource::<Selection>().unwrap().entity()
        }
        fn dirty(&self) -> bool {
            self.app.get_resource::<SceneDirty>().unwrap().0
        }
        fn entities(&mut self) -> usize {
            let world = self.world();
            world.query::<Entity, ()>().iter(world).count()
        }
    }

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn placing_a_scene_and_a_game_kind_spawns_a_selected_entity_once_each_has_loaded() {
        let mut f = fixture();
        let (root, anchor, scene, prop) = (f.root, f.anchor, f.scene, f.prop);

        let request = PlaceAsset {
            asset: scene,
            parent: None,
            at: PlaceAt::Position(Vec3::new(1.0, 0.0, 2.0)),
        };
        f.world().trigger(request.clone());
        assert_eq!(f.children(root), [anchor], "nothing spawns before the load");
        assert!(!f.dirty());
        let placed = f.settle(1);
        let pair = placed.result.clone().unwrap();
        assert_eq!(placed.request, request);
        assert_eq!(f.name(pair), "pair");
        assert_eq!(f.children(root), [anchor, pair]);
        assert!(near(f.world_position(pair), Vec3::new(1.0, 0.0, 2.0)));
        assert!(
            f.world()
                .get_component_for_entity::<SyncWithRenderWorld>(pair)
                .is_some()
        );
        let mut nodes: Vec<String> = f
            .children(pair)
            .into_iter()
            .map(|child| f.name(child))
            .collect();
        nodes.sort();
        assert_eq!(nodes, ["left", "right"]);
        assert_eq!(f.selected(), Some(pair));
        assert!(f.dirty());
        assert_eq!(f.heard::<SceneEdited>().len(), 1);
        assert_eq!(
            f.heard::<RevealEntity>()
                .iter()
                .map(|reveal| reveal.0)
                .collect::<Vec<_>>(),
            [pair]
        );

        let at = Vec3::new(12.0, 1.0, -3.0);
        let lamp = f
            .place(PlaceAsset {
                asset: prop,
                parent: Some(anchor),
                at: PlaceAt::Position(at),
            })
            .result
            .unwrap();
        assert_eq!(f.name(lamp), "lamp");
        assert_eq!(f.parent(lamp), Some(anchor));
        assert!(near(f.local(lamp), Vec3::new(3.0, 1.0, 2.0)));
        assert!(near(f.world_position(lamp), at));
        assert_eq!(
            f.world().get_component_for_entity::<Lamp>(lamp),
            Some(&Lamp { size: 3 })
        );
        assert_eq!(f.selected(), Some(lamp));

        let hook = f
            .place(PlaceAsset {
                asset: prop,
                parent: Some(anchor),
                at: PlaceAt::Origin,
            })
            .result
            .unwrap();
        assert_eq!(f.parent(hook), Some(anchor));
        assert!(near(f.local(hook), Vec3::ZERO));
        assert!(near(f.world_position(hook), Vec3::new(10.0, 0.0, 0.0)));
        let ids = [lamp, hook].map(|entity| {
            f.world()
                .get_component_for_entity::<SceneNodeId>(entity)
                .copied()
        });
        assert!(ids[0].is_some() && ids[0] != ids[1]);
        assert_eq!(f.heard::<SceneEdited>().len(), 3);
        assert!(f.heard::<SceneStatus>().is_empty());
    }

    const VIEW: Vec2 = Vec2::new(800.0, 600.0);

    enum Lands {
        OnTheMesh,
        OnTheGround,
        Ahead,
    }

    struct PointerCase {
        what: &'static str,
        pointer: Vec2,
        lands: Lands,
    }

    #[test]
    fn a_pointer_lands_on_a_mesh_else_on_the_ground_else_ahead_of_the_camera() {
        let mut f = fixture();
        let (root, prop) = (f.root, f.prop);
        let eye = Vec3::new(0.0, 2.0, 6.0);
        let camera = Camera {
            aspect: VIEW.x / VIEW.y,
            ..Default::default()
        };
        let view_proj = camera.build_projection_matrix()
            * Transform::from_translation(eye).compute_matrix().inverse();
        let wall = AssetId::new();
        f.app
            .get_resource_mut::<AssetStore<Mesh>>()
            .unwrap()
            .insert(
                wall,
                Mesh {
                    vertices: [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
                        .into_iter()
                        .map(|[x, y]| Vertex {
                            pos_coords: [x, y, 1.0],
                            ..Default::default()
                        })
                        .collect(),
                    indices: vec![0, 1, 2, 0, 2, 3],
                },
            );
        f.app.insert_resource(ViewportPointer {
            size: VIEW,
            ..Default::default()
        });
        let world = f.world();
        world.spawn((EditorCamera, camera, Transform::from_translation(eye)));
        let raised = Vec3::new(0.0, 2.0, 0.0);
        let quad = world.spawn((
            MeshComponent {
                handle: AssetHandle::weak(wall),
            },
            Aabb {
                min: Vec3::splat(-1.0),
                max: Vec3::splat(1.0),
            },
            Transform::from_translation(raised),
            GlobalTransform::new(Mat4::from_translation(raised)),
        ));
        world.entity_mut(root).add_child(quad);

        for case in [
            PointerCase {
                what: "over a mesh",
                pointer: Vec2::new(400.0, 300.0),
                lands: Lands::OnTheMesh,
            },
            PointerCase {
                what: "over empty ground",
                pointer: Vec2::new(400.0, 500.0),
                lands: Lands::OnTheGround,
            },
            PointerCase {
                what: "over ground beyond reach",
                pointer: Vec2::new(700.0, 301.0),
                lands: Lands::Ahead,
            },
        ] {
            let placed = f
                .place(PlaceAsset {
                    asset: prop,
                    parent: None,
                    at: PlaceAt::Pointer(case.pointer),
                })
                .result
                .unwrap_or_else(|error| panic!("{}: {error}", case.what));
            let position = f.local(placed);
            let seen = world_to_viewport(position, VIEW, view_proj).unwrap();
            assert!(
                (seen - case.pointer).length() < 0.5,
                "{}: {position:?} shows at {seen:?}",
                case.what
            );
            match case.lands {
                Lands::OnTheMesh => {
                    assert!(
                        (position.z - 1.0).abs() < 1e-3,
                        "{}: {position:?}",
                        case.what
                    )
                }
                Lands::OnTheGround => {
                    assert!(position.y.abs() < 1e-3, "{}: {position:?}", case.what)
                }
                Lands::Ahead => assert!(
                    ((position - eye).length() - AHEAD).abs() < 0.5,
                    "{}: {position:?}",
                    case.what
                ),
            }
        }
    }

    struct Refusal {
        what: &'static str,
        request: PlaceAsset,
        then: fn(&mut Fixture),
    }

    fn wait(_: &mut Fixture) {}

    fn close_the_scene(f: &mut Fixture) {
        let root = f.root;
        f.world().remove_component::<SceneRoot>(root);
    }

    #[test]
    fn a_refused_placement_spawns_nothing() {
        let mut f = fixture();
        let before = f.entities();
        let at = PlaceAt::Position(Vec3::ZERO);
        let cases = [
            Refusal {
                what: "an unknown asset",
                request: PlaceAsset {
                    asset: AssetId::new(),
                    parent: None,
                    at,
                },
                then: wait,
            },
            Refusal {
                what: "a kind that is not placeable",
                request: PlaceAsset {
                    asset: f.mesh,
                    parent: None,
                    at,
                },
                then: wait,
            },
            Refusal {
                what: "an asset whose load fails",
                request: PlaceAsset {
                    asset: f.broken,
                    parent: None,
                    at,
                },
                then: wait,
            },
            Refusal {
                what: "a scene that closed while the asset loaded",
                request: PlaceAsset {
                    asset: f.prop,
                    parent: None,
                    at,
                },
                then: close_the_scene,
            },
        ];
        for (index, case) in cases.into_iter().enumerate() {
            f.world().trigger(case.request);
            (case.then)(&mut f);
            assert!(f.settle(index + 1).result.is_err(), "{}", case.what);
        }
        assert_eq!(f.entities(), before);
        assert!(!f.dirty());
        assert_eq!(f.selected(), None);
        assert!(f.heard::<SceneEdited>().is_empty());
    }
}

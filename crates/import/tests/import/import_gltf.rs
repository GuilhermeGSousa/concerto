//! `import` turns a glTF into content assets whose cross-references address
//! content-tree paths, not `<source>#<sub>`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use concerto_animation::clip::AnimationClip;
use concerto_ecs::World;
use concerto_foundation::assets::asset_server::{handle_asset_load_events, AssetServer};
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_foundation::assets::content::{read_content_asset_header, save_content_asset_into};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::{Asset, AssetId, ContentAssetRoot, LoadableAsset};
use concerto_import::metadata::{sidecar_path, SourceMetadata};
use concerto_mesh::mesh::Mesh;
use concerto_mesh::skeleton::Skeleton;
use concerto_render::assets::material::StandardMaterial;
use concerto_render::assets::texture::Texture;
use concerto_scene::scene::Scene;

fn fixture(project_root: &Path) -> PathBuf {
    let destination = project_root.join("assets/triangle.gltf");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../gltf/tests/fixtures/triangle.gltf")
        .canonicalize()
        .expect("fixture exists");
    std::fs::copy(source, &destination).unwrap();
    destination
}

#[test]
fn an_imports_returned_entries_bring_a_scanned_registry_up_to_date() {
    let project_root =
        std::env::temp_dir().join(format!("import-gltf-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    save_content_asset_into(
        &Scene {
            nodes: Vec::new(),
            referenced_assets: Vec::new(),
        },
        &project_root,
        "content/authored.gasset",
        &mut AssetRegistry::new(),
    )
    .unwrap();
    let mut registry =
        AssetRegistry::from_content_tree(&project_root, "content", "gasset").expect("scan");

    let written =
        concerto_import::import_source(&fixture(&project_root), &project_root, &Default::default())
            .expect("import succeeds");
    for entry in &written {
        registry.insert(entry.clone());
    }

    assert!(!written.is_empty());
    let rescanned =
        AssetRegistry::from_content_tree(&project_root, "content", "gasset").expect("rescan");
    assert_eq!(sorted(&registry), sorted(&rescanned));

    std::fs::remove_dir_all(&project_root).ok();
}

fn sorted(registry: &AssetRegistry) -> Vec<AssetEntry> {
    let mut entries: Vec<_> = registry.iter().cloned().collect();
    entries.sort_by_key(|entry| entry.location());
    entries
}

const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

pub(crate) fn write_hero(source: &Path, meshes: usize) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gltf/tests/fixtures/skinned.gltf");
    let mut gltf: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
    gltf["images"] = serde_json::json!([{ "uri": PIXEL }]);
    gltf["textures"] = serde_json::json!([{ "source": 0 }]);
    gltf["materials"] =
        serde_json::json!([{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }]);
    gltf["meshes"][0]["primitives"][0]["material"] = 0.into();
    if meshes == 2 {
        let primitive = serde_json::json!({
            "attributes": { "POSITION": 0 },
            "indices": 1,
            "material": 0
        });
        gltf["meshes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "primitives": [primitive] }));
        let node = gltf["nodes"].as_array().unwrap().len();
        gltf["nodes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "name": "Second", "mesh": 1 }));
        gltf["scenes"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node.into());
    }
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(source, serde_json::to_vec(&gltf).unwrap()).unwrap();
}

pub(crate) fn content_files(project_root: &Path) -> Vec<String> {
    let mut pending = vec![project_root.join("content")];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(
                    path.strip_prefix(project_root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    files.sort();
    files
}

fn locations(entries: &[AssetEntry]) -> Vec<String> {
    let mut locations: Vec<String> = entries.iter().map(|entry| entry.location()).collect();
    locations.sort();
    locations
}

fn ids_by_name(entries: &[AssetEntry]) -> BTreeMap<String, AssetId> {
    entries
        .iter()
        .map(|entry| {
            let name = entry.provenance.as_ref().unwrap().sub_asset.clone();
            (name, entry.id)
        })
        .collect::<BTreeMap<_, _>>()
}

fn embedded_names(project_root: &Path, address: &str) -> Vec<String> {
    read_content_asset_header(&project_root.join(address))
        .unwrap()
        .embedded
        .iter()
        .map(|asset| asset.name().to_string())
        .collect()
}

fn set_file(source: &Path, output: &str, file: bool) {
    let sidecar = sidecar_path(source);
    let mut metadata = SourceMetadata::load(&sidecar).unwrap();
    metadata.outputs.get_mut(output).unwrap().file = file;
    std::fs::write(&sidecar, toml::to_string_pretty(&metadata).unwrap()).unwrap();
}

pub(crate) struct Runtime {
    server: AssetServer,
    world: World,
}

impl Runtime {
    pub(crate) fn new(project_root: &Path) -> Self {
        let server =
            AssetServer::with_content_root(ContentAssetRoot::Directory(project_root.to_path_buf()));
        server.publish_registry(
            AssetRegistry::from_content_tree(project_root, "content", "gasset").expect("scan"),
        );
        let mut runtime = Self {
            world: World::new(),
            server,
        };
        runtime.register::<Scene>();
        runtime.register::<Mesh>();
        runtime.register::<StandardMaterial>();
        runtime.register::<Skeleton>();
        runtime.register::<Texture>();
        runtime.register::<AnimationClip>();
        runtime.world.insert_resource(runtime.server.clone());
        runtime
    }

    fn register<A: Asset>(&mut self) {
        let store = AssetStore::<A>::new();
        self.server.register_asset(&store);
        self.world.insert_resource(store);
    }

    fn loads_as<A: LoadableAsset + 'static>(&mut self, id: AssetId) -> bool {
        let handle = self.server.load::<A>(id);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            handle_asset_load_events(&mut self.world);
            let store = self.world.get_resource::<AssetStore<A>>().unwrap();
            if store.get(&handle).is_some() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        false
    }

    pub(crate) fn loads(&mut self, entry: &AssetEntry) -> bool {
        match entry.kind.as_str() {
            "Scene" => self.loads_as::<Scene>(entry.id),
            "Mesh" => self.loads_as::<Mesh>(entry.id),
            "StandardMaterial" => self.loads_as::<StandardMaterial>(entry.id),
            "Skeleton" => self.loads_as::<Skeleton>(entry.id),
            "Texture" => self.loads_as::<Texture>(entry.id),
            "AnimationClip" => self.loads_as::<AnimationClip>(entry.id),
            kind => panic!("the fixture emits no {kind}"),
        }
    }
}

#[test]
fn a_gltf_import_is_one_scene_file_with_its_textures_and_clips_beside_it() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let source = root.join("assets/hero.gltf");
    write_hero(&source, 2);
    let import = || {
        concerto_import::import_source_into(
            &source,
            "assets/hero.gltf",
            root,
            &Default::default(),
            Default::default(),
        )
        .expect("import succeeds")
    };
    let scene = "content/hero/scene.gasset";
    let packed = [
        "content/hero/animation_Wiggle.gasset",
        scene,
        "content/hero/texture_0.gasset",
    ];

    let first = import().assets;
    assert_eq!(content_files(root), packed);
    assert_eq!(
        locations(&first),
        [
            "content/hero/animation_Wiggle.gasset",
            scene,
            "content/hero/scene.gasset#material/0",
            "content/hero/scene.gasset#mesh/0",
            "content/hero/scene.gasset#mesh/1",
            "content/hero/scene.gasset#skeleton/0",
            "content/hero/texture_0.gasset",
        ]
    );
    let mut runtime = Runtime::new(root);
    for entry in &first {
        assert!(runtime.loads(entry), "{} loads", entry.location());
    }
    let ids = ids_by_name(&first);
    assert!(read_content_asset_header(&root.join(scene))
        .unwrap()
        .references
        .contains(&ids["mesh/0"]));

    set_file(&source, "material/0", true);
    let loose = import().assets;
    let material = loose.iter().find(|entry| entry.kind == "StandardMaterial");
    let material = material.unwrap();
    assert_eq!(material.location(), "content/hero/material_0.gasset");
    assert_eq!(ids_by_name(&loose), ids);
    assert!(root.join(&material.address).is_file());
    assert!(!embedded_names(root, scene).contains(&"material/0".to_string()));
    assert!(Runtime::new(root).loads(material));

    set_file(&source, "material/0", false);
    let again = import().assets;
    assert_eq!(again, first, "removing the key embeds the material again");
    assert_eq!(content_files(root), packed);

    write_hero(&source, 1);
    let pruned = import();
    assert_eq!(pruned.removed, [ids["mesh/1"]]);
    assert_eq!(
        embedded_names(root, scene),
        ["material/0", "mesh/0", "skeleton/0"]
    );
    let mut kept = ids.clone();
    kept.remove("mesh/1");
    assert_eq!(ids_by_name(&pruned.assets), kept);
    assert_eq!(content_files(root), packed);
}

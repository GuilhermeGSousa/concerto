#![allow(missing_docs, reason = "benchmark harness, not public API")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use serde::{Deserialize, Serialize};

use concerto_ecs::World;
use concerto_foundation::assets::{
    asset_server::{handle_asset_load_events, AssetServer},
    asset_store::AssetStore,
    content::{ContentAssetHeader, EmbeddedPayload, ImportProvenance},
    handle::AssetHandle,
    registry::AssetRegistry,
    Asset, AssetId, ContentAssetRoot, LoadableAsset,
};

const EMBEDDED: usize = 1_000;
const HOST_PAYLOAD_BYTES: usize = 8 << 20;

#[derive(Asset, Serialize, Deserialize)]
struct Host {
    filler: Vec<u8>,
}

impl LoadableAsset for Host {}

#[derive(Asset, Serialize, Deserialize)]
struct Part {
    vertices: Vec<f32>,
}

impl LoadableAsset for Part {}

fn header<A: Asset>(name: &str) -> ContentAssetHeader {
    ContentAssetHeader {
        asset_id: AssetId::new(),
        references: Vec::new(),
        kind: A::name().to_string(),
        kind_version: A::FORMAT_VERSION,
        provenance: Some(ImportProvenance {
            source: "assets/host.glb".into(),
            sub_asset: name.into(),
        }),
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
    }
}

struct Project {
    dir: PathBuf,
    registry: AssetRegistry,
    parts: Vec<AssetId>,
}

impl Project {
    fn write() -> Self {
        let dir =
            std::env::temp_dir().join(format!("concerto-bench-loading-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("content/host")).unwrap();
        let host = bincode::serialize(&Host {
            filler: vec![7; HOST_PAYLOAD_BYTES],
        })
        .unwrap();
        let part = bincode::serialize(&Part {
            vertices: vec![1.0; 64],
        })
        .unwrap();
        let embedded: Vec<EmbeddedPayload> = (0..EMBEDDED)
            .map(|index| EmbeddedPayload {
                header: header::<Part>(&format!("mesh/{index}")),
                payload: &part,
                uncooked: None,
                derived_blob: None,
            })
            .collect();
        let parts = embedded.iter().map(|part| part.header.asset_id).collect();
        let file = (header::<Host>("scene"))
            .write_with_embedded(&host, &embedded)
            .expect("the host file is written");
        std::fs::write(dir.join("content/host/scene.gasset"), file).unwrap();
        let registry = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
        Project {
            dir,
            registry,
            parts,
        }
    }
}

fn load_every_part(dir: &Path, registry: &AssetRegistry, parts: &[AssetId]) -> World {
    let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.to_owned()));
    let store = AssetStore::<Part>::new();
    server.register_asset(&store);
    server.publish_registry(registry.clone());
    let mut world = World::new();
    world.insert_resource(store);
    world.insert_resource(server.clone());
    let handles: Vec<AssetHandle<Part>> = parts.iter().map(|id| server.load(*id)).collect();
    while server.is_loading() {
        handle_asset_load_events(&mut world);
        std::thread::yield_now();
    }
    let store = world.get_resource::<AssetStore<Part>>().unwrap();
    assert!(handles.iter().all(|handle| store.get(handle).is_some()));
    world
}

fn embedded_burst(c: &mut Criterion) {
    let project = Project::write();
    let mut group = c.benchmark_group("asset_loading");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("every mesh of a host with 1,000 embedded", |b| {
        b.iter_with_large_drop(|| load_every_part(&project.dir, &project.registry, &project.parts))
    });
    group.finish();
    let _ = std::fs::remove_dir_all(&project.dir);
}

criterion_group!(benches, embedded_burst);
criterion_main!(benches);

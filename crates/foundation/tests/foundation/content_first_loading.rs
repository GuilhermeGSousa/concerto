//! load_content_asset reads a content asset at <root>/<address>.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use concerto_ecs::World;
use concerto_foundation::assets::asset_server::{handle_asset_load_events, AssetServer};
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_foundation::assets::content::{
    write_content_asset, write_content_asset_with_embedded, ContentAssetHeader, EmbeddedPayload,
    ImportProvenance,
};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_foundation::assets::utils::load_content_asset;
use concerto_foundation::assets::{Asset, AssetId, ContentAssetRoot, LoadableAsset};

#[derive(Asset, serde::Serialize, serde::Deserialize, PartialEq, Clone, Debug)]
struct Pebble {
    weight: u32,
}

#[derive(Asset, serde::Serialize, serde::Deserialize, PartialEq, Clone, Debug)]
struct Boulder {
    weight: u32,
}

#[derive(Asset, serde::Serialize, serde::Deserialize, PartialEq, Clone, Debug)]
struct Cairn {
    stones: u32,
}

impl LoadableAsset for Pebble {}

impl LoadableAsset for Boulder {}

impl LoadableAsset for Cairn {}

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("content-first-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_pebble(dir: &std::path::Path, address: &str) {
    let header = ContentAssetHeader {
        asset_id: AssetId::from_path(address),
        references: Vec::new(),
        kind: Pebble::name().to_string(),
        kind_version: Pebble::FORMAT_VERSION,
        provenance: None,
        derived: None,
        embedded: Vec::new(),
    };
    let payload = bincode::serialize(&Pebble { weight: 7 }).unwrap();
    let path = dir.join(address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, &payload).unwrap()).unwrap();
}

struct Load {
    name: &'static str,
    address: &'static str,
    write: fn(&std::path::Path),
    load: fn(&ContentAssetRoot, &str) -> Result<String, String>,
    expected: Result<String, Vec<&'static str>>,
}

fn load<A: Asset + std::fmt::Debug>(
    root: &ContentAssetRoot,
    address: &str,
) -> Result<String, String> {
    pollster::block_on(load_content_asset::<A>(root, address))
        .map(|asset| format!("{asset:?}"))
        .map_err(|err| format!("{err:#}"))
}

#[test]
fn a_content_asset_loads_from_its_literal_address_and_a_failure_names_the_address() {
    let cases = [
        Load {
            name: "a content asset of the requested kind",
            address: "content/x/pebble_0.gasset",
            write: |dir| write_pebble(dir, "content/x/pebble_0.gasset"),
            load: load::<Pebble>,
            expected: Ok(format!("{:?}", Pebble { weight: 7 })),
        },
        Load {
            name: "another kind",
            address: "content/x/thing.gasset",
            write: |dir| write_pebble(dir, "content/x/thing.gasset"),
            load: load::<Boulder>,
            expected: Err(vec!["Pebble", "Boulder", "content/x/thing.gasset"]),
        },
        Load {
            name: "a missing file",
            address: "content/x/absent.gasset",
            write: |_| {},
            load: load::<Pebble>,
            expected: Err(vec!["content/x/absent.gasset"]),
        },
        Load {
            name: "a directory at the address",
            address: "content/x",
            write: |dir| std::fs::create_dir_all(dir.join("content/x")).unwrap(),
            load: load::<Pebble>,
            expected: Err(vec!["content/x"]),
        },
    ];
    for case in cases {
        let dir = temp_root("load");
        (case.write)(&dir);
        let loaded = (case.load)(&ContentAssetRoot::Directory(dir.clone()), case.address);
        std::fs::remove_dir_all(&dir).ok();
        let outcome = loaded;
        match case.expected {
            Ok(expected) => assert_eq!(outcome, Ok(expected), "{}", case.name),
            Err(fragments) => {
                let message = outcome.expect_err(case.name);
                for fragment in fragments {
                    assert!(message.contains(fragment), "{}: {message}", case.name);
                }
            }
        }
    }
}

static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if record.level() == log::Level::Error {
            ERRORS.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

fn capture_errors() {
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Error);
}

struct Runtime {
    server: AssetServer,
    world: World,
}

impl Runtime {
    fn new(dir: &std::path::Path, registry: AssetRegistry) -> Self {
        let mut server =
            AssetServer::with_content_root(ContentAssetRoot::Directory(dir.to_owned()));
        let pebbles = AssetStore::<Pebble>::new();
        let boulders = AssetStore::<Boulder>::new();
        let cairns = AssetStore::<Cairn>::new();
        server.register_asset(&pebbles);
        server.register_asset(&boulders);
        server.register_asset(&cairns);
        server.publish_registry(registry);
        let mut world = World::new();
        world.insert_resource(pebbles);
        world.insert_resource(boulders);
        world.insert_resource(cairns);
        world.insert_resource(server.clone());
        Self { server, world }
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            handle_asset_load_events(&mut self.world);
            if !self.server.is_loading() {
                return;
            }
            assert!(Instant::now() < deadline, "the loads never settled");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn value<A: Asset + Clone>(&self, handle: &AssetHandle<A>) -> Option<A> {
        self.world
            .get_resource::<AssetStore<A>>()
            .unwrap()
            .get(handle)
            .cloned()
    }
}

fn embedded_header<A: Asset>(id: AssetId, name: &str) -> ContentAssetHeader {
    ContentAssetHeader {
        asset_id: id,
        references: Vec::new(),
        kind: A::name().to_string(),
        kind_version: A::FORMAT_VERSION,
        provenance: Some(ImportProvenance {
            source: "assets/cairn.glb".into(),
            sub_asset: name.into(),
        }),
        derived: None,
        embedded: Vec::new(),
    }
}

#[test]
fn embedded_assets_load_by_id() {
    capture_errors();
    let dir = temp_root("embedded");
    let address = "content/cairn/scene.gasset";
    let [cairn, pebble, boulder] = [(); 3].map(|_| AssetId::new());
    let host = write_content_asset_with_embedded(
        &embedded_header::<Cairn>(cairn, "scene"),
        &bincode::serialize(&Cairn { stones: 2 }).unwrap(),
        &[
            EmbeddedPayload {
                header: embedded_header::<Pebble>(pebble, "pebble/0"),
                payload: &bincode::serialize(&Pebble { weight: 3 }).unwrap(),
                derived_blob: None,
            },
            EmbeddedPayload {
                header: embedded_header::<Boulder>(boulder, "boulder/0"),
                payload: &bincode::serialize(&Boulder { weight: 900 }).unwrap(),
                derived_blob: None,
            },
        ],
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("content/cairn")).unwrap();
    std::fs::write(dir.join(address), &host).unwrap();
    let registry = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();

    let mut runtime = Runtime::new(&dir, registry.clone());
    let cancelled_pebble = runtime.server.load::<Pebble>(pebble);
    let cancelled_boulder = runtime.server.load::<Boulder>(boulder);
    runtime.server.cancel_pending();
    drop(cancelled_pebble);
    drop(cancelled_boulder);
    let pebble_handle = runtime.server.load::<Pebble>(pebble);
    let boulder_handle = runtime.server.load::<Boulder>(boulder);
    let cairn_handle = runtime.server.load::<Cairn>(cairn);
    runtime.settle();

    assert_eq!(runtime.value(&cairn_handle), Some(Cairn { stones: 2 }));
    assert_eq!(runtime.value(&pebble_handle), Some(Pebble { weight: 3 }));
    assert_eq!(
        runtime.value(&boulder_handle),
        Some(Boulder { weight: 900 }),
        "a cancelled burst loads when it is requested again"
    );

    std::fs::write(dir.join(address), &host[..host.len() - 1]).unwrap();
    let mut runtime = Runtime::new(&dir, registry);
    let truncated = runtime.server.load::<Pebble>(pebble);
    runtime.settle();
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(runtime.value(&truncated), None);
    let location = "content/cairn/scene.gasset#pebble/0";
    let errors = ERRORS.lock().unwrap();
    assert!(
        errors.iter().any(|error| error.contains(location)),
        "no logged error names {location}: {errors:?}"
    );
}

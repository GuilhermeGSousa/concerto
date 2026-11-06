use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use concerto_ecs::World;
use concerto_foundation::assets::{
    asset_server::{handle_asset_load_events, AssetServer},
    asset_store::AssetStore,
    content::{
        read_content_asset, save_content_asset_into, write_cooked_content_asset, CookedSection,
    },
    derived::{DerivedData, DerivedDataCache, DerivedInputs},
    handle::AssetHandle,
    registry::AssetRegistry,
    Asset, AssetId, ContentAssetRoot, LoadableAsset,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Leaf {
    value: u64,
}

impl Asset for Leaf {
    fn name() -> &'static str {
        "Leaf"
    }
}

impl LoadableAsset for Leaf {}

#[derive(Serialize, Deserialize)]
struct Sum<const V: u32> {
    tag: String,
    a: AssetId,
    b: AssetId,
    #[serde(skip)]
    total: Option<u64>,
}

impl<const V: u32> Asset for Sum<V> {
    fn name() -> &'static str {
        "Sum"
    }
}

impl<const V: u32> LoadableAsset for Sum<V> {}

#[derive(Default)]
struct Probe {
    derives: usize,
    fail_derive: bool,
    gate: Option<mpsc::Receiver<()>>,
}

static PROBES: Mutex<Option<HashMap<String, Probe>>> = Mutex::new(None);

fn with_probe<R>(tag: &str, f: impl FnOnce(&mut Probe) -> R) -> R {
    let mut probes = PROBES.lock().unwrap();
    f(probes
        .get_or_insert_with(HashMap::new)
        .entry(tag.to_string())
        .or_default())
}

fn derives(tag: &str) -> usize {
    with_probe(tag, |probe| probe.derives)
}

impl<const V: u32> DerivedData for Sum<V> {
    const DERIVED_VERSION: u32 = V;

    fn derived_inputs(&self) -> Vec<AssetId> {
        vec![self.a, self.b]
    }

    fn derived_key(&self, inputs: &DerivedInputs) -> anyhow::Result<u64> {
        let mut hasher = DefaultHasher::new();
        for id in self.derived_inputs() {
            inputs.bytes(id).unwrap().hash(&mut hasher);
        }
        Ok(hasher.finish())
    }

    fn derive(
        &self,
        inputs: &DerivedInputs,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>> {
        let gate = with_probe(&self.tag, |probe| {
            probe.derives += 1;
            anyhow::ensure!(!probe.fail_derive, "the sum would overflow");
            Ok(probe.gate.take())
        })?;
        if let Some(gate) = gate {
            gate.recv().unwrap();
        }
        let a = inputs.decode::<Leaf>(self.a)?.value;
        progress(1, 2);
        let b = inputs.decode::<Leaf>(self.b)?.value;
        progress(2, 2);
        Ok((a + b).to_le_bytes().to_vec())
    }

    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
        let bytes: [u8; 8] = blob
            .try_into()
            .map_err(|_| anyhow::anyhow!("a sum blob is 8 bytes, not {}", blob.len()))?;
        self.total = Some(u64::from_le_bytes(bytes));
        Ok(())
    }
}

struct Project {
    root: PathBuf,
    registry: AssetRegistry,
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Project {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "derived-data-{tag}-{}",
            AssetId::new().simple_hex()
        ));
        std::fs::create_dir_all(root.join("content")).unwrap();
        Self {
            root,
            registry: AssetRegistry::new(),
        }
    }

    fn leaf(&mut self, name: &str, value: u64) -> AssetId {
        save_content_asset_into(
            &Leaf { value },
            &self.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }

    fn sum<const V: u32>(&mut self, name: &str, tag: &str, a: AssetId, b: AssetId) -> AssetId {
        save_content_asset_into(
            &Sum::<V> {
                tag: tag.to_string(),
                a,
                b,
                total: None,
            },
            &self.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }

    fn cook(&self, name: &str, derived_version: u32, blob: &[u8]) {
        let path = self.root.join(format!("content/{name}.gasset"));
        let bytes = std::fs::read(&path).unwrap();
        let (header, payload) = read_content_asset(&bytes).unwrap();
        let cooked = write_cooked_content_asset(
            &header,
            payload,
            CookedSection {
                derived_version,
                key: 0x5eed,
                blob,
            },
        )
        .unwrap();
        std::fs::write(&path, cooked).unwrap();
    }

    fn cache_dir(&self) -> PathBuf {
        self.root.join("derived-cache")
    }

    fn content_root(&self) -> ContentAssetRoot {
        ContentAssetRoot::Directory(self.root.clone())
    }
}

struct Runtime<const V: u32> {
    server: AssetServer,
    world: World,
}

impl<const V: u32> Runtime<V> {
    fn new(project: &Project, cache: DerivedDataCache) -> Self {
        let mut server = AssetServer::with_content_root(project.content_root());
        let leaves = AssetStore::<Leaf>::new();
        let sums = AssetStore::<Sum<V>>::new();
        server.register_asset(&leaves);
        server.register_derived_asset(&sums);
        server.publish_registry(project.registry.clone());
        server.set_derived_cache(cache);
        let mut world = World::new();
        world.insert_resource(leaves);
        world.insert_resource(sums);
        world.insert_resource(server.clone());
        Self { server, world }
    }

    fn pump(&mut self) {
        handle_asset_load_events(&mut self.world);
    }

    fn total(&self, handle: &AssetHandle<Sum<V>>) -> Option<Option<u64>> {
        self.world
            .get_resource::<AssetStore<Sum<V>>>()
            .unwrap()
            .get(handle)
            .map(|sum| sum.total)
    }

    fn load(&mut self, id: AssetId) -> Option<u64> {
        let handle = self.server.load::<Sum<V>>(id);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            if let Some(total) = self.total(&handle) {
                return Some(total.expect("a loaded sum has its blob applied"));
            }
            if !self.server.is_loading() {
                return None;
            }
            assert!(Instant::now() < deadline, "load timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Runtime<1> {
    fn load_leaf(&mut self, id: AssetId) -> Option<u64> {
        let handle = self.server.load::<Leaf>(id);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            if let Some(leaf) = self
                .world
                .get_resource::<AssetStore<Leaf>>()
                .unwrap()
                .get(&handle)
            {
                return Some(leaf.value);
            }
            if !self.server.is_loading() {
                return None;
            }
            assert!(Instant::now() < deadline, "load timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn key_of<const V: u32>(project: &Project, sum: &Sum<V>) -> u64 {
    let inputs = pollster::block_on(DerivedInputs::read(
        &project.content_root(),
        &project.registry,
        &sum.derived_inputs(),
    ))
    .unwrap();
    sum.derived_key(&inputs).unwrap()
}

fn entry_path(cache: &Path, version: u32, key: u64) -> PathBuf {
    cache.join(format!("Sum/v{version}/{key:016x}.blob"))
}

fn sum_value<const V: u32>(tag: &str, a: AssetId, b: AssetId) -> Sum<V> {
    Sum {
        tag: tag.to_string(),
        a,
        b,
        total: None,
    }
}

fn cache_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in read {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(cache_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn a_load_derives_applies_the_blob_and_writes_one_cache_file_at_its_path() {
    let tag = "derives";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let cache = project.cache_dir();

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::at(&cache));
    assert_eq!(runtime.load(id), Some(7));

    let key = key_of(&project, &sum_value::<1>(tag, a, b));
    assert_eq!(cache_files(&cache), vec![entry_path(&cache, 1, key)]);
    assert_eq!(
        std::fs::read(entry_path(&cache, 1, key)).unwrap(),
        7u64.to_le_bytes()
    );
    assert_eq!(derives(tag), 1);
}

#[test]
fn a_second_server_over_the_same_cache_loads_without_deriving() {
    let tag = "hit";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let cache = project.cache_dir();

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    assert_eq!(derives(tag), 1);
}

#[test]
fn changing_an_input_changes_the_key_and_derives_again() {
    let tag = "input-change";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let cache = project.cache_dir();
    let before = key_of(&project, &sum_value::<1>(tag, a, b));

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    project.leaf("b", 10);
    let after = key_of(&project, &sum_value::<1>(tag, a, b));
    assert_ne!(before, after);

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(13)
    );
    assert_eq!(derives(tag), 2);
}

#[test]
fn a_raised_derived_version_misses_the_old_entry() {
    let tag = "version";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let cache = project.cache_dir();

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    assert_eq!(
        Runtime::<2>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    assert_eq!(derives(tag), 2);
    let key = key_of(&project, &sum_value::<2>(tag, a, b));
    assert!(entry_path(&cache, 2, key).is_file());
}

#[test]
fn a_cached_blob_that_apply_refuses_is_derived_again_and_overwritten() {
    let tag = "refused";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let cache = project.cache_dir();
    let path = entry_path(&cache, 1, key_of(&project, &sum_value::<1>(tag, a, b)));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"bad").unwrap();

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    assert_eq!(derives(tag), 1);
    assert_eq!(std::fs::read(&path).unwrap(), 7u64.to_le_bytes());
}

#[test]
fn a_missing_input_fails_the_load_naming_it() {
    let tag = "missing";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let missing = AssetId::new();
    let id = project.sum::<1>("sum", tag, a, missing);

    let error = pollster::block_on(DerivedInputs::read(
        &project.content_root(),
        &project.registry,
        &[a, missing],
    ))
    .expect_err("a missing input is an error");
    assert!(
        format!("{error:#}").contains(&format!("{missing:?}")),
        "got: {error:#}"
    );

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::none());
    assert_eq!(runtime.load(id), None);
    assert_eq!(derives(tag), 0);
}

#[test]
fn a_derived_input_fails_the_load_naming_it() {
    let tag = "derived-input";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let inner = project.sum::<1>("inner", tag, a, b);
    let outer = project.sum::<1>("outer", tag, a, inner);

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::none());
    let error = pollster::block_on(DerivedInputs::read(
        &project.content_root(),
        &project.registry,
        &[a, inner],
    ))
    .expect_err("a derived input is an error");
    let message = format!("{error:#}");
    assert!(
        message.contains("derived assets can't derive from derived assets")
            && message.contains(&format!("{inner:?}")),
        "got: {message}"
    );

    assert_eq!(runtime.load(outer), None);
    assert_eq!(derives(tag), 0);
}

#[test]
fn a_derive_error_fails_the_load() {
    let tag = "derive-error";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    with_probe(tag, |probe| probe.fail_derive = true);
    let cache = project.cache_dir();

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::at(&cache));
    assert_eq!(runtime.load(id), None);
    assert_eq!(derives(tag), 1);
    assert!(cache_files(&cache).is_empty());
}

#[test]
fn a_cache_whose_root_is_a_file_still_loads_with_the_blob_in_memory() {
    let tag = "root-is-file";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let cache = project.cache_dir();
    std::fs::write(&cache, b"not a directory").unwrap();

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::at(&cache));
    assert_eq!(runtime.load(id), Some(7));
    assert!(cache.is_file());
}

#[test]
fn with_no_cache_every_load_derives() {
    let tag = "no-cache";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::none()).load(id),
        Some(7)
    );
    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::none()).load(id),
        Some(7)
    );
    assert_eq!(derives(tag), 2);
    assert!(!project.cache_dir().exists());
}

#[test]
fn a_handle_never_reads_as_loaded_before_its_blob_is_applied() {
    let tag = "gated";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let (release, gate) = mpsc::channel();
    with_probe(tag, |probe| probe.gate = Some(gate));

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::none());
    let handle = runtime.server.load::<Sum<1>>(id);
    let deadline = Instant::now() + Duration::from_secs(10);
    while derives(tag) == 0 {
        runtime.pump();
        assert_eq!(runtime.total(&handle), None);
        assert!(Instant::now() < deadline, "derive never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..20 {
        runtime.pump();
        assert_eq!(runtime.total(&handle), None);
        assert!(runtime.server.is_loading());
        std::thread::sleep(Duration::from_millis(1));
    }
    release.send(()).unwrap();
    assert_eq!(runtime.load(id), Some(7));
}

#[test]
fn a_cooked_blob_loads_without_its_inputs_and_without_deriving() {
    let tag = "cooked";
    let mut project = Project::new(tag);
    let id = project.sum::<1>("sum", tag, AssetId::new(), AssetId::new());
    project.cook("sum", 1, &42u64.to_le_bytes());
    let cache = project.cache_dir();

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::at(&cache));
    assert_eq!(runtime.load(id), Some(42));
    assert_eq!(derives(tag), 0);
    assert!(!cache.exists());
}

#[test]
fn a_cooked_blob_of_another_derived_version_is_derived_again() {
    let tag = "cooked-version";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    project.cook("sum", 2, &42u64.to_le_bytes());

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::none());
    assert_eq!(runtime.load(id), Some(7));
    assert_eq!(derives(tag), 1);
}

#[test]
fn a_cooked_blob_that_apply_refuses_is_derived_again() {
    let tag = "cooked-refused";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    project.cook("sum", 1, b"bad");

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::none());
    assert_eq!(runtime.load(id), Some(7));
    assert_eq!(derives(tag), 1);
}

#[test]
fn a_derived_section_on_an_asset_that_is_not_derived_is_ignored() {
    let tag = "cooked-leaf";
    let mut project = Project::new(tag);
    let id = project.leaf("leaf", 5);
    project.cook("leaf", 1, b"stray");

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::none());
    assert_eq!(runtime.load_leaf(id), Some(5));
}

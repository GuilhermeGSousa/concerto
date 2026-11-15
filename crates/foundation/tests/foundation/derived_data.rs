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
        read_asset_record, read_content_asset, read_content_asset_parts, save_content_asset_into,
        write_content_asset_with_embedded, write_cooked_content_asset, ContentAssetHeader,
        CookedSection, DerivedSection, EmbeddedPayload, ImportProvenance,
    },
    derived::{DerivedData, DerivedDataCache, DerivedInputs},
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry, EmbeddedIn},
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

#[derive(Serialize, Deserialize)]
struct Maybe {
    tag: String,
    derive: bool,
    value: u64,
    inputs: Vec<AssetId>,
    #[serde(skip)]
    total: Option<u64>,
}

impl Maybe {
    fn new(tag: &str, derive: bool, value: u64, inputs: Vec<AssetId>) -> Self {
        Self {
            tag: tag.to_string(),
            derive,
            value,
            inputs,
            total: None,
        }
    }
}

impl Asset for Maybe {
    fn name() -> &'static str {
        "Maybe"
    }
}

impl LoadableAsset for Maybe {}

fn input_value(inputs: &DerivedInputs, id: AssetId) -> anyhow::Result<u64> {
    match inputs.decode::<Leaf>(id) {
        Ok(leaf) => Ok(leaf.value),
        Err(_) => Ok(inputs.decode::<Maybe>(id)?.value),
    }
}

impl DerivedData for Maybe {
    const DERIVED_VERSION: u32 = 1;

    fn is_derived(&self) -> bool {
        self.derive
    }

    fn derived_inputs(&self) -> Vec<AssetId> {
        self.inputs.clone()
    }

    fn derived_key(&self, inputs: &DerivedInputs) -> anyhow::Result<u64> {
        let mut hasher = DefaultHasher::new();
        self.value.hash(&mut hasher);
        for &id in &self.inputs {
            inputs.bytes(id).unwrap().hash(&mut hasher);
        }
        Ok(hasher.finish())
    }

    fn derive(
        &self,
        inputs: &DerivedInputs,
        _progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>> {
        with_probe(&self.tag, |probe| probe.derives += 1);
        let mut total = self.value;
        for &id in &self.inputs {
            total += input_value(inputs, id)?;
        }
        Ok(total.to_le_bytes().to_vec())
    }

    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
        let bytes: [u8; 8] = blob
            .try_into()
            .map_err(|_| anyhow::anyhow!("a maybe blob is 8 bytes, not {}", blob.len()))?;
        self.total = Some(u64::from_le_bytes(bytes));
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Pointer {
    target: AssetId,
}

impl Asset for Pointer {
    fn name() -> &'static str {
        "Pointer"
    }
}

impl LoadableAsset for Pointer {}

#[derive(Serialize, Deserialize)]
struct Chain {
    tag: String,
    pointer: AssetId,
    #[serde(skip)]
    total: Option<u64>,
}

impl Chain {
    fn new(tag: &str, pointer: AssetId) -> Self {
        Self {
            tag: tag.to_string(),
            pointer,
            total: None,
        }
    }
}

impl Asset for Chain {
    fn name() -> &'static str {
        "Chain"
    }
}

impl LoadableAsset for Chain {}

impl DerivedData for Chain {
    const DERIVED_VERSION: u32 = 1;

    fn derived_inputs(&self) -> Vec<AssetId> {
        vec![self.pointer]
    }

    fn further_inputs(&self, inputs: &DerivedInputs) -> anyhow::Result<Vec<AssetId>> {
        Ok(vec![inputs.decode::<Pointer>(self.pointer)?.target])
    }

    fn derived_key(&self, inputs: &DerivedInputs) -> anyhow::Result<u64> {
        let target = inputs.decode::<Pointer>(self.pointer)?.target;
        let mut hasher = DefaultHasher::new();
        for id in [self.pointer, target] {
            inputs
                .bytes(id)
                .ok_or_else(|| anyhow::anyhow!("input {id:?} was not read"))?
                .hash(&mut hasher);
        }
        Ok(hasher.finish())
    }

    fn derive(
        &self,
        inputs: &DerivedInputs,
        _progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>> {
        with_probe(&self.tag, |probe| probe.derives += 1);
        let target = inputs.decode::<Pointer>(self.pointer)?.target;
        Ok(input_value(inputs, target)?.to_le_bytes().to_vec())
    }

    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
        let bytes: [u8; 8] = blob
            .try_into()
            .map_err(|_| anyhow::anyhow!("a chain blob is 8 bytes, not {}", blob.len()))?;
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

    fn maybe(&mut self, name: &str, value: &Maybe) -> AssetId {
        save_content_asset_into(
            value,
            &self.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }

    fn pointer(&mut self, name: &str, target: AssetId) -> AssetId {
        save_content_asset_into(
            &Pointer { target },
            &self.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }

    fn chain(&mut self, name: &str, tag: &str, pointer: AssetId) -> AssetId {
        save_content_asset_into(
            &Chain::new(tag, pointer),
            &self.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }

    fn embed<A: Asset>(&mut self, name: &str, embedded_as: &str, value: &A) -> AssetId {
        let address = format!("content/{name}.gasset");
        let header = |kind: &str, kind_version: u32, sub_asset: &str| ContentAssetHeader {
            asset_id: AssetId::new(),
            references: Vec::new(),
            kind: kind.to_string(),
            kind_version,
            provenance: Some(ImportProvenance {
                source: format!("assets/{name}.glb"),
                sub_asset: sub_asset.to_string(),
            }),
            derived: None,
            embedded: Vec::new(),
        };
        let host = header(Leaf::name(), Leaf::FORMAT_VERSION, "scene");
        let embedded = header(A::name(), A::FORMAT_VERSION, embedded_as);
        let file = write_content_asset_with_embedded(
            &host,
            &bincode::serialize(&Leaf { value: 0 }).unwrap(),
            &[EmbeddedPayload {
                header: embedded.clone(),
                payload: &bincode::serialize(value).unwrap(),
                derived_blob: None,
            }],
        )
        .unwrap();
        std::fs::write(self.root.join(&address), file).unwrap();
        let id = embedded.asset_id;
        let embedded_in = EmbeddedIn {
            host: host.asset_id,
            name: embedded_as.to_string(),
        };
        let mut register = |header: ContentAssetHeader, embedded_in: Option<EmbeddedIn>| {
            self.registry.insert(AssetEntry {
                id: header.asset_id,
                address: address.clone(),
                kind: header.kind,
                kind_version: Some(header.kind_version),
                provenance: header.provenance,
                embedded_in,
            });
        };
        register(host, None);
        register(embedded, Some(embedded_in));
        id
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
        let maybes = AssetStore::<Maybe>::new();
        let pointers = AssetStore::<Pointer>::new();
        let chains = AssetStore::<Chain>::new();
        server.register_asset(&leaves);
        server.register_derived_asset(&sums);
        server.register_derived_asset(&maybes);
        server.register_asset(&pointers);
        server.register_derived_asset(&chains);
        server.publish_registry(project.registry.clone());
        server.set_derived_cache(cache);
        let mut world = World::new();
        world.insert_resource(leaves);
        world.insert_resource(sums);
        world.insert_resource(maybes);
        world.insert_resource(pointers);
        world.insert_resource(chains);
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

impl<const V: u32> Runtime<V> {
    fn load_maybe(&mut self, id: AssetId) -> Option<Option<u64>> {
        let handle = self.server.load::<Maybe>(id);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            if let Some(maybe) = self
                .world
                .get_resource::<AssetStore<Maybe>>()
                .unwrap()
                .get(&handle)
            {
                return Some(maybe.total);
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
    fn load_chain(&mut self, id: AssetId) -> Option<u64> {
        let handle = self.server.load::<Chain>(id);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            if let Some(chain) = self
                .world
                .get_resource::<AssetStore<Chain>>()
                .unwrap()
                .get(&handle)
            {
                return Some(chain.total.expect("a loaded chain has its blob applied"));
            }
            if !self.server.is_loading() {
                return None;
            }
            assert!(Instant::now() < deadline, "load timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn chain_inputs(project: &Project, chain: &Chain) -> anyhow::Result<DerivedInputs> {
    let root = project.content_root();
    let mut inputs = pollster::block_on(DerivedInputs::read(
        &root,
        &project.registry,
        &chain.derived_inputs(),
    ))?;
    let further = chain.further_inputs(&inputs)?;
    pollster::block_on(inputs.read_more(&root, &project.registry, &further))?;
    Ok(inputs)
}

fn chain_key(project: &Project, chain: &Chain) -> u64 {
    chain
        .derived_key(&chain_inputs(project, chain).unwrap())
        .unwrap()
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

fn put_cached(cache: &Path, key: u64, blob: &[u8]) {
    let path = entry_path(cache, 1, key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, blob).unwrap();
}

struct CookedFile {
    header: ContentAssetHeader,
    payload: Vec<u8>,
    section: Option<DerivedSection>,
    blob: Vec<u8>,
}

fn read_cooked(path: &Path) -> CookedFile {
    let bytes = std::fs::read(path).unwrap();
    let parts = read_content_asset_parts(&bytes).unwrap();
    CookedFile {
        header: ContentAssetHeader {
            derived: None,
            ..parts.header.clone()
        },
        payload: parts.payload.to_vec(),
        section: parts.header.derived,
        blob: parts.derived_blob.unwrap_or_default().to_vec(),
    }
}

fn assert_cooked_from(project: &Project, out: &Path, address: &str, key: u64, blob: &[u8]) {
    let original = read_cooked(&project.root.join(address));
    let cooked = read_cooked(&out.join(address));
    assert_eq!(cooked.header, original.header);
    assert_eq!(cooked.payload, original.payload);
    assert_eq!(
        cooked.section,
        Some(DerivedSection {
            derived_version: 1,
            key,
            len: blob.len() as u64,
        })
    );
    assert_eq!(cooked.blob, blob);
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
fn the_cache_keys_a_blob_by_version_and_input_bytes_and_replaces_one_apply_refuses() {
    let tag = "cache";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    let id = project.sum::<1>("sum", tag, a, b);
    let cache = project.cache_dir();
    let first = key_of(&project, &sum_value::<1>(tag, a, b));

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    assert_eq!(cache_files(&cache), vec![entry_path(&cache, 1, first)]);
    assert_eq!(
        std::fs::read(entry_path(&cache, 1, first)).unwrap(),
        7u64.to_le_bytes()
    );
    assert_eq!(derives(tag), 1);

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(7)
    );
    assert_eq!(derives(tag), 1, "a second server hits the cache");

    project.leaf("b", 10);
    let changed = key_of(&project, &sum_value::<1>(tag, a, b));
    assert_ne!(first, changed);
    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(13)
    );
    assert_eq!(derives(tag), 2, "a changed input misses");

    assert_eq!(
        Runtime::<2>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(13)
    );
    assert_eq!(derives(tag), 3, "a raised derived version misses");
    assert!(entry_path(&cache, 2, changed).is_file());

    std::fs::write(entry_path(&cache, 1, changed), b"bad").unwrap();
    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(id),
        Some(13)
    );
    assert_eq!(derives(tag), 4, "a blob apply refuses is derived again");
    assert_eq!(
        std::fs::read(entry_path(&cache, 1, changed)).unwrap(),
        13u64.to_le_bytes()
    );

    let embedded = project.embed("host", "leaf/0", &Leaf { value: 4 });
    let over = project.sum::<1>("over", tag, a, embedded);
    let embedded_key = key_of(&project, &sum_value::<1>(tag, a, embedded));
    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load(over),
        Some(7),
        "an input embedded in a host file is read from it"
    );
    assert!(entry_path(&cache, 1, embedded_key).is_file());

    let entry = project.registry.get(embedded).unwrap().clone();
    let loose = "content/loose.gasset";
    std::fs::write(
        project.root.join(loose),
        read_asset_record(&project.root, &entry).unwrap(),
    )
    .unwrap();
    project.registry.insert(AssetEntry {
        address: loose.to_string(),
        embedded_in: None,
        ..entry
    });
    assert_eq!(
        key_of(&project, &sum_value::<1>(tag, a, embedded)),
        embedded_key,
        "the key is the same for an input embedded and written as a loose file"
    );

    let host_path = project.root.join("content/host.gasset");
    let host = project
        .registry
        .id_for_address("content/host.gasset")
        .unwrap();
    let host_key = key_of(&project, &sum_value::<1>(tag, a, host));
    let file = std::fs::read(&host_path).unwrap();
    let parts = read_content_asset_parts(&file).unwrap();
    let rewritten = write_content_asset_with_embedded(
        &parts.header.record_header(None).unwrap(),
        parts.payload,
        &[EmbeddedPayload {
            header: parts.header.record_header(Some("leaf/0")).unwrap(),
            payload: &bincode::serialize(&Leaf { value: 9 }).unwrap(),
            derived_blob: None,
        }],
    )
    .unwrap();
    assert_ne!(rewritten, file);
    std::fs::write(&host_path, rewritten).unwrap();
    assert_eq!(
        key_of(&project, &sum_value::<1>(tag, a, host)),
        host_key,
        "the key over a host is unchanged when one of its embedded assets changes"
    );
}

struct Uncached {
    tag: &'static str,
    cache: fn(&Project) -> DerivedDataCache,
    after: fn(&Project) -> bool,
}

#[test]
fn without_a_writable_cache_every_load_derives_with_the_blob_in_memory() {
    let cases = [Uncached {
        tag: "no-cache",
        cache: |_| DerivedDataCache::none(),
        after: |project| !project.cache_dir().exists(),
    }];
    for case in cases {
        let mut project = Project::new(case.tag);
        let a = project.leaf("a", 3);
        let b = project.leaf("b", 4);
        let id = project.sum::<1>("sum", case.tag, a, b);
        let cache = (case.cache)(&project);

        assert_eq!(Runtime::<1>::new(&project, cache.clone()).load(id), Some(7));
        assert_eq!(Runtime::<1>::new(&project, cache).load(id), Some(7));
        assert_eq!(derives(case.tag), 2, "{}", case.tag);
        assert!((case.after)(&project), "{}", case.tag);
    }
}

struct Cooked {
    tag: &'static str,
    inputs: bool,
    derived_version: u32,
    blob: &'static [u8],
    total: u64,
    derives: usize,
}

#[test]
fn a_cooked_blob_of_the_current_version_loads_without_inputs_and_any_other_is_derived_again() {
    let cases = [
        Cooked {
            tag: "cooked",
            inputs: false,
            derived_version: 1,
            blob: &[42, 0, 0, 0, 0, 0, 0, 0],
            total: 42,
            derives: 0,
        },
        Cooked {
            tag: "cooked-version",
            inputs: true,
            derived_version: 2,
            blob: &[42, 0, 0, 0, 0, 0, 0, 0],
            total: 7,
            derives: 1,
        },
    ];
    for case in cases {
        let mut project = Project::new(case.tag);
        let leaf = |project: &mut Project, name: &str, value: u64| {
            if case.inputs {
                project.leaf(name, value)
            } else {
                AssetId::new()
            }
        };
        let a = leaf(&mut project, "a", 3);
        let b = leaf(&mut project, "b", 4);
        let id = project.sum::<1>("sum", case.tag, a, b);
        project.cook("sum", case.derived_version, case.blob);
        let cache = project.cache_dir();

        let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::at(&cache));
        assert_eq!(runtime.load(id), Some(case.total), "{}", case.tag);
        assert_eq!(derives(case.tag), case.derives, "{}", case.tag);
        assert_eq!(cache.exists(), case.derives > 0, "{}", case.tag);
    }
}

struct Failing {
    load: Option<u64>,
    read_error: anyhow::Result<DerivedInputs>,
}

struct Failure {
    tag: &'static str,
    build: fn(&mut Project, &'static str) -> Failing,
    derives: usize,
}

fn read_inputs(project: &Project, ids: &[AssetId]) -> anyhow::Result<DerivedInputs> {
    pollster::block_on(DerivedInputs::read(
        &project.content_root(),
        &project.registry,
        ids,
    ))
}

#[test]
fn a_load_fails_on_a_missing_input_or_a_derive_error() {
    let cases = [
        Failure {
            tag: "missing",
            build: |project, tag| {
                let a = project.leaf("a", 3);
                let missing = AssetId::new();
                let id = project.sum::<1>("sum", tag, a, missing);
                Failing {
                    load: Runtime::<1>::new(project, DerivedDataCache::none()).load(id),
                    read_error: read_inputs(project, &[a, missing]),
                }
            },
            derives: 0,
        },
        Failure {
            tag: "derive-error",
            build: |project, tag| {
                let a = project.leaf("a", 3);
                let b = project.leaf("b", 4);
                let id = project.sum::<1>("sum", tag, a, b);
                with_probe(tag, |probe| probe.fail_derive = true);
                let cache = project.cache_dir();
                let load = Runtime::<1>::new(project, DerivedDataCache::at(&cache)).load(id);
                assert!(cache_files(&cache).is_empty());
                Failing {
                    load,
                    read_error: Err(anyhow::anyhow!("the inputs read")),
                }
            },
            derives: 1,
        },
    ];
    for case in cases {
        let mut project = Project::new(case.tag);
        let failing = (case.build)(&mut project, case.tag);
        assert_eq!(failing.load, None, "{}", case.tag);
        assert_eq!(derives(case.tag), case.derives, "{}", case.tag);
        assert!(failing.read_error.is_err(), "{}", case.tag);
    }
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
fn a_cook_writes_each_derived_entry_with_its_payload_key_and_blob_and_nothing_else() {
    let tag = "cook";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    project.sum::<1>("sum", tag, a, b);
    project.sum::<1>("nested/double", tag, a, a);
    let out = project.root.join("cooked");

    let server = Runtime::<1>::new(&project, DerivedDataCache::none()).server;
    assert!(server.can_cook("Sum"));
    assert!(!server.can_cook("Leaf"));
    let report = server.cook(&out).unwrap();

    assert_eq!(
        report.cooked,
        vec!["content/nested/double.gasset", "content/sum.gasset"]
    );
    let mut files = cache_files(&out);
    files.sort();
    assert_eq!(
        files,
        vec![
            out.join("content/nested/double.gasset"),
            out.join("content/sum.gasset")
        ]
    );
    assert_cooked_from(
        &project,
        &out,
        "content/sum.gasset",
        key_of(&project, &sum_value::<1>(tag, a, b)),
        &7u64.to_le_bytes(),
    );
    assert_cooked_from(
        &project,
        &out,
        "content/nested/double.gasset",
        key_of(&project, &sum_value::<1>(tag, a, a)),
        &6u64.to_le_bytes(),
    );
    assert_eq!(derives(tag), 2);
}

struct CookCached {
    tag: &'static str,
    cached: &'static [u8],
    blob: u64,
    derives: usize,
}

#[test]
fn a_cook_uses_a_cached_blob_unless_apply_refuses_it() {
    let cases = [CookCached {
        tag: "cook-hit",
        cached: &[99, 0, 0, 0, 0, 0, 0, 0],
        blob: 99,
        derives: 0,
    }];
    for case in cases {
        let mut project = Project::new(case.tag);
        let a = project.leaf("a", 3);
        let b = project.leaf("b", 4);
        project.sum::<1>("sum", case.tag, a, b);
        let cache = project.cache_dir();
        let key = key_of(&project, &sum_value::<1>(case.tag, a, b));
        put_cached(&cache, key, case.cached);
        let out = project.root.join("cooked");

        Runtime::<1>::new(&project, DerivedDataCache::at(&cache))
            .server
            .cook(&out)
            .unwrap();

        assert_cooked_from(
            &project,
            &out,
            "content/sum.gasset",
            key,
            &case.blob.to_le_bytes(),
        );
        assert_eq!(derives(case.tag), case.derives, "{}", case.tag);
    }
}

#[test]
fn a_failing_derive_or_an_embedded_derived_asset_stops_the_cook() {
    let tag = "cook-fail";
    let after = "cook-fail-after";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let b = project.leaf("b", 4);
    project.sum::<1>("first", tag, a, b);
    project.sum::<1>("second", after, a, b);
    with_probe(tag, |probe| probe.fail_derive = true);
    let out = project.root.join("cooked");

    Runtime::<1>::new(&project, DerivedDataCache::none())
        .server
        .cook(&out)
        .expect_err("a failing derive fails the cook");

    assert!(cache_files(&out).is_empty());
    assert_eq!(derives(after), 0);

    let tag = "cook-embedded";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    project.embed("host", "sum/0", &sum_value::<1>(tag, a, a));
    let out = project.root.join("cooked");

    Runtime::<1>::new(&project, DerivedDataCache::none())
        .server
        .cook(&out)
        .expect_err("an embedded asset that derives fails the cook");

    assert!(cache_files(&out).is_empty());
    assert_eq!(derives(tag), 0);
}

#[test]
fn an_instance_derives_only_when_it_says_so_and_one_that_does_not_reads_no_inputs() {
    let tag = "maybe";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let plain = project.maybe("plain", &Maybe::new(tag, false, 5, vec![AssetId::new()]));
    let derived = project.maybe("derived", &Maybe::new(tag, true, 5, vec![a]));
    let cache = project.cache_dir();

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::at(&cache));
    assert_eq!(runtime.load_maybe(plain), Some(None));
    assert_eq!(derives(tag), 0);
    assert!(!cache.exists());
    assert_eq!(runtime.load_maybe(derived), Some(Some(8)));
    assert_eq!(derives(tag), 1);
}

#[test]
fn an_input_that_does_not_derive_is_accepted_and_one_that_derives_is_refused() {
    let tag = "maybe-input";
    let mut project = Project::new(tag);
    let a = project.leaf("a", 3);
    let plain = project.maybe("plain", &Maybe::new(tag, false, 4, vec![AssetId::new()]));
    let derived = project.maybe("derived", &Maybe::new(tag, true, 6, vec![a]));
    let over_plain = project.maybe("over-plain", &Maybe::new(tag, true, 1, vec![plain]));
    let over_derived = project.maybe("over-derived", &Maybe::new(tag, true, 1, vec![derived]));

    read_inputs(&project, &[a, plain]).expect("an input that does not derive is accepted");

    read_inputs(&project, &[a, derived]).expect_err("an input that derives is refused");

    let mut runtime = Runtime::<1>::new(&project, DerivedDataCache::none());
    assert_eq!(runtime.load_maybe(over_plain), Some(Some(5)));
    assert_eq!(runtime.load_maybe(over_derived), None);
    assert_eq!(derives(tag), 1);
}

#[test]
fn further_inputs_are_read_and_seen_by_the_key_and_the_derive() {
    let tag = "further";
    let mut project = Project::new(tag);
    let target = project.leaf("target", 7);
    let pointer = project.pointer("pointer", target);
    let id = project.chain("chain", tag, pointer);
    let cache = project.cache_dir();
    let before = chain_key(&project, &Chain::new(tag, pointer));

    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load_chain(id),
        Some(7)
    );
    assert_eq!(
        cache_files(&cache),
        vec![cache.join(format!("Chain/v1/{before:016x}.blob"))]
    );
    assert_eq!(derives(tag), 1);

    project.leaf("target", 10);
    assert_ne!(before, chain_key(&project, &Chain::new(tag, pointer)));
    assert_eq!(
        Runtime::<1>::new(&project, DerivedDataCache::at(&cache)).load_chain(id),
        Some(10)
    );
    assert_eq!(derives(tag), 2);
}

#[test]
fn a_cook_reads_further_inputs() {
    let tag = "further-cook";
    let mut project = Project::new(tag);
    let target = project.leaf("target", 7);
    let pointer = project.pointer("pointer", target);
    project.chain("chain", tag, pointer);
    let out = project.root.join("cooked");

    let report = Runtime::<1>::new(&project, DerivedDataCache::none())
        .server
        .cook(&out)
        .unwrap();

    assert_eq!(report.cooked, vec!["content/chain.gasset"]);
    assert_cooked_from(
        &project,
        &out,
        "content/chain.gasset",
        chain_key(&project, &Chain::new(tag, pointer)),
        &7u64.to_le_bytes(),
    );
    assert_eq!(derives(tag), 1);
}

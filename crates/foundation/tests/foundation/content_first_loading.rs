//! load_content_asset reads a content asset at <root>/<address>.
use concerto_foundation::assets::content::{write_content_asset, ContentAssetHeader};
use concerto_foundation::assets::utils::load_content_asset;
use concerto_foundation::assets::{Asset, AssetId, ContentAssetRoot};

#[derive(Asset, serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Pebble {
    weight: u32,
}

#[derive(Asset, serde::Serialize, serde::Deserialize, Debug)]
struct Boulder {
    weight: u32,
}

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

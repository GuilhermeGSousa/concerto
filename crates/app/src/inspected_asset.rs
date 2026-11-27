use std::path::Path;

use anyhow::Context;
use concerto_ecs::resource::Resource;
use concerto_foundation::assets::{
    content::{decode_content_asset, save_content_asset_into},
    registry::AssetRegistry,
    Asset, AssetId, CreatableAsset,
};
use concerto_reflect::Reflect;

/// What the editor needs to create, load, show and save an asset kind it can't name.
#[derive(Clone, Copy)]
pub struct InspectedAsset {
    /// The asset kind, as [`Asset::name`] gives it.
    pub kind: &'static str,
    /// Creates a new asset of the kind at an address under a project root, records it in the
    /// registry, and returns its id.
    pub create: fn(&Path, &str, &mut AssetRegistry) -> anyhow::Result<AssetId>,
    /// Decodes a content asset file's bytes into its id and its value.
    pub decode: fn(&[u8]) -> anyhow::Result<DecodedAsset>,
    /// Saves a value of the kind to an address under a project root, records it in the registry,
    /// and returns its id.
    pub save: fn(&dyn Reflect, &Path, &str, &mut AssetRegistry) -> anyhow::Result<AssetId>,
}

impl InspectedAsset {
    /// The record of `A`.
    pub fn of<A: CreatableAsset + Reflect>() -> Self {
        Self {
            kind: A::name(),
            create: create::<A>,
            decode: decode::<A>,
            save: save::<A>,
        }
    }
}

/// An inspected asset read from its file.
pub struct DecodedAsset {
    /// The asset's id, read from its header.
    pub id: AssetId,
    /// The asset's value.
    pub value: Box<dyn Reflect>,
}

/// Every asset kind the game marked with `register_inspected_asset`, in registration order.
#[derive(Resource, Default)]
pub struct InspectedAssets(Vec<InspectedAsset>);

impl InspectedAssets {
    /// The records, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &InspectedAsset> {
        self.0.iter()
    }

    pub(crate) fn insert(&mut self, record: InspectedAsset) {
        if self.0.iter().all(|known| known.kind != record.kind) {
            self.0.push(record);
        }
    }
}

fn create<A: CreatableAsset>(
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    save_content_asset_into(&A::new_asset(), project_root, address, registry)
}

fn decode<A: Asset + Reflect>(bytes: &[u8]) -> anyhow::Result<DecodedAsset> {
    let (header, value) = decode_content_asset::<A>(bytes)?;
    Ok(DecodedAsset {
        id: header.asset_id,
        value: Box::new(value),
    })
}

fn save<A: Asset + Reflect>(
    value: &dyn Reflect,
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    let value = value
        .downcast_ref::<A>()
        .with_context(|| format!("the value to save is not a {}", A::name()))?;
    save_content_asset_into(value, project_root, address, registry)
}

#[cfg(test)]
mod tests {
    use crate::{plugins::AssetManagerPlugin, App};
    use concerto_foundation::assets::{
        content::decode_content_asset, registry::AssetRegistry, Asset, AssetId, CreatableAsset,
        LoadableAsset,
    };
    use concerto_reflect::Reflect;

    use super::InspectedAssets;

    #[derive(serde::Serialize, serde::Deserialize, Reflect, PartialEq, Debug)]
    struct Note {
        text: String,
        weight: f32,
    }
    impl Asset for Note {
        fn name() -> &'static str {
            "TestNote"
        }
    }
    impl CreatableAsset for Note {
        fn new_asset() -> Self {
            Note {
                text: "hello".into(),
                weight: 1.5,
            }
        }
    }
    impl LoadableAsset for Note {}

    #[derive(serde::Serialize, serde::Deserialize, Reflect)]
    struct Tag {
        label: String,
    }
    impl Asset for Tag {
        fn name() -> &'static str {
            "TestTag"
        }
    }
    impl CreatableAsset for Tag {
        fn new_asset() -> Self {
            Tag {
                label: String::new(),
            }
        }
    }
    impl LoadableAsset for Tag {}

    fn app() -> App {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app
    }

    #[test]
    fn a_record_creates_decodes_and_saves_its_asset_and_refuses_another_type() {
        let mut app = app();
        app.register_inspected_asset::<Note>();
        let record = app
            .get_resource::<InspectedAssets>()
            .unwrap()
            .iter()
            .next()
            .copied()
            .unwrap();
        let root =
            std::env::temp_dir().join(format!("inspected-asset-{}", AssetId::new().simple_hex()));
        let address = "content/notes/first.gasset";
        let mut registry = AssetRegistry::new();

        let id = (record.create)(&root, address, &mut registry).unwrap();
        assert_eq!(registry.get(id).unwrap().kind, "TestNote");
        let mut decoded = (record.decode)(&std::fs::read(root.join(address)).unwrap()).unwrap();
        assert_eq!(decoded.id, id);
        assert_eq!(
            decoded.value.downcast_ref::<Note>(),
            Some(&Note::new_asset())
        );

        decoded.value.downcast_mut::<Note>().unwrap().text = "changed".into();
        let saved = (record.save)(decoded.value.as_ref(), &root, address, &mut registry).unwrap();

        assert_eq!(saved, id);
        let (header, note) =
            decode_content_asset::<Note>(&std::fs::read(root.join(address)).unwrap()).unwrap();
        assert_eq!(header.asset_id, id);
        assert_eq!(
            note,
            Note {
                text: "changed".into(),
                weight: 1.5,
            }
        );

        let wrong = Tag {
            label: "tag".into(),
        };
        assert!((record.save)(&wrong, &root, "content/x.gasset", &mut registry).is_err());
        assert!(!root.join("content/x.gasset").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

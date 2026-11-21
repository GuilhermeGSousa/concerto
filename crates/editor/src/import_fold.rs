//! How the catalogue folds an import's output under the scene it made.
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use concerto_foundation::assets::{
    AssetId,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_scene::scene::Scene;

const FACE_SUB_ASSET: &str = "scene";

/// Whether `asset` is the scene an import made, which the catalogue shows in place of the import's whole output.
pub fn is_import_face(asset: &AssetEntry) -> bool {
    asset.is::<Scene>()
        && asset
            .provenance
            .as_ref()
            .is_some_and(|provenance| provenance.sub_asset == FACE_SUB_ASSET)
}

/// The name the catalogue shows for `asset`: its source's file name without the extension for an import's scene, its own file name otherwise.
pub fn shown_name(asset: &AssetEntry) -> Cow<'_, str> {
    asset
        .provenance
        .as_ref()
        .filter(|_| is_import_face(asset))
        .and_then(|provenance| Path::new(&provenance.source).file_stem())
        .and_then(|stem| stem.to_str())
        .map_or_else(|| asset.name(), Cow::Borrowed)
}

/// The imports of a registry that have a scene, each folded into that scene, its face, with the rest of its output as the face's parts.
pub struct ImportFolds<'a> {
    faces: HashMap<&'a str, &'a AssetEntry>,
    parts: HashMap<AssetId, Vec<&'a AssetEntry>>,
}

impl<'a> ImportFolds<'a> {
    pub fn new(registry: &'a AssetRegistry) -> Self {
        let mut faces: HashMap<&str, &AssetEntry> = HashMap::new();
        for asset in registry.iter().filter(|asset| is_import_face(asset)) {
            if let Some(provenance) = &asset.provenance {
                faces.entry(&provenance.source).or_insert(asset);
            }
        }
        let mut folds = Self {
            faces,
            parts: HashMap::new(),
        };
        for asset in registry.iter() {
            if let Some(face) = folds.face_of(asset) {
                folds.parts.entry(face.id).or_default().push(asset);
            }
        }
        folds
    }

    /// The face `asset` is folded under; `None` for a face and for an asset that stands alone.
    pub fn face_of(&self, asset: &AssetEntry) -> Option<&'a AssetEntry> {
        let provenance = asset.provenance.as_ref()?;
        self.faces
            .get(provenance.source.as_str())
            .copied()
            .filter(|face| face.id != asset.id)
    }

    /// The assets folded under `face`, in registry order; empty for any other asset.
    pub fn parts_of(&self, face: AssetId) -> &[&'a AssetEntry] {
        self.parts.get(&face).map_or(&[], Vec::as_slice)
    }

    /// Whether `asset` matches the lower-case `query`: a part by its own name and kind, any other asset by its shown name, address and kind.
    pub fn matches(&self, asset: &AssetEntry, query: &str) -> bool {
        let has = |text: &str| text.to_lowercase().contains(query);
        if self.face_of(asset).is_some() {
            has(&asset.name()) || has(&asset.kind)
        } else {
            has(&shown_name(asset)) || has(&asset.address) || has(&asset.kind)
        }
    }
}

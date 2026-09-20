pub mod render_mesh;
pub mod render_texture;
pub mod render_window;

use std::collections::HashMap;

use app::{extractor::Extracted, plugins::Plugin, schedule_groups::Extract};
use ecs::{
    resource::{Res, ResMut, Resource},
    system::input::{StaticSystemInput, SystemInput, SystemInputData},
};
use essential::assets::{asset_store::AssetStore, Asset, AssetId};

pub enum AssetPreparationError {
    NotReady,
}

pub trait RenderAsset: Send + Sync + Sized + 'static {
    type SourceAsset: Asset;
    type PreparationParams: SystemInput;

    fn prepare_asset(
        source_asset: &Self::SourceAsset,
        params: &mut SystemInputData<Self::PreparationParams>,
    ) -> Result<Self, AssetPreparationError>;
}

pub(crate) fn prepare_render_asset<A: RenderAsset>(
    mut params: StaticSystemInput<<A as RenderAsset>::PreparationParams>,
    asset_store: Extracted<Res<AssetStore<A::SourceAsset>>>,
    mut render_assets: ResMut<RenderAssets<A>>,
) {
    for (asset_id, asset) in asset_store.into_iter() {
        if render_assets.contains(asset_id) {
            continue;
        }

        let prepared_asset = A::prepare_asset(asset, &mut params);

        if let Ok(prepared_asset) = prepared_asset {
            render_assets.insert(*asset_id, prepared_asset)
        }
    }
}

#[derive(Resource)]
pub struct RenderAssets<A: RenderAsset + 'static>(HashMap<AssetId, A>);

impl<A: RenderAsset + 'static> RenderAssets<A> {
    pub fn new() -> Self {
        RenderAssets(HashMap::new())
    }

    pub fn insert(&mut self, id: AssetId, asset: A) {
        self.0.insert(id, asset);
    }

    pub fn get(&self, id: &AssetId) -> Option<&A> {
        self.0.get(id)
    }

    pub fn contains(&self, id: &AssetId) -> bool {
        self.0.contains_key(id)
    }
}

impl<A: RenderAsset + 'static> Default for RenderAssets<A> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RenderAssetPlugin<A: RenderAsset> {
    _marker: std::marker::PhantomData<A>,
}

impl<A: RenderAsset> RenderAssetPlugin<A> {
    pub fn new() -> Self {
        RenderAssetPlugin {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<A: RenderAsset> Default for RenderAssetPlugin<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: RenderAsset + 'static> Plugin for RenderAssetPlugin<A> {
    fn build(&self, app: &mut app::App) {
        app.render_mut().insert_resource(RenderAssets::<A>::new());
        app.add_render_system(Extract, prepare_render_asset::<A>);
    }
}

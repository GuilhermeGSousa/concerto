pub mod render_mesh;
pub mod render_texture;
pub mod render_window;

use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use concerto_app::{extractor::Extracted, plugins::Plugin, schedule_groups::Extract, App};
use concerto_ecs::{
    resource::{Res, ResMut, Resource},
    system::input::{StaticSystemInput, SystemInput, SystemInputData},
    IntoSystemConfig, SystemSet,
};
use concerto_foundation::assets::{asset_store::AssetStore, Asset, AssetId};

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

    /// The bytes preparing `source` uploads, counted against [`RenderAssetBytesPerFrame`]; `None` is never deferred.
    fn byte_len(_source: &Self::SourceAsset) -> Option<usize> {
        None
    }
}

/// The most bytes of render assets prepared in one frame; `None` prepares everything at once.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderAssetBytesPerFrame {
    pub max_bytes: Option<usize>,
}

/// The render world's count of the bytes prepared this frame against [`RenderAssetBytesPerFrame`].
#[derive(Resource, Default)]
pub struct RenderAssetBytesPerFrameLimiter {
    max_bytes: Option<usize>,
    bytes_written: AtomicUsize,
    prepared: AtomicBool,
    deferred: AtomicBool,
}

impl RenderAssetBytesPerFrameLimiter {
    fn reset(&mut self, max_bytes: Option<usize>) {
        self.max_bytes = max_bytes;
        *self.bytes_written.get_mut() = 0;
        *self.prepared.get_mut() = false;
        *self.deferred.get_mut() = false;
    }

    fn try_write(&self, bytes: usize) -> bool {
        let Some(max_bytes) = self.max_bytes else {
            return true;
        };
        let written = self
            .bytes_written
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |written| {
                Some(written.saturating_add(bytes))
            })
            .unwrap_or_else(|written| written);
        if written >= max_bytes {
            self.deferred.store(true, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn settled(&self) -> bool {
        !self.prepared.load(Ordering::Relaxed) && !self.deferred.load(Ordering::Relaxed)
    }
}

/// The set in [`Extract`] that holds every `prepare_render_asset` system.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct PrepareRenderAssets;

pub(crate) fn extract_render_asset_bytes_per_frame(
    budget: Extracted<Option<Res<RenderAssetBytesPerFrame>>>,
    mut limiter: ResMut<RenderAssetBytesPerFrameLimiter>,
) {
    let max_bytes = (*budget).as_ref().and_then(|budget| budget.max_bytes);
    limiter.reset(max_bytes);
}

pub(crate) fn register_render_asset_budget(app: &mut App) {
    if app.get_resource::<RenderAssetBytesPerFrame>().is_none() {
        app.insert_resource(RenderAssetBytesPerFrame::default());
    }
    app.render_mut()
        .insert_resource(RenderAssetBytesPerFrameLimiter::default());
    app.add_render_system(
        Extract,
        extract_render_asset_bytes_per_frame.before(PrepareRenderAssets),
    );
}

/// Whether the last frame of `app` prepared no render asset and deferred none; `true` without the render plugin.
pub fn render_assets_settled(app: &App) -> bool {
    let Some(limiter) = app
        .render()
        .get_resource::<RenderAssetBytesPerFrameLimiter>()
    else {
        return true;
    };
    limiter.settled()
}

pub(crate) fn prepare_render_asset<A: RenderAsset>(
    mut params: StaticSystemInput<<A as RenderAsset>::PreparationParams>,
    asset_store: Extracted<Res<AssetStore<A::SourceAsset>>>,
    mut render_assets: ResMut<RenderAssets<A>>,
    limiter: Option<Res<RenderAssetBytesPerFrameLimiter>>,
) {
    let limiter = limiter.as_deref();
    for (asset_id, asset) in asset_store.into_iter() {
        if render_assets.contains(asset_id) {
            continue;
        }

        if let (Some(limiter), Some(bytes)) = (limiter, A::byte_len(asset)) {
            if !limiter.try_write(bytes) {
                continue;
            }
        }

        let prepared_asset = A::prepare_asset(asset, &mut params);

        if let Ok(prepared_asset) = prepared_asset {
            if let Some(limiter) = limiter {
                limiter.prepared.store(true, Ordering::Relaxed);
            }
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

    pub fn remove(&mut self, id: &AssetId) -> Option<A> {
        self.0.remove(id)
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
    fn build(&self, app: &mut App) {
        app.render_mut().insert_resource(RenderAssets::<A>::new());
        app.add_render_system(
            Extract,
            prepare_render_asset::<A>.in_set(PrepareRenderAssets),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Payload(Option<usize>);

    impl Asset for Payload {
        fn name() -> &'static str {
            "Payload"
        }
    }

    struct Uploaded;

    impl RenderAsset for Uploaded {
        type SourceAsset = Payload;
        type PreparationParams = ();

        fn prepare_asset(
            _source_asset: &Self::SourceAsset,
            _params: &mut SystemInputData<Self::PreparationParams>,
        ) -> Result<Self, AssetPreparationError> {
            Ok(Uploaded)
        }

        fn byte_len(source: &Self::SourceAsset) -> Option<usize> {
            source.0
        }
    }

    fn app(max_bytes: Option<usize>, sizes: &[Option<usize>]) -> App {
        let mut app = App::new();
        app.set_extract_fn(concerto_app::extractor::extract);
        register_render_asset_budget(&mut app);
        app.register_plugin(RenderAssetPlugin::<Uploaded>::new());
        let mut store = AssetStore::<Payload>::new();
        for &size in sizes {
            store.insert(AssetId::new(), Payload(size));
        }
        app.insert_resource(store)
            .insert_resource(RenderAssetBytesPerFrame { max_bytes });
        app.finish_plugin_build();
        app
    }

    fn prepared(app: &App) -> usize {
        app.render()
            .get_resource::<RenderAssets<Uploaded>>()
            .unwrap()
            .0
            .len()
    }

    fn deferred(app: &App) -> bool {
        app.render()
            .get_resource::<RenderAssetBytesPerFrameLimiter>()
            .unwrap()
            .deferred
            .load(Ordering::Relaxed)
    }

    #[test]
    fn a_budget_lets_through_the_asset_that_crosses_it_and_defers_the_rest() {
        let mut app = app(Some(10), &[Some(4); 4]);

        app.update();
        assert_eq!(prepared(&app), 3);
        assert!(deferred(&app));

        app.update();
        assert_eq!(prepared(&app), 4);
        assert!(!deferred(&app));
    }

    #[test]
    fn an_asset_larger_than_the_budget_is_still_prepared() {
        let mut app = app(Some(10), &[Some(100)]);

        app.update();

        assert_eq!(prepared(&app), 1);
    }

    #[test]
    fn without_a_budget_every_asset_is_prepared_in_one_frame() {
        let mut app = app(None, &[Some(4); 4]);

        app.update();

        assert_eq!(prepared(&app), 4);
        assert!(!deferred(&app));
    }

    #[test]
    fn an_asset_without_a_byte_length_is_never_deferred() {
        let mut app = app(Some(10), &[Some(100), Some(100), None, None]);

        app.update();

        assert_eq!(prepared(&app), 3);
    }

    #[test]
    fn render_assets_settle_after_a_frame_that_prepared_and_deferred_nothing() {
        let mut app = app(Some(10), &[Some(4); 4]);

        app.update();
        assert!(!render_assets_settled(&app));
        app.update();
        assert!(!render_assets_settled(&app));
        app.update();
        assert!(render_assets_settled(&app));
    }

    #[test]
    fn an_app_without_a_limiter_counts_as_settled() {
        assert!(render_assets_settled(&App::new()));
    }
}

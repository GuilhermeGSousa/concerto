use concerto_app::{App, plugins::Plugin};
use concerto_foundation::assets::handle::AssetHandle;

use crate::asset::Ragdoll;

/// Registers the ragdoll asset so it loads, shows in the Asset Browser and opens in the inspector's form.
pub struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.register_inspected_asset::<Ragdoll>();
        app.register_type::<AssetHandle<Ragdoll>>();
    }
}

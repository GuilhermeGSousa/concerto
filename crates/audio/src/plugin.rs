use concerto_app::{App, Plugin};

use crate::AudioClip;

/// Registers the `AudioClip` asset; needs an `AssetServer`.
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset::<AudioClip>();
    }
}

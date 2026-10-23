use std::marker::PhantomData;

use concerto_ecs::command::EntityCommandQueue;
use concerto_foundation::assets::{Asset, AssetId, handle::AssetHandle};
use concerto_ui::node::UINode;

use crate::inspector::PropertyEditor;

pub struct AssetPicker<T: Asset>(PhantomData<T>);

impl<T: Asset> Default for AssetPicker<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T: Asset> PropertyEditor<AssetHandle<T>> for AssetPicker<T> {
    type Snapshot = AssetId;

    type Edit = AssetHandle<T>;

    fn snapshot(&self, value: &AssetHandle<T>) -> Self::Snapshot {
        value.id()
    }

    fn build(
        &self,
        row: &mut EntityCommandQueue,
        snapshot: &Self::Snapshot,
        theme: &concerto_ui::theme::UITheme,
    ) {
        row.add_child((UINode::default(), theme.text("Some Asset")));
    }

    fn apply(
        &self,
        value: &mut AssetHandle<T>,
        edit: &Self::Edit,
    ) -> Result<(), crate::inspector::EditError> {
        Ok(())
    }
}

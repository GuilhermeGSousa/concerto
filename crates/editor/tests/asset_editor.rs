#[path = "../examples/custom_asset.rs"]
mod custom_asset;

#[test]
fn downstream_custom_asset_editor_smoke_test() {
    custom_asset::smoke_test().unwrap();
}

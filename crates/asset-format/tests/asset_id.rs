use concerto_asset_format::AssetId;

#[test]
fn the_nil_id_is_nil_and_a_new_one_is_not() {
    assert!(AssetId::nil().is_nil());
    assert!(!AssetId::new().is_nil());
}

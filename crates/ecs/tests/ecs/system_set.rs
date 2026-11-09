use concerto_ecs::SystemSet;

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
enum TestSet {
    First,
    Second,
}

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
struct UnitSet;

#[test]
fn interning_the_same_variant_twice_yields_equal_handles() {
    assert_eq!(TestSet::First.intern(), TestSet::First.intern());
}

#[test]
fn different_variants_intern_differently() {
    assert_ne!(TestSet::First.intern(), TestSet::Second.intern());
}

#[test]
fn different_types_with_the_same_shape_intern_differently() {
    let a: Box<dyn SystemSet> = Box::new(UnitSet);
    let b: Box<dyn SystemSet> = Box::new(TestSet::First);
    assert!(a.as_ref() != b.as_ref());
}

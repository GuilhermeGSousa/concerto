use concerto_ecs::SystemSet;

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
enum TestSet {
    First,
    Second,
}

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
struct UnitSet;

#[test]
fn interned_sets_are_equal_only_for_the_same_type_and_value() {
    assert_eq!(TestSet::First.intern(), TestSet::First.intern());
    assert_ne!(TestSet::First.intern(), TestSet::Second.intern());
    let unit: Box<dyn SystemSet> = Box::new(UnitSet);
    let variant: Box<dyn SystemSet> = Box::new(TestSet::First);
    assert!(unit.as_ref() != variant.as_ref());
}

use std::{any::TypeId, sync::Mutex};

use concerto_reflect::{
    FromType, PropertyVisitor, Reflect, ReflectDefault, ReflectRef, TypeData, TypeRegistry,
};

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if record.level() == log::Level::Warn {
            WARNINGS.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

fn capture_warnings() {
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Warn);
}

/// Type data a test declares on its types, recording which type built it.
struct ReflectThing(&'static str);

impl TypeData for ReflectThing {}

impl<T: Reflect> FromType<T> for ReflectThing {
    fn from_type() -> Self {
        Self(std::any::type_name::<T>())
    }
}

#[derive(Reflect, Default, PartialEq, Debug)]
#[reflect(Thing, Default)]
struct Gain {
    amount: f32,
}

#[derive(Reflect)]
struct Plain {
    on: bool,
}

#[derive(Reflect, serde::Serialize, serde::Deserialize, Default)]
#[reflect(opaque, Thing)]
struct Handle(u32);

mod elsewhere {
    use concerto_reflect::Reflect;

    #[derive(Reflect)]
    pub struct Gain {
        pub other: f32,
    }
}

#[test]
fn a_registration_carries_its_names_and_the_type_data_its_derive_declared() {
    let mut registry = TypeRegistry::default();
    registry.register::<Gain>();

    let registration = registry.get(TypeId::of::<Gain>()).unwrap();
    assert_eq!(registration.type_path(), std::any::type_name::<Gain>());
    assert_eq!(registration.short_name(), "Gain");
    assert_eq!(
        registration.data::<ReflectThing>().unwrap().0,
        std::any::type_name::<Gain>()
    );
    assert!(registration.data::<ReflectDefault>().is_some());
    assert!(
        registry
            .get_type_data::<ReflectThing>(TypeId::of::<Gain>())
            .is_some()
    );
}

#[test]
fn a_type_declaring_nothing_has_no_type_data() {
    let mut registry = TypeRegistry::default();
    registry.register::<Plain>();

    let registration = registry.get(TypeId::of::<Plain>()).unwrap();
    assert!(registration.data::<ReflectThing>().is_none());
    assert!(registration.data::<ReflectDefault>().is_none());
}

#[test]
fn registering_twice_changes_nothing() {
    let mut registry = TypeRegistry::default();
    registry.register::<Gain>();
    let once = registry.iter().count();
    registry.register::<Gain>();

    assert_eq!(registry.iter().count(), once);
}

#[test]
fn reflect_default_builds_the_default_value() {
    let mut registry = TypeRegistry::default();
    registry.register::<Gain>();

    let value = registry
        .get_type_data::<ReflectDefault>(TypeId::of::<Gain>())
        .unwrap()
        .default();

    assert_eq!(
        (value.as_ref() as &dyn std::any::Any).downcast_ref::<Gain>(),
        Some(&Gain::default())
    );
}

#[test]
fn an_opaque_type_is_a_leaf_that_visits_no_fields_and_still_carries_its_type_data() {
    struct Count(usize);
    impl PropertyVisitor for Count {
        fn field(&mut self, _: &'static str, _: &dyn Reflect) {
            self.0 += 1;
        }
    }
    let mut count = Count(0);
    Handle(3).visit(&mut count);
    assert_eq!(count.0, 0);
    assert!(matches!(Handle(3).reflect_ref(), ReflectRef::Leaf(_)));

    let mut registry = TypeRegistry::default();
    registry.register::<Handle>();
    assert!(
        registry
            .get_type_data::<ReflectThing>(TypeId::of::<Handle>())
            .is_some()
    );
}

#[test]
fn a_name_resolves_by_full_path_first_then_by_short_name() {
    let mut registry = TypeRegistry::default();
    registry.register::<Gain>();

    let by_path = registry.get_by_name(std::any::type_name::<Gain>()).unwrap();
    let by_short = registry.get_by_name("Gain").unwrap();

    assert_eq!(by_path.type_id(), TypeId::of::<Gain>());
    assert_eq!(by_short.type_id(), TypeId::of::<Gain>());
    assert!(registry.get_by_name("Missing").is_none());
}

#[test]
fn two_types_sharing_a_short_name_resolve_to_the_last_registered_with_a_warning() {
    capture_warnings();
    let mut registry = TypeRegistry::default();
    registry.register::<Gain>();

    registry.register::<elsewhere::Gain>();

    assert_eq!(
        registry.get_by_name("Gain").unwrap().type_id(),
        TypeId::of::<elsewhere::Gain>()
    );
    assert_eq!(
        registry
            .get_by_name(std::any::type_name::<Gain>())
            .unwrap()
            .type_id(),
        TypeId::of::<Gain>()
    );
    let warnings = WARNINGS.lock().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains(std::any::type_name::<Gain>())
                && warning.contains(std::any::type_name::<elsewhere::Gain>())),
        "{warnings:?}"
    );
}

trait Special {}

#[derive(Reflect)]
struct Yes;

impl Special for Yes {}

struct ReflectSpecial;

impl TypeData for ReflectSpecial {}

impl<T: Reflect + Special> FromType<T> for ReflectSpecial {
    fn from_type() -> Self {
        Self
    }
}

#[derive(Reflect)]
#[reflect(Special)]
struct Wrapper<T: Reflect> {
    #[allow(dead_code)]
    inner: T,
}

impl Special for Wrapper<Yes> {}

#[test]
fn a_generic_type_registers_where_its_type_data_applies() {
    let mut registry = TypeRegistry::default();
    registry.register::<Wrapper<Yes>>();

    assert!(
        registry
            .get_type_data::<ReflectSpecial>(TypeId::of::<Wrapper<Yes>>())
            .is_some()
    );
    assert!(registry.get(TypeId::of::<Yes>()).is_some());
}

#[derive(Reflect, Default)]
struct Leafy {
    level: u8,
}

#[derive(Reflect, Default)]
struct Twig {
    length: u16,
}

#[derive(Reflect, Default)]
enum Shape {
    #[default]
    Empty,
    Round {
        radius: i64,
    },
    Pair(u32, Leafy),
}

#[derive(Reflect, Default)]
struct Tree {
    height: f32,
    crown: Option<Leafy>,
    twigs: Vec<Twig>,
    shape: Shape,
}

#[test]
fn registering_a_type_registers_the_types_its_fields_hold() {
    let mut registry = TypeRegistry::default();
    registry.register::<Tree>();

    for id in [
        TypeId::of::<f32>(),
        TypeId::of::<Option<Leafy>>(),
        TypeId::of::<Leafy>(),
        TypeId::of::<u8>(),
        TypeId::of::<Vec<Twig>>(),
        TypeId::of::<Twig>(),
        TypeId::of::<u16>(),
        TypeId::of::<Shape>(),
        TypeId::of::<i64>(),
        TypeId::of::<u32>(),
    ] {
        assert!(registry.get(id).is_some(), "{id:?} was not registered");
    }
}

#[derive(Reflect, Default)]
struct Node {
    children: Vec<Node>,
}

#[test]
fn a_type_that_holds_itself_registers_once() {
    let mut registry = TypeRegistry::default();
    registry.register::<Node>();

    assert!(registry.get(TypeId::of::<Node>()).is_some());
    assert!(registry.get(TypeId::of::<Vec<Node>>()).is_some());
    assert_eq!(registry.iter().count(), 2);
}

#[test]
fn an_opaque_type_registers_none_of_its_fields() {
    let mut registry = TypeRegistry::default();
    registry.register::<Handle>();

    assert_eq!(registry.iter().count(), 1);
    assert!(registry.get(TypeId::of::<u32>()).is_none());
}

use std::{any::TypeId, collections::HashSet, sync::Mutex};

use concerto_reflect::{
    FromType, PropertyVisitor, Reflect, ReflectDefault, ReflectEnumInfo, ReflectList,
    ReflectOption, ReflectRef, TypeData, TypeRegistry,
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
fn a_registration_carries_its_names_and_only_the_type_data_its_derive_declared() {
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

    registry.register::<Plain>();
    let registration = registry.get(TypeId::of::<Plain>()).unwrap();
    assert!(registration.data::<ReflectThing>().is_none());
    assert!(registration.data::<ReflectDefault>().is_none());
}

#[test]
fn a_registration_fetched_with_get_outlives_a_later_register() {
    let mut registry = TypeRegistry::default();
    registry.register::<Gain>();

    let registration = registry.get(TypeId::of::<Gain>()).unwrap();
    registry.register::<Plain>();
    let by_name = registry.get_by_name("Gain").unwrap();
    let listed = registry.iter().count();
    drop(registry);

    assert_eq!(registration.short_name(), "Gain");
    assert!(registration.data::<ReflectThing>().is_some());
    assert_eq!(by_name.type_id(), registration.type_id());
    assert!(listed > 2);
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
fn a_name_resolves_by_full_path_then_by_the_last_registered_short_name_with_a_warning() {
    capture_warnings();
    let mut registry = TypeRegistry::default();
    registry.register::<Gain>();
    assert_eq!(
        registry.get_by_name("Gain").unwrap().type_id(),
        TypeId::of::<Gain>()
    );
    assert!(registry.get_by_name("Missing").is_none());

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

#[derive(Reflect, Default)]
struct Node {
    children: Vec<Node>,
}

#[derive(Reflect, Default)]
enum Mood {
    #[default]
    Calm,
    Angry,
    Sleepy,
}

#[derive(Reflect, serde::Serialize, serde::Deserialize, Default)]
#[reflect(opaque)]
enum Sealed {
    #[default]
    Open,
    Shut,
}

struct Registers {
    name: &'static str,
    register: fn(&mut TypeRegistry),
    added: Vec<TypeId>,
}

#[test]
fn registering_a_type_adds_it_and_the_types_its_fields_hold_once() {
    let cases = [
        Registers {
            name: "a struct's fields, through options, lists and variants",
            register: |registry| registry.register::<Tree>(),
            added: vec![
                TypeId::of::<Tree>(),
                TypeId::of::<Option<Leafy>>(),
                TypeId::of::<Leafy>(),
                TypeId::of::<Vec<Twig>>(),
                TypeId::of::<Twig>(),
                TypeId::of::<Shape>(),
            ],
        },
        Registers {
            name: "a type registered twice",
            register: |registry| {
                registry.register::<Gain>();
                registry.register::<Gain>();
            },
            added: vec![TypeId::of::<Gain>()],
        },
        Registers {
            name: "a type that holds itself",
            register: |registry| registry.register::<Node>(),
            added: vec![TypeId::of::<Node>(), TypeId::of::<Vec<Node>>()],
        },
        Registers {
            name: "an opaque type",
            register: |registry| registry.register::<Handle>(),
            added: vec![TypeId::of::<Handle>()],
        },
        Registers {
            name: "a generic type",
            register: |registry| registry.register::<Wrapper<Yes>>(),
            added: vec![TypeId::of::<Wrapper<Yes>>(), TypeId::of::<Yes>()],
        },
    ];
    for case in cases {
        let mut registry = TypeRegistry::default();
        let built_in: HashSet<TypeId> = registry
            .iter()
            .map(|registration| registration.type_id())
            .collect();
        (case.register)(&mut registry);
        let added: HashSet<TypeId> = registry
            .iter()
            .map(|registration| registration.type_id())
            .filter(|id| !built_in.contains(id))
            .collect();
        assert_eq!(
            registry.iter().count(),
            built_in.len() + added.len(),
            "{}",
            case.name
        );
        assert_eq!(
            added,
            case.added.into_iter().collect::<HashSet<_>>(),
            "{}",
            case.name
        );
    }
}

struct Shaped {
    name: &'static str,
    id: TypeId,
    variants: Option<&'static [&'static str]>,
    option: bool,
    list: bool,
}

#[test]
fn an_enum_registers_its_variant_names_in_order_an_option_its_marker_and_a_vec_its_list() {
    let mut registry = TypeRegistry::default();
    registry.register::<Mood>();
    registry.register::<Tree>();
    registry.register::<Gain>();
    registry.register::<Sealed>();
    registry.register::<Option<f32>>();
    let cases = [
        Shaped {
            name: "a unit-only enum",
            id: TypeId::of::<Mood>(),
            variants: Some(&["Calm", "Angry", "Sleepy"]),
            option: false,
            list: false,
        },
        Shaped {
            name: "an enum with tuple and named variants",
            id: TypeId::of::<Shape>(),
            variants: Some(&["Empty", "Round", "Pair"]),
            option: false,
            list: false,
        },
        Shaped {
            name: "an option",
            id: TypeId::of::<Option<Leafy>>(),
            variants: Some(&["None", "Some"]),
            option: true,
            list: false,
        },
        Shaped {
            name: "an option of a leaf",
            id: TypeId::of::<Option<f32>>(),
            variants: Some(&["None", "Some"]),
            option: true,
            list: false,
        },
        Shaped {
            name: "a vec field",
            id: TypeId::of::<Vec<Twig>>(),
            variants: None,
            option: false,
            list: true,
        },
        Shaped {
            name: "a struct",
            id: TypeId::of::<Tree>(),
            variants: None,
            option: false,
            list: false,
        },
        Shaped {
            name: "a struct declaring type data",
            id: TypeId::of::<Gain>(),
            variants: None,
            option: false,
            list: false,
        },
        Shaped {
            name: "an opaque enum",
            id: TypeId::of::<Sealed>(),
            variants: None,
            option: false,
            list: false,
        },
    ];
    for case in cases {
        assert_eq!(
            registry
                .get_type_data::<ReflectEnumInfo>(case.id)
                .map(|info| info.variants),
            case.variants,
            "{}",
            case.name
        );
        assert_eq!(
            registry.get_type_data::<ReflectOption>(case.id).is_some(),
            case.option,
            "{}",
            case.name
        );
        assert_eq!(
            registry.get_type_data::<ReflectList>(case.id).is_some(),
            case.list,
            "{}",
            case.name
        );
    }
}

use std::{any::TypeId, collections::HashSet};

use concerto_reflect::{
    FromType, Reflect, ReflectDefault, ReflectEnumInfo, ReflectList, ReflectOption, TypeData,
    TypeRegistry,
};

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
            name: "a type that holds itself",
            register: |registry| registry.register::<Node>(),
            added: vec![TypeId::of::<Node>(), TypeId::of::<Vec<Node>>()],
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
    registry.register::<Tree>();
    let cases = [
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

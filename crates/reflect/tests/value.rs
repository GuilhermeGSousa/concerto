use std::any::Any;

use concerto_reflect::{
    LeafError, LeafHook, Reflect, ReflectedValue, apply_value, apply_value_with, to_value,
    to_value_with,
};
use glam::Vec3;

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
struct Inner {
    weight: f32,
    label: String,
}

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
struct Pair(f32, bool);

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
struct Marker;

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
enum Shape {
    #[default]
    Point,
    Sphere(f32),
    Cone {
        angle: f32,
        length: f32,
    },
}

#[derive(Reflect, Default, Debug, PartialEq, Clone)]
struct Everything {
    position: Vec3,
    inner: Inner,
    pair: Pair,
    marker: Marker,
    shape: Shape,
    other: Shape,
    maybe: Option<Inner>,
    never: Option<f32>,
    list: Vec<Inner>,
}

#[derive(Reflect, Default, Debug, PartialEq)]
struct Smaller {
    weight: f32,
}

#[derive(Reflect, Default, Debug, PartialEq)]
struct Larger {
    weight: f32,
    added: f32,
}

fn everything() -> Everything {
    Everything {
        position: Vec3::new(1.0, 2.0, 3.0),
        inner: Inner {
            weight: 0.5,
            label: "inner".into(),
        },
        pair: Pair(4.0, true),
        marker: Marker,
        shape: Shape::Sphere(2.0),
        other: Shape::Cone {
            angle: 0.25,
            length: 8.0,
        },
        maybe: Some(Inner {
            weight: 1.5,
            label: "maybe".into(),
        }),
        never: None,
        list: vec![
            Inner {
                weight: 1.0,
                label: "a".into(),
            },
            Inner {
                weight: 2.0,
                label: "b".into(),
            },
        ],
    }
}

fn round_trip<T: Reflect + Default>(value: &T) -> T {
    let stored = to_value(value).unwrap();
    let mut loaded = T::default();
    assert!(apply_value(&mut loaded, &stored).is_empty());
    loaded
}

#[test]
fn every_kind_survives_a_round_trip() {
    let value = everything();
    assert_eq!(round_trip(&value), value);
}

#[test]
fn the_tree_survives_bincode() {
    let stored = to_value(&everything()).unwrap();
    let bytes = bincode::serialize(&stored).unwrap();
    let decoded: ReflectedValue = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded, stored);
}

#[test]
fn a_field_the_tree_lacks_keeps_its_default() {
    let stored = to_value(&Smaller { weight: 3.0 }).unwrap();
    let mut loaded = Larger {
        weight: 0.0,
        added: 7.0,
    };
    assert!(apply_value(&mut loaded, &stored).is_empty());
    assert_eq!(
        loaded,
        Larger {
            weight: 3.0,
            added: 7.0
        }
    );
}

#[test]
fn a_stored_field_the_type_lacks_is_ignored_with_a_warning_naming_its_path() {
    let stored = to_value(&Larger {
        weight: 3.0,
        added: 1.0,
    })
    .unwrap();
    let mut loaded = Smaller::default();
    let warnings = apply_value(&mut loaded, &stored);
    assert_eq!(loaded, Smaller { weight: 3.0 });
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].path, "added");
}

#[test]
fn a_node_of_the_wrong_kind_leaves_the_field_unchanged_with_a_warning() {
    let stored = ReflectedValue::Struct(vec![
        ("weight".into(), ReflectedValue::List(Vec::new())),
        ("label".into(), to_value(&"kept".to_string()).unwrap()),
    ]);
    let mut loaded = Inner {
        weight: 9.0,
        label: String::new(),
    };
    let warnings = apply_value(&mut loaded, &stored);
    assert_eq!(
        loaded,
        Inner {
            weight: 9.0,
            label: "kept".into()
        }
    );
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].path, "weight");
}

#[test]
fn a_leaf_that_does_not_decode_leaves_the_field_unchanged_with_a_warning() {
    let stored = ReflectedValue::Struct(vec![("weight".into(), ReflectedValue::Leaf(vec![1]))]);
    let mut loaded = Smaller { weight: 2.0 };
    let warnings = apply_value(&mut loaded, &stored);
    assert_eq!(loaded, Smaller { weight: 2.0 });
    assert_eq!(warnings[0].path, "weight");
}

#[test]
fn an_unknown_variant_leaves_the_enum_unchanged_with_a_warning() {
    let stored = ReflectedValue::Variant {
        name: "Cube".into(),
        fields: Vec::new(),
    };
    let mut loaded = Shape::Sphere(1.0);
    let warnings = apply_value(&mut loaded, &stored);
    assert_eq!(loaded, Shape::Sphere(1.0));
    assert_eq!(warnings.len(), 1);
}

#[test]
fn a_list_grows_and_shrinks_to_the_stored_length() {
    let stored = to_value(&vec![1.0_f32, 2.0, 3.0]).unwrap();
    let mut longer = vec![9.0_f32; 5];
    assert!(apply_value(&mut longer, &stored).is_empty());
    assert_eq!(longer, [1.0, 2.0, 3.0]);

    let mut shorter = vec![9.0_f32];
    assert!(apply_value(&mut shorter, &stored).is_empty());
    assert_eq!(shorter, [1.0, 2.0, 3.0]);
}

#[test]
fn warnings_name_nested_paths_through_lists_and_variants() {
    let stored = ReflectedValue::Struct(vec![(
        "list".into(),
        ReflectedValue::List(vec![ReflectedValue::Struct(vec![(
            "gone".into(),
            to_value(&1.0_f32).unwrap(),
        )])]),
    )]);
    let mut loaded = Everything::default();
    let warnings = apply_value(&mut loaded, &stored);
    assert_eq!(warnings[0].path, "list[0].gone");
}

struct Doubling;

impl LeafHook for Doubling {
    fn encode(&mut self, leaf: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        let value = (leaf as &dyn Any).downcast_ref::<f32>()?;
        Some(Ok(bincode::serialize(&(value * 2.0)).unwrap()))
    }

    fn decode(&mut self, leaf: &mut dyn Reflect, bytes: &[u8]) -> Option<Result<(), LeafError>> {
        let value = (leaf as &mut dyn Any).downcast_mut::<f32>()?;
        *value = bincode::deserialize::<f32>(bytes).unwrap() * 2.0;
        Some(Ok(()))
    }
}

#[test]
fn a_hook_sees_each_leaf_first_and_others_use_their_own_encoding() {
    let value = Inner {
        weight: 1.0,
        label: "same".into(),
    };
    let stored = to_value_with(&value, &mut Doubling).unwrap();
    let mut loaded = Inner::default();
    assert!(apply_value_with(&mut loaded, &stored, &mut Doubling).is_empty());
    assert_eq!(
        loaded,
        Inner {
            weight: 4.0,
            label: "same".into()
        }
    );
}

struct Refusing;

impl LeafHook for Refusing {
    fn encode(&mut self, _: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        Some(Err(LeafError("refused".into())))
    }

    fn decode(&mut self, _: &mut dyn Reflect, _: &[u8]) -> Option<Result<(), LeafError>> {
        None
    }
}

#[test]
fn a_failed_encode_fails_the_whole_value_and_names_the_path() {
    let error = to_value_with(&everything(), &mut Refusing).unwrap_err();
    assert_eq!(error.path, "position");
}

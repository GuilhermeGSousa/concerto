use concerto_reflect::{Reflect, ReflectedValue, apply_value, to_value};
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

struct Mismatch {
    name: &'static str,
    stored: ReflectedValue,
    expected: Everything,
    warnings: &'static [&'static str],
}

fn field(name: &str, value: ReflectedValue) -> ReflectedValue {
    ReflectedValue::Struct(vec![(name.into(), value)])
}

#[test]
fn a_stored_tree_writes_what_matches_and_warns_at_the_path_of_what_does_not() {
    let label = |text: &str| to_value(&text.to_string()).unwrap();
    let cases = [
        Mismatch {
            name: "a field the tree lacks keeps its value",
            stored: field("pair", to_value(&Pair(9.0, false)).unwrap()),
            expected: Everything {
                pair: Pair(9.0, false),
                ..everything()
            },
            warnings: &[],
        },
        Mismatch {
            name: "a stored field the type lacks is ignored",
            stored: field("added", to_value(&1.0_f32).unwrap()),
            expected: everything(),
            warnings: &["added"],
        },
        Mismatch {
            name: "a node of the wrong kind leaves the field unchanged",
            stored: field(
                "inner",
                ReflectedValue::Struct(vec![
                    ("weight".into(), ReflectedValue::List(Vec::new())),
                    ("label".into(), label("kept")),
                ]),
            ),
            expected: Everything {
                inner: Inner {
                    weight: 0.5,
                    label: "kept".into(),
                },
                ..everything()
            },
            warnings: &["inner.weight"],
        },
        Mismatch {
            name: "an unknown variant leaves the enum unchanged",
            stored: field(
                "shape",
                ReflectedValue::Variant {
                    name: "Cube".into(),
                    fields: Vec::new(),
                },
            ),
            expected: everything(),
            warnings: &["shape"],
        },
        Mismatch {
            name: "a list shrinks to the stored length and paths name its elements",
            stored: field(
                "list",
                ReflectedValue::List(vec![field("gone", to_value(&1.0_f32).unwrap())]),
            ),
            expected: Everything {
                list: everything().list[..1].to_vec(),
                ..everything()
            },
            warnings: &["list[0].gone"],
        },
        Mismatch {
            name: "a list grows to the stored length from defaults",
            stored: field(
                "list",
                ReflectedValue::List(vec![
                    ReflectedValue::Struct(Vec::new()),
                    ReflectedValue::Struct(Vec::new()),
                    field("label", label("c")),
                ]),
            ),
            expected: Everything {
                list: [
                    everything().list,
                    vec![Inner {
                        weight: 0.0,
                        label: "c".into(),
                    }],
                ]
                .concat(),
                ..everything()
            },
            warnings: &[],
        },
    ];
    for case in cases {
        let mut loaded = everything();
        let warnings = apply_value(&mut loaded, &case.stored);
        assert_eq!(loaded, case.expected, "{}", case.name);
        assert_eq!(
            warnings
                .iter()
                .map(|warning| warning.path.as_str())
                .collect::<Vec<_>>(),
            case.warnings,
            "{}",
            case.name
        );
    }
}

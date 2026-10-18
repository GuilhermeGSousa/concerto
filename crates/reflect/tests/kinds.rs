use std::any::Any;

use concerto_reflect::{PropertyVisitor, Reflect, ReflectMut, ReflectRef};
use glam::Vec3;

#[derive(Reflect, Default, Debug, PartialEq)]
struct Named {
    weight: f32,
    label: String,
}

#[derive(Reflect, Default, Debug, PartialEq)]
struct Pair(f32, bool);

#[derive(Reflect, Default, Debug, PartialEq)]
struct Marker;

#[derive(Reflect, Default, Debug, PartialEq)]
enum Shape {
    #[default]
    Point,
    Sphere(f32),
    Cone {
        angle: f32,
        length: f32,
    },
}

#[derive(Reflect, Default, Debug, PartialEq)]
struct Generic<T: Reflect + Default> {
    value: T,
}

fn names(value: &dyn Reflect) -> Vec<&'static str> {
    struct Names(Vec<&'static str>);
    impl PropertyVisitor for Names {
        fn field(&mut self, name: &'static str, _: &dyn Reflect) {
            self.0.push(name);
        }
    }
    let mut names = Names(Vec::new());
    value.visit(&mut names);
    names.0
}

#[test]
fn named_tuple_and_unit_structs_are_structs_with_named_fields() {
    let named = Named {
        weight: 2.0,
        label: "a".into(),
    };
    let ReflectRef::Struct(fields) = named.reflect_ref() else {
        panic!("a named struct is a Struct");
    };
    assert_eq!(fields.field_len(), 2);
    assert_eq!(fields.name_at(1), Some("label"));
    assert_eq!(
        (fields.field("weight").unwrap() as &dyn Any).downcast_ref::<f32>(),
        Some(&2.0)
    );

    let pair = Pair(1.0, true);
    let ReflectRef::Struct(fields) = pair.reflect_ref() else {
        panic!("a tuple struct is a Struct");
    };
    assert_eq!(
        (0..fields.field_len())
            .map(|index| fields.name_at(index).unwrap())
            .collect::<Vec<_>>(),
        ["0", "1"]
    );

    let ReflectRef::Struct(fields) = Marker.reflect_ref() else {
        panic!("a unit struct is a Struct");
    };
    assert_eq!(fields.field_len(), 0);
}

#[test]
fn an_enum_names_its_variant_and_exposes_that_variants_fields() {
    let shape = Shape::Cone {
        angle: 0.5,
        length: 2.0,
    };
    let ReflectRef::Enum(variant) = shape.reflect_ref() else {
        panic!("an enum is an Enum");
    };
    assert_eq!(variant.variant_name(), "Cone");
    assert_eq!(variant.field_len(), 2);
    assert_eq!(variant.name_at(0), Some("angle"));

    let sphere = Shape::Sphere(3.0);
    let ReflectRef::Enum(variant) = sphere.reflect_ref() else {
        panic!("an enum is an Enum");
    };
    assert_eq!(variant.variant_name(), "Sphere");
    assert_eq!(variant.name_at(0), Some("0"));
}

#[test]
fn setting_a_variant_builds_it_from_defaults() {
    let mut shape = Shape::Point;
    let ReflectMut::Enum(variant) = shape.reflect_mut() else {
        panic!("an enum is an Enum");
    };
    assert!(variant.set_variant("Cone"));
    assert!(!variant.set_variant("Cube"));
    assert_eq!(
        shape,
        Shape::Cone {
            angle: 0.0,
            length: 0.0
        }
    );
}

#[test]
fn an_option_is_an_enum_of_none_and_some() {
    let mut value: Option<f32> = None;
    let ReflectMut::Enum(variant) = value.reflect_mut() else {
        panic!("an Option is an Enum");
    };
    assert_eq!(variant.variant_name(), "None");
    assert!(variant.set_variant("Some"));
    assert_eq!(variant.name_at(0), Some("0"));
    assert_eq!(value, Some(0.0));
}

#[test]
fn a_vec_is_a_list_that_grows_with_defaults_and_shrinks() {
    let mut list = vec![1.0_f32, 2.0];
    let ReflectMut::List(elements) = list.reflect_mut() else {
        panic!("a Vec is a List");
    };
    elements.push_default();
    assert_eq!(elements.len(), 3);
    elements.truncate(1);
    assert_eq!(list, [1.0]);
}

#[test]
fn a_leaf_encodes_and_decodes_itself() {
    let source = Vec3::new(1.0, 2.0, 3.0);
    let ReflectRef::Leaf(leaf) = source.reflect_ref() else {
        panic!("Vec3 is a Leaf");
    };
    let bytes = leaf.encode().unwrap();

    let mut target = Vec3::ZERO;
    let ReflectMut::Leaf(leaf) = target.reflect_mut() else {
        panic!("Vec3 is a Leaf");
    };
    leaf.decode(&bytes).unwrap();
    assert!(leaf.decode(&[1]).is_err());
    assert_eq!(target, source);
}

#[test]
fn visiting_sees_struct_fields_and_the_active_variants_fields_only() {
    assert_eq!(names(&Named::default()), ["weight", "label"]);
    assert_eq!(names(&Pair::default()), ["0", "1"]);
    assert_eq!(
        names(&Shape::Cone {
            angle: 0.0,
            length: 0.0
        }),
        ["angle", "length"]
    );
    assert!(names(&Shape::Point).is_empty());
    assert!(names(&vec![1.0_f32]).is_empty());
    assert!(names(&1.0_f32).is_empty());
}

#[test]
fn a_generic_struct_reflects_its_parameter() {
    let value = Generic { value: 4_u32 };
    let ReflectRef::Struct(fields) = value.reflect_ref() else {
        panic!("a generic struct is a Struct");
    };
    assert_eq!(
        (fields.field("value").unwrap() as &dyn Any).downcast_ref::<u32>(),
        Some(&4)
    );
}

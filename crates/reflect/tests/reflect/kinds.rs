use concerto_reflect::{Reflect, ReflectMut};

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

    let mut value: Option<f32> = None;
    let ReflectMut::Enum(variant) = value.reflect_mut() else {
        panic!("an Option is an Enum");
    };
    assert!(variant.set_variant("Some"));
    assert_eq!(value, Some(0.0));
}

#[test]
fn a_vec_is_a_list_that_grows_with_defaults_shrinks_and_removes_in_range() {
    let mut list = vec![1.0_f32, 2.0, 3.0];
    let ReflectMut::List(elements) = list.reflect_mut() else {
        panic!("a Vec is a List");
    };
    assert!(elements.remove(1));
    assert!(!elements.remove(2));
    elements.push_default();
    assert_eq!(elements.len(), 3);
    elements.truncate(2);
    assert_eq!(list, [1.0, 3.0]);
}

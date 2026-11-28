use std::{any::TypeId, thread};

use concerto_name::Name;
use concerto_reflect::{to_json, ReflectDefault, ReflectJson, ReflectedValue, TypeRegistry};
use proptest::prelude::*;
use serde_json::json;

proptest! {
    #[test]
    fn two_names_are_equal_exactly_when_their_texts_are(first in "[aAé]{0,3}", second in "[aAé]{0,3}") {
        prop_assert_eq!(Name::new(&first) == Name::new(&second), first == second);
        prop_assert_eq!(Name::new(&first), Name::from(first.clone()));
        prop_assert_eq!(Name::new(&first).as_str(), first.as_str());
    }
}

#[test]
fn names_interned_on_several_threads_are_equal_per_text() {
    let texts: Vec<String> = (0..64)
        .map(|index| format!("thread-{}", index % 8))
        .collect();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let texts = texts.clone();
            thread::spawn(move || texts.iter().map(|text| Name::new(text)).collect::<Vec<_>>())
        })
        .collect();
    let results: Vec<Vec<Name>> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();

    for names in &results {
        for (name, text) in names.iter().zip(&texts) {
            assert_eq!(*name, Name::new(text));
            assert_eq!(name.as_str(), text);
        }
    }
}

#[test]
fn a_name_is_stored_as_its_text_in_bincode_reflection_and_json() {
    let bytes = bincode::serialize(&Name::new("Hips")).unwrap();
    assert_eq!(bytes, bincode::serialize(&String::from("Hips")).unwrap());
    assert_eq!(
        bincode::deserialize::<Name>(&bytes).unwrap(),
        Name::new("Hips")
    );

    let stored = ReflectedValue::encode(&Name::new("Head")).unwrap();
    assert_eq!(
        stored,
        ReflectedValue::encode(&String::from("Head")).unwrap()
    );
    let mut read = Name::default();
    assert!(stored.apply(&mut read).is_empty());
    assert_eq!(read, Name::new("Head"));

    let mut registry = TypeRegistry::default();
    registry.register::<Name>();
    let id = TypeId::of::<Name>();
    let registration = registry.get(id).unwrap();
    assert!(registration.data::<ReflectDefault>().is_some());
    assert!(registration.data::<ReflectJson>().is_some());
    assert_eq!(to_json(&Name::new("Neck"), &registry), json!("Neck"));
    assert_eq!(
        ReflectedValue::from_json(&json!("Neck"), id, &registry).unwrap(),
        ReflectedValue::encode(&Name::new("Neck")).unwrap()
    );
    assert!(ReflectedValue::from_json(&json!(3), id, &registry).is_err());
}

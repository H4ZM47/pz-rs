use super::*;

fn info(id: i16, name: &str) -> AttributeTypeInfo {
    AttributeTypeInfo {
        id,
        name: name.into(),
        kind: AttributeKind::Float,
        initial: Some(InitialValue::Float(1.0)),
        hidden_ui: false,
    }
}

fn unbounded() -> Lookup<NumericAttributeValidation> {
    Lookup::Found(NumericAttributeValidation {
        requires_validation: false,
        bounds: None,
    })
}

#[test]
fn name_id_and_cloned_registry_keep_actual_type_identity_under_a_type_guard() {
    let mut registry = AttributeTypes::default();
    let actual = registry.insert_with_validation(info(3, "Authored"), false, unbounded());
    let cloned = registry.clone();
    let mut guard = actual.borrow_mut();
    assert!(Rc::ptr_eq(&actual, &registry.get_ref(3).unwrap()));
    assert!(Rc::ptr_eq(
        &actual,
        &cloned.by_name_ref("AUTHORED").unwrap()
    ));
    guard.numeric_validation = Lookup::Found(NumericAttributeValidation {
        requires_validation: true,
        bounds: Some(NumericAttributeBounds {
            min: InitialValue::Float(0.0),
            max: InitialValue::Float(1.0),
        }),
    });
    drop(guard);
    assert_eq!(
        cloned.get_ref(3).unwrap().borrow().numeric_validation,
        actual.borrow().numeric_validation
    );
    let snapshot = (*registry.get(3).unwrap()).clone();
    assert_eq!(snapshot.name, "Authored");
}

#[test]
fn explicit_metadata_replacement_does_not_retarget_a_retained_native_type() {
    let mut registry = AttributeTypes::default();
    let original = registry.insert_with_validation(info(7, "Original"), false, unbounded());
    registry.insert(info(7, "Replacement"));
    let replacement = registry.get_ref(7).unwrap();
    assert!(!Rc::ptr_eq(&original, &replacement));
    assert_eq!(original.borrow().name, "Original");
    assert_eq!(replacement.borrow().numeric_validation, Lookup::Unknown);
    assert!(registry.by_name_ref("Original").is_none());
    assert!(Rc::ptr_eq(
        &replacement,
        &registry.by_name_ref("replacement").unwrap()
    ));
}

#[test]
fn static_bounds_effects_enable_validation_and_keep_the_first_actual_vars() {
    let mut registry = AttributeTypes::default();
    let actual = registry.insert_with_validation(info(1, "Bounded"), false, unbounded());
    let fields = HashMap::from([("Bounded".into(), actual.clone())]);
    let args = |max| {
        vec![
            Value::Static {
                class: ATTRIBUTE.into(),
                name: "Bounded".into(),
            },
            Value::Call {
                class: "java/lang/Float".into(),
                name: "valueOf".into(),
                descriptor: "(F)Ljava/lang/Float;".into(),
                args: vec![Value::Float(0.0)],
            },
            Value::Float(max),
        ]
    };
    apply_static_attribute_bounds(&fields, &args(1.0)).unwrap();
    apply_static_attribute_bounds(&fields, &args(2.0)).unwrap();
    assert_eq!(
        actual.borrow().numeric_validation,
        Lookup::Found(NumericAttributeValidation {
            requires_validation: true,
            bounds: Some(NumericAttributeBounds {
                min: InitialValue::Float(0.0),
                max: InitialValue::Float(1.0)
            }),
        })
    );
}

#[test]
#[ignore = "requires PZ_GAME_DIR; reads installed class data without loading Java"]
fn installed_attribute_bounds_come_from_decoded_static_effect_order() {
    let game = std::path::PathBuf::from(std::env::var_os("PZ_GAME_DIR").expect("PZ_GAME_DIR"));
    let jar = Jar::open(game.join("projectzomboid.jar")).unwrap();
    let registry = AttributeTypes::from_jar(&jar).unwrap();
    for (name, max) in [
        ("Sharpness", InitialValue::Float(1.0)),
        ("HeadCondition", InitialValue::Int(1000)),
    ] {
        let actual = registry.by_name_ref(name).unwrap();
        let actual = actual.borrow();
        assert!(!actual.read_only);
        let Lookup::Found(validation) = &actual.numeric_validation else {
            panic!("native validation was not decoded")
        };
        assert!(validation.requires_validation);
        assert_eq!(validation.bounds.as_ref().unwrap().max, max);
    }
    assert_eq!(
        registry
            .by_name_ref("OriginX")
            .unwrap()
            .borrow()
            .numeric_validation,
        unbounded()
    );
}

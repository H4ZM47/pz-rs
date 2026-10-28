use super::*;
use crate::runtime_pool::RuntimeComponentFactory;
use crate::tables::{
    AttributeTypeInfo, AttributeTypes, NumericAttributeBounds, NumericAttributeValidation,
};
use crate::{BasicContext, Component, RuntimeComponent};
use pz_scripts::objects::entity::component_script::ComponentScript;

fn reset_pools() {
    POOLS.with(|pools| *pools.borrow_mut() = AttributePools::default());
}
fn context() -> BasicContext {
    let mut attributes = AttributeTypes::default();
    for (id, name, kind, initial, max) in [
        (
            0,
            "Sharpness",
            AttributeKind::Float,
            InitialValue::Float(1.0),
            InitialValue::Float(1.0),
        ),
        (
            1,
            "HeadCondition",
            AttributeKind::Int,
            InitialValue::Int(10),
            InitialValue::Int(1000),
        ),
        (
            2,
            "HeadConditionMax",
            AttributeKind::Int,
            InitialValue::Int(10),
            InitialValue::Int(1000),
        ),
    ] {
        let min = if kind == AttributeKind::Float {
            InitialValue::Float(0.0)
        } else {
            InitialValue::Int(0)
        };
        attributes.insert_with_validation(
            AttributeTypeInfo {
                id,
                name: name.into(),
                kind,
                initial: Some(initial),
                hidden_ui: true,
            },
            false,
            Lookup::Found(NumericAttributeValidation {
                requires_validation: true,
                bounds: Some(NumericAttributeBounds { min, max }),
            }),
        );
    }
    BasicContext {
        attributes,
        ..Default::default()
    }
}
fn type_ref(ctx: &BasicContext, id: i16) -> AttributeTypeRef {
    match ctx.attribute_type_ref(id) {
        Lookup::Found(reference) => reference,
        _ => panic!("authored actual type missing"),
    }
}
fn script(values: &str) -> pz_scripts::objects::entity::components::attributes::AttributesScript {
    let root = pz_scripts::parser::parse(&format!("component Attributes {{ {values} }}"));
    let mut script = pz_scripts::objects::entity::components::attributes::AttributesScript::new();
    script
        .load(&pz_scripts::ScriptManager::new(), &root.children[0])
        .unwrap();
    script
}
fn component() -> ComponentRef {
    RuntimeComponent::new_unowned(Component::Attributes(Default::default())).unwrap()
}

#[test]
fn get_or_add_checks_actual_type_identity_before_reading_type_body() {
    reset_pools();
    let ctx = context();
    let same_id_ctx = context();
    let existing_type = type_ref(&ctx, 1);
    let other_type = type_ref(&same_id_ctx, 1);
    assert!(!Rc::ptr_eq(&existing_type, &other_type));
    let actual = component();
    let mut retired = RetiredComponents::new();
    let first = actual
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .get_or_add(&existing_type, &mut retired)
        .unwrap();
    let held_existing_type = existing_type.borrow_mut();
    let held_first = first.borrow_mut();
    {
        let body = actual.borrow();
        let attrs = body.attributes().unwrap();
        assert!(Rc::ptr_eq(
            attrs.get_ref_by_type(&existing_type).unwrap().unwrap(),
            &first
        ));
        assert!(attrs.get_ref_by_type(&other_type).unwrap().is_none());
    }
    drop(held_first);
    let found = actual
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .get_or_add(&existing_type, &mut retired)
        .unwrap();
    assert!(Rc::ptr_eq(&first, &found));

    let held_other_type = other_type.borrow_mut();
    assert!(matches!(
        actual
            .borrow_mut()
            .attributes_mut()
            .unwrap()
            .get_or_add(&other_type, &mut retired),
        Err(RuntimeError::BorrowedTransient("AttributeType"))
    ));
    assert_eq!(
        actual.borrow().attributes().unwrap().entry_slots().count(),
        1
    );
    drop(held_other_type);
    let second = actual
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .get_or_add(&other_type, &mut retired)
        .unwrap();
    assert!(!Rc::ptr_eq(&first, &second));
    let body = actual.borrow();
    let slots: Vec<_> = body.attributes().unwrap().entry_slots().collect();
    assert_eq!(slots.len(), 2);
    assert!(Rc::ptr_eq(slots[0].type_slot(), &existing_type));
    assert!(Rc::ptr_eq(slots[1].type_slot(), &other_type));
    assert!(Rc::ptr_eq(
        body.attributes()
            .unwrap()
            .get_ref_by_type(&existing_type)
            .unwrap()
            .unwrap(),
        &first
    ));
    assert!(Rc::ptr_eq(
        body.attributes()
            .unwrap()
            .get_ref_by_type(&other_type)
            .unwrap()
            .unwrap(),
        &second
    ));
    drop(slots);
    drop(body);
    drop(held_existing_type);
}

#[test]
fn native_clear_preserves_component_and_numeric_instance_fifo_with_type_null_value_retained() {
    reset_pools();
    let ctx = context();
    let one = type_ref(&ctx, 1);
    let two = type_ref(&ctx, 2);
    let mut factory = RuntimeComponentFactory::new();
    let actual = factory.obtain(ComponentType::Attributes).unwrap();
    let mut retired = RetiredComponents::new();
    {
        let mut body = actual.borrow_mut();
        let attrs = body.attributes_mut().unwrap();
        attrs
            .set_value(&one, AttributeValue::Int(20), &mut retired)
            .unwrap();
        attrs
            .set_value(&two, AttributeValue::Int(30), &mut retired)
            .unwrap();
    }
    let first = actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(1)
        .unwrap()
        .unwrap()
        .clone();
    let second = actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(2)
        .unwrap()
        .unwrap()
        .clone();
    factory.release_detached(&actual, &mut retired).unwrap();
    assert!(first.borrow().type_slot().is_none());
    assert_eq!(first.borrow().value(), &AttributeValue::Int(20));
    assert_eq!(second.borrow().value(), &AttributeValue::Int(30));
    assert_eq!(
        actual
            .borrow()
            .attributes()
            .unwrap()
            .max_attribute_id()
            .unwrap(),
        -1
    );
    let obtained = factory.obtain(ComponentType::Attributes).unwrap();
    assert!(Rc::ptr_eq(&obtained, &actual));
    let reused_first = obtained
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .get_or_add(&two, &mut retired)
        .unwrap();
    let reused_second = obtained
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .get_or_add(&one, &mut retired)
        .unwrap();
    assert!(Rc::ptr_eq(&first, &reused_first));
    assert!(Rc::ptr_eq(&second, &reused_second));
    assert_eq!(first.borrow().value(), &AttributeValue::Int(10));
    assert!(Rc::ptr_eq(first.borrow().type_slot().unwrap(), &two));
}

#[test]
fn actual_script_template_copies_distinct_instances_and_reuses_shared_type_definition() {
    reset_pools();
    let ctx = context();
    let script = script("Sharpness = 2.0, HeadCondition = 15,");
    let mut notices = Vec::new();
    let mut retired = RetiredComponents::new();
    let template = get_or_create_attributes_template(
        &script,
        &ctx,
        &|| panic!("successful parse must not query Core.debug"),
        &mut notices,
        &mut retired,
    )
    .unwrap()
    .unwrap();
    let actual = component();
    actual
        .borrow_mut()
        .read_attributes_template(Some(&template), "Parent", &mut notices, &mut retired)
        .unwrap();
    let source = template
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(0)
        .unwrap()
        .unwrap()
        .clone();
    let copied = actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(0)
        .unwrap()
        .unwrap()
        .clone();
    assert!(!Rc::ptr_eq(&source, &copied));
    assert!(Rc::ptr_eq(
        source.borrow().type_slot().unwrap(),
        copied.borrow().type_slot().unwrap()
    ));
    assert_eq!(copied.borrow().value(), &AttributeValue::Float(1.0));
    actual
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(
            &type_ref(&ctx, 0),
            AttributeValue::Float(0.25),
            &mut retired,
        )
        .unwrap();
    assert_eq!(source.borrow().value(), &AttributeValue::Float(1.0));
    assert_eq!(copied.borrow().value(), &AttributeValue::Float(0.25));
    let cached = get_or_create_attributes_template(
        &script,
        &ctx,
        &|| panic!("cached template must not query Core.debug"),
        &mut notices,
        &mut retired,
    )
    .unwrap()
    .unwrap();
    assert!(Rc::ptr_eq(&template, &cached));
    assert!(notices.is_empty());
}

#[test]
fn absent_template_logs_native_error_without_clearing_retained_receiver_state() {
    let ctx = context();
    let actual = component();
    let mut retired = RetiredComponents::new();
    actual
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&type_ref(&ctx, 1), AttributeValue::Int(3), &mut retired)
        .unwrap();
    let instance = actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(1)
        .unwrap()
        .unwrap()
        .clone();
    let mut notices = Vec::new();
    actual
        .borrow_mut()
        .read_attributes_template(None, "ActualParent", &mut notices, &mut retired)
        .unwrap();
    assert_eq!(
        notices,
        [RuntimeNativeNotice::GeneralError(
            "Unable to create AttributeContainer from script: ActualParent".into()
        )]
    );
    assert!(Rc::ptr_eq(
        &instance,
        actual
            .borrow()
            .attributes()
            .unwrap()
            .get_ref(1)
            .unwrap()
            .unwrap()
    ));
    assert_eq!(instance.borrow().value(), &AttributeValue::Int(3));
}

#[test]
fn malformed_template_retains_initial_values_and_queries_debug_only_in_failed_parse_phase() {
    let ctx = context();
    let script = script("Sharpness = ++1,");
    let mut retired = RetiredComponents::new();
    let mut notices = Vec::new();
    let calls = std::cell::Cell::new(0);
    let debug = || {
        calls.set(calls.get() + 1);
        let slot = script.template_slot().unwrap();
        let actual =
            Rc::downcast::<RefCell<RuntimeComponent>>(slot.as_ref().unwrap().clone()).unwrap();
        assert!(
            actual.try_borrow_mut().is_ok(),
            "debug query ends actual template guard"
        );
        Ok(false)
    };
    let template =
        get_or_create_attributes_template(&script, &ctx, &debug, &mut notices, &mut retired)
            .unwrap()
            .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(
        template
            .borrow()
            .attributes()
            .unwrap()
            .get_value(0)
            .unwrap(),
        Some(AttributeValue::Float(1.0))
    );
    assert!(
        matches!(&notices[0], RuntimeNativeNotice::GeneralException(message) if message.contains("NumberFormatException"))
    );
    assert_eq!(
        notices[1],
        RuntimeNativeNotice::GeneralError(
            "WARNING: Item - > Attribute 'Sharpness' could not be added.".into()
        )
    );
    let cached = get_or_create_attributes_template(
        &script,
        &ctx,
        &|| panic!("completed failure cache does not rebuild"),
        &mut notices,
        &mut retired,
    )
    .unwrap()
    .unwrap();
    assert!(Rc::ptr_eq(&template, &cached));
}

#[test]
fn debug_template_exception_nulls_actual_cache_and_marks_one_completed_creation_attempt() {
    let ctx = context();
    let script = script("HeadCondition = bad,");
    let mut retired = RetiredComponents::new();
    let mut notices = Vec::new();
    assert!(get_or_create_attributes_template(
        &script,
        &ctx,
        &|| Ok(true),
        &mut notices,
        &mut retired
    )
    .unwrap()
    .is_none());
    assert!(script.template_created());
    assert!(script.template_slot().unwrap().is_none());
    assert_eq!(notices.len(), 2);
    assert_eq!(
        notices[1],
        RuntimeNativeNotice::GeneralException(
            "java.lang.RuntimeException: Attribute 'HeadCondition' could not be added.".into()
        )
    );
    assert!(get_or_create_attributes_template(
        &script,
        &ctx,
        &|| panic!("caught creation failure is cached"),
        &mut notices,
        &mut retired
    )
    .unwrap()
    .is_none());
}

#[test]
fn native_load_inserts_actual_initial_instance_before_truncated_scalar_value() {
    let ctx = context();
    let actual = component();
    let mut retired = RetiredComponents::new();
    let mut reader = Reader::new(&[8, 0, 1, 1]);
    let error = actual
        .borrow_mut()
        .load_payload_in_place(&mut reader, 249, &ctx, &mut retired)
        .unwrap_err();
    assert!(matches!(error, RuntimeError::Codec(_)));
    assert_eq!(reader.pos(), 4);
    assert_eq!(
        actual.borrow().attributes().unwrap().get_value(1).unwrap(),
        Some(AttributeValue::Int(10))
    );
}

#[test]
fn compressed_native_save_patches_current_bit_before_a_later_nested_borrow_error() {
    let mut ctx = context();
    for id in 0..7 {
        ctx.attributes.insert_with_validation(
            AttributeTypeInfo {
                id,
                name: format!("Type{id}"),
                kind: AttributeKind::Int,
                initial: Some(InitialValue::Int(0)),
                hidden_ui: false,
            },
            false,
            Lookup::Found(NumericAttributeValidation {
                requires_validation: false,
                bounds: None,
            }),
        );
    }
    let actual = component();
    let mut retired = RetiredComponents::new();
    for id in [3, 1, 6, 0, 5, 2, 4] {
        actual
            .borrow_mut()
            .attributes_mut()
            .unwrap()
            .set_value(
                &type_ref(&ctx, id),
                AttributeValue::Int(100 + id as i32),
                &mut retired,
            )
            .unwrap();
    }
    let third = actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(6)
        .unwrap()
        .unwrap()
        .clone();
    let guard = third.borrow_mut();
    let mut writer = Writer::new();
    assert!(matches!(
        actual.borrow().write_payload(&mut writer),
        Err(RuntimeError::BorrowedTransient("AttributeInstance"))
    ));
    assert_eq!(&writer.buf[..2], &[1, 1]);
    assert_eq!(
        i64::from_be_bytes(writer.buf[2..10].try_into().unwrap()),
        (1 << 3) | (1 << 1) | (1 << 6)
    );
    assert_eq!(
        i32::from_be_bytes(writer.buf[10..14].try_into().unwrap()),
        103
    );
    assert_eq!(
        i32::from_be_bytes(writer.buf[14..18].try_into().unwrap()),
        101
    );
    assert_eq!(writer.buf.len(), 18);
    drop(guard);
}

#[test]
fn archive_copy_has_fresh_actual_instances_and_borrow_preflight_without_owner_alias() {
    let ctx = context();
    let actual = component();
    let mut retired = RetiredComponents::new();
    actual
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&type_ref(&ctx, 1), AttributeValue::Int(17), &mut retired)
        .unwrap();
    let original = actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(1)
        .unwrap()
        .unwrap()
        .clone();
    let guard = original.borrow_mut();
    assert!(matches!(
        actual.borrow().check_archive_copy(),
        Err(RuntimeError::BorrowedTransient("AttributeInstance"))
    ));
    drop(guard);
    let copied = actual.borrow().try_archive_copy_unowned().unwrap();
    let copy_instance = copied
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(1)
        .unwrap()
        .unwrap()
        .clone();
    assert!(!Rc::ptr_eq(&original, &copy_instance));
    assert_eq!(copy_instance.borrow().value(), &AttributeValue::Int(17));
    assert!(copied.borrow().owner_slot().is_none());
}

#[test]
fn archive_values_are_an_explicit_alternative_without_fabricated_live_instances() {
    let legacy = AttributeContainer {
        attributes: vec![Attribute {
            id: 1,
            value: AttributeValue::Int(9),
        }],
    };
    let actual = RuntimeComponent::new_unowned(Component::Attributes(legacy.clone())).unwrap();
    assert_eq!(
        actual.borrow().value_snapshot().unwrap(),
        Component::Attributes(legacy)
    );
    assert!(actual.borrow().attributes().unwrap().get_ref(1).is_err());
    assert!(actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref_by_type(&type_ref(&context(), 1))
        .is_err());
    let mut retired = RetiredComponents::new();
    actual
        .borrow_mut()
        .load_payload_in_place(&mut Reader::new(&[0]), 249, &context(), &mut retired)
        .unwrap();
    assert!(actual.borrow().attributes().unwrap().is_resolved());
    assert!(actual
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(1)
        .unwrap()
        .is_none());
}

#[test]
fn held_second_instance_clear_retry_never_releases_or_enqueues_the_first_twice() {
    reset_pools();
    let ctx = context();
    let one = type_ref(&ctx, 1);
    let two = type_ref(&ctx, 2);
    let mut attrs = RuntimeAttributes::default();
    let mut retired = RetiredComponents::new();
    let first = attrs.get_or_add(&one, &mut retired).unwrap();
    let second = attrs.get_or_add(&two, &mut retired).unwrap();
    let guard = second.borrow_mut();
    assert!(matches!(
        attrs.clear(&mut retired),
        Err(RuntimeError::BorrowedTransient("AttributeInstance"))
    ));
    assert!(
        first.borrow().type_slot().is_some(),
        "preflight ends before native release mutates first"
    );
    assert_eq!(POOLS.with(|pool| pool.borrow().ints.len()), 0);
    drop(guard);
    attrs.clear(&mut retired).unwrap();
    assert_eq!(POOLS.with(|pool| pool.borrow().ints.len()), 2);
    let first_again = attrs.get_or_add(&two, &mut retired).unwrap();
    let second_again = attrs.get_or_add(&one, &mut retired).unwrap();
    assert!(Rc::ptr_eq(&first, &first_again));
    assert!(Rc::ptr_eq(&second, &second_again));
    assert!(!Rc::ptr_eq(&first_again, &second_again));
}

#[test]
fn float_validation_uses_java_nan_and_signed_zero_min_max_rules() {
    let bounds = Lookup::Found(NumericAttributeValidation {
        requires_validation: true,
        bounds: Some(NumericAttributeBounds {
            min: InitialValue::Float(0.0),
            max: InitialValue::Float(1.0),
        }),
    });
    let AttributeValue::Float(value) = validate(&bounds, AttributeValue::Float(-0.0)).unwrap()
    else {
        panic!("Float expected")
    };
    assert_eq!(
        value.to_bits(),
        0.0f32.to_bits(),
        "Math.max(-0,+0) chooses +0"
    );
    let nan_bound = Lookup::Found(NumericAttributeValidation {
        requires_validation: true,
        bounds: Some(NumericAttributeBounds {
            min: InitialValue::Float(0.0),
            max: InitialValue::Float(f64::NAN),
        }),
    });
    let AttributeValue::Float(value) = validate(&nan_bound, AttributeValue::Float(0.5)).unwrap()
    else {
        panic!("Float expected")
    };
    assert!(
        value.is_nan(),
        "Math.min(value,NaN) propagates the bound NaN"
    );
    assert_eq!(java_min(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
}

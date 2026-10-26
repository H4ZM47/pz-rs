use super::*;
use pz_entity::tables::{
    AttributeTypeInfo, InitialValue, NumericAttributeBounds, NumericAttributeValidation,
};
use pz_entity::{AttributeKind, BasicContext, ComponentType, RetiredComponents, RuntimeComponent};
use pz_inventory::shared;
use pz_inventory::{ItemClass, ItemTemplate};

struct Fixture {
    previous: Rc<dyn crate::bind_items::ItemLuaEnv>,
    context: Rc<BasicContext>,
    item: ItemRef,
    attributes: pz_entity::ComponentRef,
    fluid: pz_entity::ComponentRef,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let mut retired = RetiredComponents::new();
        for kind in [ComponentType::Attributes, ComponentType::FluidContainer] {
            pz_inventory::component_host::detach_component_for_transfer(
                &self.item,
                kind,
                &mut retired,
            )
            .unwrap();
        }
        pz_inventory::component_host::retire_components(retired);
        pz_inventory::component_host::drain_retired_components();
        crate::bind_items::install_item_env(self.previous.clone());
    }
}

fn fixture() -> Fixture {
    let mut context = BasicContext::default();
    for (id, name, kind) in [
        (0, "Sharpness", AttributeKind::Float),
        (1, "HeadCondition", AttributeKind::Int),
        (2, "HeadConditionMax", AttributeKind::Int),
        (5, "OriginX", AttributeKind::Int),
    ] {
        let (initial, min, max) = if kind == AttributeKind::Float {
            (
                InitialValue::Float(1.0),
                InitialValue::Float(0.0),
                InitialValue::Float(1.0),
            )
        } else {
            (
                InitialValue::Int(0),
                InitialValue::Int(0),
                InitialValue::Int(1000),
            )
        };
        context.attributes.insert_with_validation(
            AttributeTypeInfo {
                id,
                name: name.into(),
                kind,
                initial: Some(initial),
                hidden_ui: false,
            },
            false,
            Lookup::Found(NumericAttributeValidation {
                requires_validation: true,
                bounds: Some(NumericAttributeBounds { min, max }),
            }),
        );
    }
    let attributes =
        RuntimeComponent::new_unowned(Component::Attributes(Default::default())).unwrap();
    let fluid =
        RuntimeComponent::new_unowned(Component::FluidContainer(Default::default())).unwrap();
    let mut retired = RetiredComponents::new();
    for (id, value) in [
        (0, AttributeValue::Float(0.9)),
        (1, AttributeValue::Int(1)),
        (2, AttributeValue::Int(10)),
        (5, AttributeValue::Int(4)),
    ] {
        attributes
            .borrow_mut()
            .attributes_mut()
            .unwrap()
            .set_value(
                &context.attributes.get_ref(id).unwrap(),
                value,
                &mut retired,
            )
            .unwrap();
    }
    drop(retired);
    let mut item = InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Normal,
            "Base",
            "AuthoredSharpness",
        )),
        1,
    );
    item.condition = 2;
    item.condition_max = 10;
    item.components
        .runtime_mut()
        .push_ref_if_absent(attributes.clone())
        .unwrap();
    item.components
        .runtime_mut()
        .push_ref_if_absent(fluid.clone())
        .unwrap();
    let item = shared::try_new_item(item).unwrap();
    let previous = item_env();
    let mut environment = crate::bind_items::GameItemEnv::new(None);
    let context = Rc::new(context);
    environment.entity = Some(context.clone());
    crate::bind_items::install_item_env(Rc::new(environment));
    Fixture {
        previous,
        context,
        item,
        attributes,
        fluid,
    }
}

#[test]
fn sharpness_mutation_reports_a_needed_borrow_but_ignores_unrelated_components_and_instances() {
    let fixture = fixture();
    let (head, origin, sharpness) = {
        let body = fixture.attributes.borrow();
        let attributes = body.attributes().unwrap();
        (
            attributes.get_ref(1).unwrap().unwrap().clone(),
            attributes.get_ref(5).unwrap().unwrap().clone(),
            attributes.get_ref(0).unwrap().unwrap().clone(),
        )
    };
    let head_guard = head.borrow_mut();
    assert!(matches!(
        sharpness_mut(&fixture.item),
        Err(pz_entity::runtime_load::RuntimeHostError::Storage(
            pz_entity::RuntimeError::BorrowedTransient("AttributeInstance")
        ))
    ));
    assert_eq!(sharpness.borrow().value(), &AttributeValue::Float(0.9));
    drop(head_guard);
    let _origin_guard = origin.borrow_mut();
    let _fluid_guard = fixture.fluid.borrow_mut();
    assert_eq!(sharpness_mut(&fixture.item).unwrap(), 0.1);
    assert_eq!(sharpness.borrow().value(), &AttributeValue::Float(0.1));
    assert!(Rc::ptr_eq(
        &sharpness,
        fixture
            .attributes
            .borrow()
            .attributes()
            .unwrap()
            .get_ref(0)
            .unwrap()
            .unwrap()
    ));
    assert!(Rc::ptr_eq(
        &fixture.fluid,
        &fixture.item.borrow().get_fluid_ref().unwrap().unwrap()
    ));
}

fn instance(fixture: &Fixture, id: i16) -> pz_entity::runtime_attributes::AttributeRef {
    fixture
        .attributes
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(id)
        .unwrap()
        .unwrap()
        .clone()
}

// Native scalar load bypasses script/type validation. This lets the tests
// retain NaN payloads and -0 that a validated setter would normalize.
fn load_raw(fixture: &Fixture, values: &[(i8, AttributeValue)]) {
    let mut writer = pz_entity::io::Writer::new();
    writer.i8(8);
    writer.i16(values.len() as i16);
    for (id, value) in values {
        writer.i8(*id);
        match value {
            AttributeValue::Float(value) => writer.f32(*value),
            AttributeValue::Int(value) => writer.i32(*value),
            _ => panic!("authored numeric scalar expected"),
        }
    }
    let mut retired = RetiredComponents::new();
    let result = fixture.attributes.borrow_mut().load_payload_in_place(
        &mut pz_entity::io::Reader::new(&writer.buf),
        249,
        fixture.context.as_ref(),
        &mut retired,
    );
    pz_inventory::component_host::retire_components(retired);
    result.unwrap();
}

fn stored_float(instance: &pz_entity::runtime_attributes::AttributeRef) -> f32 {
    match instance.borrow().value() {
        AttributeValue::Float(value) => *value,
        _ => panic!("actual Float expected"),
    }
}

#[test]
fn native_raw_sharpness_nan_skips_the_setter_and_preserves_its_payload() {
    let fixture = fixture();
    let raw = f32::from_bits(0x7fc0_1234);
    load_raw(&fixture, &[(0, AttributeValue::Float(raw))]);
    let actual = instance(&fixture, 0);
    let returned = sharpness_mut(&fixture.item).unwrap();
    assert_eq!(returned.to_bits(), raw.to_bits());
    assert_eq!(stored_float(&actual).to_bits(), raw.to_bits());
    assert!(Rc::ptr_eq(&actual, &instance(&fixture, 0)));
}

#[test]
fn native_nan_maximum_ratio_skips_mutation_of_a_finite_sharpness() {
    let fixture = fixture();
    load_raw(
        &fixture,
        &[
            (0, AttributeValue::Float(0.9)),
            (1, AttributeValue::Int(0)),
            (2, AttributeValue::Int(0)),
        ],
    );
    let actual = instance(&fixture, 0);
    assert_eq!(sharpness_mut(&fixture.item).unwrap(), 0.9);
    assert_eq!(stored_float(&actual), 0.9);
}

#[test]
fn native_equal_signed_zeros_skip_script_validation_and_preserve_raw_bits() {
    let fixture = fixture();
    {
        let mut item = fixture.item.borrow_mut();
        item.condition = 0;
        item.condition_max = -1;
    }
    for raw in [-0.0f32, 0.0] {
        load_raw(&fixture, &[(0, AttributeValue::Float(raw))]);
        let actual = instance(&fixture, 0);
        let returned = sharpness_mut(&fixture.item).unwrap();
        assert_eq!(returned.to_bits(), raw.to_bits());
        assert_eq!(stored_float(&actual).to_bits(), raw.to_bits());
    }
}

#[test]
fn native_sharpness_returns_the_actual_post_script_validation_value_on_the_same_instance() {
    let fixture = fixture();
    let actual = instance(&fixture, 0);
    {
        let type_ref = fixture.context.attributes.get_ref(0).unwrap();
        let mut definition = type_ref.borrow_mut();
        definition.read_only = true;
        definition.numeric_validation = Lookup::Found(NumericAttributeValidation {
            requires_validation: true,
            bounds: Some(NumericAttributeBounds {
                min: InitialValue::Float(0.2),
                max: InitialValue::Float(1.0),
            }),
        });
    }
    // Native setValueFromScriptString ignores ordinary setValue's readonly
    // diagnostic branch and validates the captured 0.1 to the actual 0.2.
    assert_eq!(sharpness_mut(&fixture.item).unwrap(), 0.2);
    assert_eq!(stored_float(&actual), 0.2);
    assert!(Rc::ptr_eq(&actual, &instance(&fixture, 0)));
}

#[test]
fn native_sharpness_lookup_does_not_alias_a_distinct_type_with_the_same_id() {
    let fixture = fixture();
    load_raw(&fixture, &[]);
    let mut shadow_registry = pz_entity::tables::AttributeTypes::default();
    let shadow_type = shadow_registry.insert_with_validation(
        AttributeTypeInfo {
            id: 0,
            name: "ShadowSharpness".into(),
            kind: AttributeKind::Float,
            initial: Some(InitialValue::Float(1.0)),
            hidden_ui: false,
        },
        false,
        Lookup::Found(NumericAttributeValidation {
            requires_validation: false,
            bounds: None,
        }),
    );
    let mut retired = RetiredComponents::new();
    fixture
        .attributes
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&shadow_type, AttributeValue::Float(0.75), &mut retired)
        .unwrap();
    pz_inventory::component_host::retire_components(retired);
    let shadow = instance(&fixture, 0);
    assert!(!has_sharpness_ref(&fixture.item).unwrap());
    assert_eq!(sharpness_mut(&fixture.item).unwrap(), 0.0);
    assert_eq!(stored_float(&shadow), 0.75);
    assert!(Rc::ptr_eq(
        shadow.borrow().type_slot().unwrap(),
        &shadow_type
    ));
}

#[test]
fn native_sharpness_presence_and_nonmutating_getter_need_no_type_body_borrow() {
    let fixture = fixture();
    load_raw(&fixture, &[(0, AttributeValue::Float(0.0))]);
    let canonical_type = fixture.context.attributes.get_ref(0).unwrap();
    let _type_guard = canonical_type.borrow_mut();
    assert!(has_sharpness_ref(&fixture.item).unwrap());
    assert_eq!(sharpness_mut(&fixture.item).unwrap(), 0.0);
}

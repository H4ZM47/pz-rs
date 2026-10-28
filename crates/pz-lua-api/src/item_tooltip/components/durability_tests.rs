use super::*;
use pz_entity::{BasicContext, ComponentType, RetiredComponents, RuntimeComponent};
use pz_inventory::{shared, ItemClass, ItemTemplate};
use pz_scripts::objects::entity::components::combat::{DurabilityScript, MaterialType};

struct Fixture {
    item: ItemRef,
    component: pz_entity::ComponentRef,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let mut retired = RetiredComponents::new();
        for kind in [ComponentType::Attributes, ComponentType::Durability] {
            pz_inventory::component_host::detach_component_for_transfer(
                &self.item,
                kind,
                &mut retired,
            )
            .unwrap();
        }
    }
}

fn fixture() -> Fixture {
    let component =
        RuntimeComponent::new_unowned(Component::Durability(Default::default())).unwrap();
    component
        .borrow_mut()
        .read_durability_script(&DurabilityScript::new())
        .unwrap();
    let mut item = InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Normal,
            "Base",
            "ActualDurabilityTooltip",
        )),
        1,
    );
    item.components
        .runtime_mut()
        .push_ref_if_absent(component.clone())
        .unwrap();
    Fixture {
        item: shared::try_new_item(item).unwrap(),
        component,
    }
}

#[test]
fn actual_null_material_keeps_label_prefix_then_later_mutation_reads_same_body() {
    let fixture = fixture();
    let context = BasicContext::default();
    let mut layout = Layout::default();
    layout.add_item().label("earlier component", LABEL);
    let error = durability_tooltip(&fixture.item, &context, &mut layout).unwrap_err();
    assert!(matches!(
        error,
        pz_kahlua::LuaError::Runtime {
            class: "java.lang.NullPointerException",
            ..
        }
    ));
    assert_eq!(layout.items.len(), 2);
    assert_eq!(layout.items[0].label.as_deref(), Some("earlier component"));
    assert_eq!(layout.items[1].label.as_deref(), Some("Material:"));
    assert_eq!(layout.items[1].value, None);
    {
        let mut body = fixture.component.borrow_mut();
        let durability = body.durability_mut().unwrap();
        durability.material = Some(MaterialType::Metal_Light);
        durability.current_hit_points = -3.25;
    }
    durability_tooltip(&fixture.item, &context, &mut layout).unwrap();
    assert_eq!(layout.items.len(), 4);
    assert_eq!(layout.items[2].value.as_deref(), Some("Metal_Light"));
    assert_eq!(layout.items[3].value.as_deref(), Some("-3.25"));
}

#[test]
fn borrowed_actual_durability_is_a_host_error_before_any_new_layout_row() {
    let fixture = fixture();
    let mut layout = Layout::default();
    layout.add_item().label("existing", LABEL);
    let guard = fixture.component.borrow_mut();
    assert!(durability_tooltip(&fixture.item, &BasicContext::default(), &mut layout).is_err());
    assert_eq!(layout.items.len(), 1);
    drop(guard);
    fixture
        .component
        .borrow_mut()
        .durability_mut()
        .unwrap()
        .material = Some(MaterialType::Glass);
    durability_tooltip(&fixture.item, &BasicContext::default(), &mut layout).unwrap();
    assert_eq!(layout.items[1].value.as_deref(), Some("Glass"));
}

#[test]
fn aggregate_component_tooltip_keeps_actual_attribute_rows_before_null_material_failure() {
    use pz_entity::components::attributes::AttributeValue;
    use pz_entity::tables::{AttributeTypeInfo, InitialValue, NumericAttributeValidation};
    use pz_kahlua::{TableRef, Thread};
    let fixture = fixture();
    let mut context = BasicContext::default();
    let kind = context.attributes.insert_with_validation(
        AttributeTypeInfo {
            id: 12,
            name: "VisibleCounter".into(),
            kind: pz_entity::AttributeKind::Int,
            initial: Some(InitialValue::Int(0)),
            hidden_ui: false,
        },
        false,
        Lookup::Found(NumericAttributeValidation {
            requires_validation: false,
            bounds: None,
        }),
    );
    let attributes =
        RuntimeComponent::new_unowned(Component::Attributes(Default::default())).unwrap();
    let mut retired = RetiredComponents::new();
    attributes
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&kind, AttributeValue::Int(17), &mut retired)
        .unwrap();
    pz_inventory::component_host::ItemComponentHost::new(&fixture.item, &context)
        .add(&attributes, &mut retired)
        .unwrap();
    struct RestoreEnv(Rc<dyn crate::bind_items::ItemLuaEnv>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            crate::bind_items::install_item_env(self.0.clone());
        }
    }
    let _restore = RestoreEnv(item_env());
    let mut environment = crate::bind_items::GameItemEnv::new(None);
    environment.entity = Some(Rc::new(context));
    crate::bind_items::install_item_env(Rc::new(environment));
    let mut thread = Thread::new(TableRef::new());
    let mut cx = super::super::Cx::new(&mut thread, None);
    cx.opts.tooltip_attributes = false;
    let element = pz_ui::tooltip::ObjectTooltip::create("Small");
    let tip = super::super::Tip::of(&element).unwrap();
    let mut layout = Layout::default();
    let error = do_tooltip(&mut cx, &fixture.item, &tip, &mut layout).unwrap_err();
    assert!(matches!(
        error,
        pz_kahlua::LuaError::Runtime {
            class: "java.lang.NullPointerException",
            ..
        }
    ));
    assert_eq!(layout.items.len(), 2);
    assert_eq!(layout.items[0].value.as_deref(), Some("17"));
    assert_eq!(layout.items[1].label.as_deref(), Some("Material:"));
    assert_eq!(layout.items[1].value, None);
}

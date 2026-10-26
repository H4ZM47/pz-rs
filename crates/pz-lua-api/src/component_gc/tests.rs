use super::*;
use pz_entity::{Component, Components, EntityOwner, FluidContainer};
use pz_inventory::{ItemClass, ItemTemplate};
use std::cell::Cell;

struct Restore {
    item: Option<pz_inventory::shared::ItemObserver>,
    object: Option<fn(&IsoObjectRef)>,
}

impl Restore {
    fn new() -> Self {
        let _ = gc::collect_tables();
        let _ = take_registration_faults();
        Self {
            item: pz_inventory::shared::install_item_observer(Some(observe_item)),
            object: pz_world::object_slot::install_object_observer(Some(observe_object)),
        }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        pz_inventory::shared::install_item_observer(self.item);
        pz_world::object_slot::install_object_observer(self.object);
        pz_inventory::component_host::drain_retired_components();
        retry_pending();
        let _ = gc::collect_tables();
        let _ = take_registration_faults();
    }
}

fn item(class: ItemClass) -> (ItemRef, ComponentRef) {
    let mut item = InventoryItem::create(
        Rc::new(ItemTemplate::new(class, "Base", "GcFluidFixture")),
        1,
    );
    item.components =
        pz_inventory::item_components::ItemComponents::from_values_unowned(Components {
            list: vec![Component::FluidContainer(FluidContainer::default())],
        })
        .unwrap();
    let item = pz_inventory::shared::try_new_item(item).unwrap();
    let component = item.borrow().get_fluid_ref().unwrap().unwrap();
    (item, component)
}

#[test]
fn never_exposed_actual_item_component_cycle_collects_after_last_root() {
    let _restore = Restore::new();
    let (item, component) = item(ItemClass::Normal);
    let weak_item = Rc::downgrade(&item);
    let weak_component = Rc::downgrade(&component);
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&item)));
    drop(item);
    drop(component);
    assert!(
        weak_item.upgrade().is_some(),
        "actual strong native backedge formed a cycle"
    );
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak_item.upgrade().is_none());
    assert!(weak_component.upgrade().is_none());
}

#[test]
fn actual_world_allocation_observer_preserves_raw_component_root_then_collects_owner_cycle() {
    let _restore = Restore::new();
    let mut value = IsoObject::new(7, pz_world::ObjectKind::Object, None);
    value.key_id = 73;
    value.components =
        pz_inventory::item_components::ItemComponents::from_values_unowned(Components {
            list: vec![Component::FluidContainer(FluidContainer::default())],
        })
        .unwrap();
    // ObjectSlot must observe this actual allocation before attaching the
    // strong owner; this case never constructs item, object or fluid userdata.
    let slot = pz_world::ObjectSlot::new(value);
    let object = slot.ref_handle();
    let component = object
        .borrow()
        .components
        .get_ref(pz_entity::ComponentType::FluidContainer)
        .unwrap()
        .unwrap();
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    *color.borrow_mut() = [0.2, 0.4, 0.6, 0.8];
    let weak_object = Rc::downgrade(&object);
    let weak_component = Rc::downgrade(&component);
    assert!(object.borrow().components.is_bound());
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&object)));
    drop(slot);
    drop(object);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    {
        let object = weak_object
            .upgrade()
            .expect("the retained actual ComponentRef roots its actual world owner");
        assert_eq!(object.borrow().key_id, 73);
        let member = object
            .borrow()
            .components
            .get_ref(pz_entity::ComponentType::FluidContainer)
            .unwrap()
            .unwrap();
        assert!(Rc::ptr_eq(&member, &component));
        assert!(component
            .borrow()
            .owner_slot()
            .unwrap()
            .ptr_eq(&EntityOwner::from_ref(&object)));
    }
    drop(component);
    assert!(
        weak_object.upgrade().is_some(),
        "actual native cycle exists"
    );
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak_object.upgrade().is_none());
    assert!(weak_component.upgrade().is_none());
    // Retaining only the actual final cache cannot keep either former owner
    // alive, and collector detachment must not run native reset or pool free.
    assert_eq!(*color.borrow(), [0.2, 0.4, 0.6, 0.8]);
}

#[test]
fn retained_actual_component_or_item_userdata_roots_preserve_live_data() {
    let _restore = Restore::new();
    let (item, component) = item(ItemClass::Normal);
    item.borrow_mut().condition = 37;
    let value = crate::bind_items::item_value(&item);
    let weak_item = Rc::downgrade(&item);
    drop(item);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert_eq!(weak_item.upgrade().unwrap().borrow().condition, 37);
    drop(value);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(
        weak_item.upgrade().is_some(),
        "raw actual ComponentRef remains a root"
    );
    assert!(component.borrow().owner_slot().is_some());
    drop(component);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak_item.upgrade().is_none());
}

#[test]
fn retained_color_roots_its_actual_color_only_and_not_the_former_component() {
    let _restore = Restore::new();
    let (item, component) = item(ItemClass::Normal);
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    let value = crate::bind_fluid_container::color_value(&color).unwrap();
    let weak_item = Rc::downgrade(&item);
    let weak_component = Rc::downgrade(&component);
    drop(item);
    drop(component);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak_item.upgrade().is_none());
    assert!(weak_component.upgrade().is_none());
    assert_eq!(
        crate::bind_misc_classes::try_color_of(&value).unwrap(),
        Some(*color.borrow())
    );
    assert!(matches!(value, pz_kahlua::Value::Object(_)));
}

#[test]
fn held_component_guard_defers_collection_and_cached_getter_metadata_retries() {
    let _restore = Restore::new();
    let (item, component) = item(ItemClass::Normal);
    let body = component.borrow_mut();
    let first = crate::bind_fluid_container::fluid_value(&component).unwrap();
    assert_eq!(
        register_component(&component).unwrap(),
        MetadataStatus::Deferred
    );
    assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    drop(body);
    let second = crate::bind_fluid_container::fluid_value(&component).unwrap();
    assert_eq!(first.ptr_id(), second.ptr_id());
    retry_pending();
    assert_eq!(
        register_component(&component).unwrap(),
        MetadataStatus::Complete
    );
    assert!(matches!(
        gc::collect_pending(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(component.borrow().owner_slot().is_some());
    drop((first, second, item, component));
}

#[test]
fn unknown_owner_remains_conservative_and_is_not_cleared_as_a_known_entity() {
    struct Opaque {
        component: ComponentRef,
    }
    let _restore = Restore::new();
    let component =
        pz_entity::RuntimeComponent::new_unowned(Component::FluidContainer(Default::default()))
            .unwrap();
    let opaque = Rc::new(RefCell::new(Opaque {
        component: component.clone(),
    }));
    let _ = component
        .borrow_mut()
        .replace_owner(Some(&EntityOwner::from_ref(&opaque)));
    register_component(&component).unwrap();
    let weak = Rc::downgrade(&component);
    drop(component);
    drop(opaque);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    let component = weak
        .upgrade()
        .expect("unregistered owner outgoing edge stays an external root");
    let owner = component
        .borrow()
        .owner_slot()
        .unwrap()
        .try_handle::<Opaque>()
        .unwrap();
    assert!(Rc::ptr_eq(&owner.borrow().component, &component));
    assert!(component.borrow().owner_slot().is_some());
    let retired = component.borrow_mut().replace_owner(None);
    drop(retired);
    drop(owner);
    drop(component);
}

#[test]
fn metadata_conflict_is_recorded_and_genuine_fluid_getter_errors_without_erasing_data() {
    struct Other;
    impl TraceDescriptor<InventoryItem> for Other {
        fn trace(_: &InventoryItem, _: &mut TraceVisitor) {}
    }
    let _restore = Restore::new();
    let item = Rc::new(RefCell::new(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Conflict")),
        1,
    )));
    item.borrow_mut().condition = 19;
    gc::register_existing::<_, Other>(&item).unwrap();
    let fault = register_item(&item).unwrap_err();
    assert_eq!(fault.error, RegistrationError::DescriptorConflict);
    assert!(take_registration_faults().contains(&fault));
    assert_eq!(item.borrow().condition, 19);
    item.borrow_mut().components =
        pz_inventory::item_components::ItemComponents::from_values_unowned(Components {
            list: vec![Component::FluidContainer(FluidContainer::default())],
        })
        .unwrap();
    let component = item.borrow().get_fluid_ref().unwrap().unwrap();
    let _ = component
        .borrow_mut()
        .replace_owner(Some(&EntityOwner::from_ref(&item)));
    assert!(
        crate::bind_fluid_container::fluid_value(&component).is_err(),
        "metadata conflicts are not a fake missing component"
    );
    assert!(component.borrow().owner_slot().is_some());
    let value = crate::bind_items::item_value(&item);
    assert!(Rc::ptr_eq(
        &crate::bind_items::item_arg_value(&value).unwrap(),
        &item
    ));
    assert!(take_registration_faults().contains(&fault));
    let entries = item.borrow_mut().components.take_entries();
    let owner = component.borrow_mut().replace_owner(None);
    drop((entries, owner));
}

struct ReentrantParent {
    dropped: Rc<Cell<bool>>,
    sprites: Rc<RefCell<pz_world::sprite::IsoSpriteManager>>,
}

impl Drop for ReentrantParent {
    fn drop(&mut self) {
        crate::bind_world::with_world(|_| {
            let _sprites = self.sprites.borrow_mut();
            self.dropped.set(true);
        });
    }
}

#[test]
fn actual_item_final_node_pin_drop_reenters_world_and_sprite_only_after_guards_end() {
    let _restore = Restore::new();
    let (item, component) = item(ItemClass::Container);
    let sprites = Rc::new(RefCell::new(pz_world::sprite::IsoSpriteManager::new(
        pz_world::sprite::test_tables(),
    )));
    let dropped = Rc::new(Cell::new(false));
    let inventory = pz_inventory::shared::inventory_of(&item.borrow()).unwrap();
    inventory.borrow_mut().parent = Some(Rc::new(ReentrantParent {
        dropped: dropped.clone(),
        sprites: sprites.clone(),
    }));
    drop(inventory);
    drop(item);
    drop(component);
    crate::bind_world::with_world(|_| {
        let _sprites = sprites.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
        assert!(!dropped.get());
    });
    assert!(!dropped.get());
    assert!(matches!(
        gc::collect_pending(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(dropped.get());
}

fn attribute_context() -> (pz_entity::BasicContext, pz_entity::tables::AttributeTypeRef) {
    use pz_entity::tables::{
        AttributeTypeInfo, InitialValue, NumericAttributeBounds, NumericAttributeValidation,
    };
    let mut context = pz_entity::BasicContext::default();
    let type_ref = context.attributes.insert_with_validation(
        AttributeTypeInfo {
            id: 9,
            name: "Sharpness".into(),
            kind: pz_entity::AttributeKind::Float,
            initial: Some(InitialValue::Float(1.0)),
            hidden_ui: false,
        },
        false,
        pz_entity::Lookup::Found(NumericAttributeValidation {
            requires_validation: true,
            bounds: Some(NumericAttributeBounds {
                min: InitialValue::Float(0.0),
                max: InitialValue::Float(1.0),
            }),
        }),
    );
    (context, type_ref)
}

#[test]
fn actual_attribute_instance_guard_defers_gc_and_raw_component_keeps_its_real_owner() {
    use pz_entity::components::attributes::AttributeValue;
    let _restore = Restore::new();
    let (_context, type_ref) = attribute_context();
    let component =
        pz_entity::runtime_pool::obtain_component(pz_entity::ComponentType::Attributes).unwrap();
    let mut retired = pz_entity::RetiredComponents::new();
    component
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&type_ref, AttributeValue::Float(0.75), &mut retired)
        .unwrap();
    let instance = component
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(9)
        .unwrap()
        .unwrap()
        .clone();
    let mut value = InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "GcAttributes")),
        1,
    );
    value
        .components
        .runtime_mut()
        .push_ref_if_absent(component.clone())
        .unwrap();
    let item = pz_inventory::shared::try_new_item(value).unwrap();
    let weak_item = Rc::downgrade(&item);
    drop(retired);
    drop(item);
    let guard = instance.borrow_mut();
    assert_eq!(
        register_attribute(&instance).unwrap(),
        MetadataStatus::Deferred
    );
    assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    drop(guard);
    retry_pending();
    assert!(matches!(
        gc::collect_pending(),
        gc::CollectionResult::Collected(_)
    ));
    let item = weak_item.upgrade().expect("raw component owns actual item");
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&item)));
    assert!(Rc::ptr_eq(
        instance.borrow().type_slot().unwrap(),
        &type_ref
    ));
    assert_eq!(instance.borrow().value(), &AttributeValue::Float(0.75));
    drop((item, component));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak_item.upgrade().is_none());
    assert!(Rc::ptr_eq(
        instance.borrow().type_slot().unwrap(),
        &type_ref
    ));
    assert_eq!(instance.borrow().value(), &AttributeValue::Float(0.75));
}

#[test]
fn native_script_template_and_instance_edges_collect_without_native_reset_or_pool_release() {
    use pz_entity::components::attributes::AttributeValue;
    use pz_scripts::base::ScriptObject;
    let _restore = Restore::new();
    let (context, type_ref) = attribute_context();
    let manager = pz_scripts::ScriptManager::new();
    let mut definition = GameEntityScript::new();
    definition.init_load_pp(&manager, "GcAttributeDefinition");
    definition
        .load(
            &manager,
            "GcAttributeDefinition",
            "entity GcAttributeDefinition { component Attributes { Sharpness = 0.75, } }",
        )
        .unwrap();
    let definition = Rc::new(RefCell::new(definition));
    let mut retired = pz_entity::RetiredComponents::new();
    let template = {
        let definition = definition.borrow();
        let attributes = definition
            .component::<AttributesScript>(pz_entity::ComponentType::Attributes)
            .unwrap();
        pz_entity::runtime_attributes::get_or_create_attributes_template(
            attributes,
            &context,
            &|| Ok(false),
            &mut Vec::new(),
            &mut retired,
        )
        .unwrap()
        .unwrap()
    };
    let instance = template
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(9)
        .unwrap()
        .unwrap()
        .clone();
    let script_info =
        pz_entity::RuntimeComponent::new_unowned(Component::Script(Default::default())).unwrap();
    assert!(script_info
        .borrow_mut()
        .set_original_script_actual(
            false,
            "GcAttributeDefinition".into(),
            &EntityOwner::from_ref(&definition),
        )
        .unwrap()
        .is_none());
    let mut value = InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Normal,
            "Base",
            "GcDefinitionOwner",
        )),
        1,
    );
    value
        .components
        .runtime_mut()
        .push_ref_if_absent(script_info.clone())
        .unwrap();
    let item = pz_inventory::shared::try_new_item(value).unwrap();
    let weak_item = Rc::downgrade(&item);
    let weak_info = Rc::downgrade(&script_info);
    let weak_definition = Rc::downgrade(&definition);
    let weak_template = Rc::downgrade(&template);
    drop(retired);
    drop((item, script_info, definition, template));
    assert!(
        weak_item.upgrade().is_some(),
        "actual item-owner cycle exists"
    );
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak_item.upgrade().is_none());
    assert!(weak_info.upgrade().is_none());
    assert!(weak_definition.upgrade().is_none());
    assert!(weak_template.upgrade().is_none());
    // The retained raw native instance owns its actual type, not its former
    // container. Collector teardown must not run release's type=null reset.
    assert!(Rc::ptr_eq(
        instance.borrow().type_slot().unwrap(),
        &type_ref
    ));
    assert_eq!(instance.borrow().value(), &AttributeValue::Float(0.75));
    assert!(
        pz_scripts::objects::entity::components::attributes::take_retired_attribute_templates()
            .is_empty(),
        "the collector moved the physical script cache before dropping its Box"
    );
}

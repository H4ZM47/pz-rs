use super::*;
use crate::component::ComponentCodec;
use crate::env::ItemEnv;
use crate::io::{Reader, Writer};
use crate::{InventoryItem, ItemClass, ItemTemplate};
use pz_entity::runtime_load::{load_native_with, RuntimeLoadOptions};
use pz_entity::{BasicContext, Component, Components, FluidContainer, RuntimeComponent};
use std::cell::Cell;
use std::rc::Rc;

fn value() -> InventoryItem {
    InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "LiveTest")),
        1,
    )
}

fn actual_with(component: &ComponentRef) -> ItemRef {
    let mut item = value();
    item.components
        .runtime_mut()
        .push_ref_if_absent(component.clone())
        .unwrap();
    shared::try_new_item(item).unwrap()
}

thread_local! { static OBSERVED: Cell<usize> = const { Cell::new(0) }; }
thread_local! {
    static LOADED_ATTRIBUTES: RefCell<Vec<pz_entity::components::attributes::AttributeValue>> = const { RefCell::new(Vec::new()) };
    static FACTORY_NOTICES: RefCell<Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>> = const { RefCell::new(Vec::new()) };
}
thread_local! {
    static NOTICE_ITEM: RefCell<Option<std::rc::Weak<std::cell::RefCell<InventoryItem>>>> = const { RefCell::new(None) };
    static NOTICE_SCRIPT: RefCell<Option<std::rc::Weak<std::cell::RefCell<pz_scripts::objects::items::Item>>>> = const { RefCell::new(None) };
    static NOTICES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn before_owner(item: &ItemRef) {
    let item = item
        .try_borrow()
        .expect("observer cannot receive an item guard");
    for entry in item.components.entry_slots() {
        if let pz_entity::RuntimeEntryView::Live { component, .. } = entry {
            assert!(component.try_borrow().unwrap().owner_slot().is_none());
        }
    }
    OBSERVED.with(|count| count.set(count.get() + 1));
}

#[test]
fn actual_creation_observes_before_owner_and_archive_copy_rebinds_independent_refs() {
    let old = shared::install_item_observer(Some(before_owner));
    OBSERVED.with(|count| count.set(0));
    let component =
        RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer::default()))
            .unwrap();
    let actual = actual_with(&component);
    assert_eq!(OBSERVED.with(Cell::get), 1);
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&actual)));
    let archived = actual.borrow().clone();
    let copied_component = archived.get_fluid_ref().unwrap().unwrap();
    assert!(!Rc::ptr_eq(&component, &copied_component));
    assert!(copied_component.borrow().owner_slot().is_none());
    let copied = shared::try_new_item(archived).unwrap();
    assert!(copied_component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&copied)));
    shared::install_item_observer(old);
    let mut retired = RetiredComponents::new();
    detach_component_for_transfer(&actual, ComponentType::FluidContainer, &mut retired).unwrap();
    detach_component_for_transfer(&copied, ComponentType::FluidContainer, &mut retired).unwrap();
}

#[test]
fn bound_empty_item_rejects_unowned_bulk_add_instead_of_creating_ownerless_live_component() {
    let actual = shared::try_new_item(value()).unwrap();
    let mut values = Components::new();
    values.add(Component::FluidContainer(FluidContainer::default()));
    assert!(matches!(
        actual.borrow_mut().set_components(values),
        Err(RuntimeHostError::Unsupported { .. })
    ));
    assert!(actual.borrow().components.is_empty());
}

#[test]
fn actual_transfer_keeps_allocation_and_old_owner_pin_until_outer_retirement() {
    let component = RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer {
        capacity: 7.25,
        ..Default::default()
    }))
    .unwrap();
    let source = actual_with(&component);
    let target = shared::try_new_item(value()).unwrap();
    let old_source = Rc::downgrade(&source);
    let context = BasicContext::default();
    let mut retired = RetiredComponents::new();
    let detached =
        detach_component_for_transfer(&source, ComponentType::FluidContainer, &mut retired)
            .unwrap()
            .unwrap();
    assert!(Rc::ptr_eq(&component, &detached));
    assert!(source.borrow().components.is_empty());
    assert!(component.borrow().owner_slot().is_none());
    let mut host = ItemComponentHost::new(&target, &context);
    assert!(host.add(&detached, &mut retired).unwrap());
    host.connect(&mut retired).unwrap();
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&target)));
    assert_eq!(component.borrow().fluid().unwrap().capacity, 7.25);
    drop(source);
    assert!(
        old_source.upgrade().is_some(),
        "detached native owner is retained outside all guards"
    );
    drop(retired);
    assert!(old_source.upgrade().is_none());
    let mut retired = RetiredComponents::new();
    detach_component_for_transfer(&target, ComponentType::FluidContainer, &mut retired).unwrap();
}

#[test]
fn actual_host_failed_body_retains_prefix_and_existing_component_identity() {
    let component = RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer {
        capacity: 88.0,
        rain_catcher: 0.75,
        ..Default::default()
    }))
    .unwrap();
    let item = actual_with(&component);
    let context = BasicContext::default();
    let mut host = ItemComponentHost::new(&item, &context);
    let mut writer = pz_entity::io::Writer::new();
    writer.i8(1);
    writer.i32(4);
    writer.i16(2);
    writer.i16(1); // Capacity flag, missing f32.
    let report = load_native_with(
        &mut pz_entity::io::Reader::new(&writer.buf),
        249,
        &mut host,
        RuntimeLoadOptions {
            collect_loaded: true,
        },
    )
    .unwrap();
    assert_eq!(report.issues.len(), 1);
    assert!(report.loaded.is_empty());
    assert_eq!(component.borrow().fluid().unwrap().capacity, 1.0);
    assert_eq!(component.borrow().fluid().unwrap().rain_catcher, 0.75);
    assert!(Rc::ptr_eq(
        &component,
        &item.borrow().get_fluid_ref().unwrap().unwrap()
    ));
    let mut retired = report.retired;
    detach_component_for_transfer(&item, ComponentType::FluidContainer, &mut retired).unwrap();
}

#[test]
fn legacy_body_write_preserves_final_cache_allocations_and_invalidates_derived_data() {
    let component =
        RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer::default()))
            .unwrap();
    let item = actual_with(&component);
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    let properties = component.borrow().fluid_properties_slot().unwrap().clone();
    let retired_data = item
        .borrow_mut()
        .set_fluid_container(FluidContainer {
            capacity: 15.25,
            ..Default::default()
        })
        .unwrap();
    // Finish the item guard before releasing the returned owning slots.
    drop(retired_data);
    assert!(Rc::ptr_eq(
        &color,
        component.borrow().fluid_color_slot().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &properties,
        component.borrow().fluid_properties_slot().unwrap()
    ));
    assert!(component.borrow().fluid().unwrap().cache_invalidated);
    let outcome = component
        .borrow_mut()
        .fluid_recalculate_data(&BasicContext::default(), false)
        .unwrap();
    notify_fluid_update(&outcome).unwrap();
    assert!(outcome.recalculated);
    assert_eq!(component.borrow().fluid().unwrap().capacity, 15.25);
    let mut retired = RetiredComponents::new();
    detach_component_for_transfer(&item, ComponentType::FluidContainer, &mut retired).unwrap();
}

struct PlainEnv;
impl ItemEnv for PlainEnv {}

#[test]
fn tiny_positive_item_record_loads_original_buffer_and_rewinds_only_after_success() {
    let actual = shared::try_new_item(value()).unwrap();
    let mut source = value();
    source.id = 123456;
    let mut record = Writer::new();
    source.save(&mut record, &PlainEnv).unwrap();
    let mut input = Writer::new();
    input.i32(1);
    input.bytes(&record.buf);
    input.i32(0x12345678);
    let mut reader = Reader::new(&input.buf);
    let old_debug = install_component_debug_query(None);
    let result =
        crate::saved::load_item_record(&mut reader, 249, &PlainEnv, Some(&actual)).unwrap();
    install_component_debug_query(old_debug);
    assert!(Rc::ptr_eq(result.item.as_ref().unwrap(), &actual));
    assert_eq!(
        actual.borrow().id,
        123456,
        "body was read beyond declared one-byte record"
    );
    assert_eq!(
        reader.pos, 5,
        "original sized wrapper repairs cursor after the virtual body load"
    );
}

fn true_debug() -> HostResult<bool> {
    Ok(true)
}

#[test]
fn debug_mismatch_throws_after_seek_and_retains_supplied_actual_receiver_mutation() {
    let actual = shared::try_new_item(value()).unwrap();
    let mut source = value();
    source.id = 345678;
    let mut record = Writer::new();
    source.save(&mut record, &PlainEnv).unwrap();
    let mut input = Writer::new();
    input.i32(1);
    input.bytes(&record.buf);
    let mut reader = Reader::new(&input.buf);
    let old_debug = install_component_debug_query(Some(true_debug));
    let failure =
        crate::saved::load_item_record(&mut reader, 249, &PlainEnv, Some(&actual)).unwrap_err();
    install_component_debug_query(old_debug);
    assert_eq!(reader.pos, 5);
    assert_eq!(actual.borrow().id, 345678);
    assert!(Rc::ptr_eq(failure.receiver.as_ref().unwrap(), &actual));
    assert_eq!(
        failure.error,
        RuntimeHostError::NativeException(EntityError(
            "InventoryItem.loadItem() read more data than save() wrote (Unknown)".into()
        ))
    );
}

#[test]
fn null_item_skip_consumes_one_byte_at_a_time_before_underflow_outside_body_catch() {
    let mut input = Writer::new();
    input.i32(20);
    input.i16(999);
    input.i8(0);
    input.bytes(&[8, 9, 10]);
    let mut reader = Reader::new(&input.buf);
    let failure = crate::saved::load_item_record(&mut reader, 249, &PlainEnv, None).unwrap_err();
    assert_eq!(reader.pos, input.buf.len());
    assert_eq!(failure.phase, crate::saved::LiveRecordPhase::Repair);
    assert!(failure.receiver.is_none());
    assert!(matches!(
        failure.error,
        RuntimeHostError::NativeException(_)
    ));
}

struct ConstructorCodec;
impl ComponentCodec for ConstructorCodec {
    fn factory_runtime(
        &self,
        _: &ItemRef,
        _: &ItemTemplate,
        _: &mut dyn crate::component::FactoryRng,
        _: &mut RetiredComponents,
        _: &mut Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>,
    ) -> HostResult<()> {
        Err(RuntimeHostError::NativeException(EntityError(
            "authored constructor failure".into(),
        )))
    }
}
struct ConstructorEnv;
impl ItemEnv for ConstructorEnv {
    fn template(&self, _: i16) -> Option<Rc<ItemTemplate>> {
        Some(value().template)
    }
    fn components(&self) -> &dyn ComponentCodec {
        &ConstructorCodec
    }
}

#[test]
fn constructor_exception_escapes_before_native_body_catch_and_record_repair() {
    let mut writer = Writer::new();
    writer.i32(30);
    writer.i16(1);
    writer.i8(0);
    writer.bytes(&[1, 2, 3]);
    let mut reader = Reader::new(&writer.buf);
    let failure =
        crate::saved::load_item_record(&mut reader, 249, &ConstructorEnv, None).unwrap_err();
    assert_eq!(reader.pos, 7);
    assert_eq!(failure.phase, crate::saved::LiveRecordPhase::Factory);
    assert_eq!(
        failure.error,
        RuntimeHostError::NativeException(EntityError("authored constructor failure".into()))
    );
}

struct RetirementProbe(Rc<RefCell<usize>>);
impl Drop for RetirementProbe {
    fn drop(&mut self) {
        *self.0.borrow_mut() += 1;
    }
}

struct RetiringFactoryCodec(RefCell<Option<EntityOwner>>);
impl ComponentCodec for RetiringFactoryCodec {
    fn factory_runtime(
        &self,
        _: &ItemRef,
        _: &ItemTemplate,
        _: &mut dyn crate::component::FactoryRng,
        retired: &mut RetiredComponents,
        _: &mut Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>,
    ) -> HostResult<()> {
        if let Some(owner) = self.0.borrow_mut().take() {
            retired.push_owner(owner);
        }
        Ok(())
    }
}

struct RetiringFactoryEnv(RetiringFactoryCodec);
impl ItemEnv for RetiringFactoryEnv {
    fn components(&self) -> &dyn ComponentCodec {
        &self.0
    }
}

#[test]
fn successful_factory_defers_final_retired_owner_until_outer_guard_ends() {
    drain_retired_components();
    let outer = Rc::new(RefCell::new(0));
    let probe = Rc::new(RefCell::new(RetirementProbe(outer.clone())));
    let env = RetiringFactoryEnv(RetiringFactoryCodec(RefCell::new(Some(
        EntityOwner::from_ref(&probe),
    ))));
    drop(probe);
    let guard = outer.borrow_mut();
    let actual = crate::env::create_from_template_ref(&env, value().template, 1).unwrap();
    assert_eq!(*guard, 0, "factory success must keep retired owners pinned");
    drop(guard);
    drain_retired_components();
    assert_eq!(*outer.borrow(), 1);
    drop(actual);
}

#[test]
fn archive_preflight_rejects_borrowed_actual_transient_cache_before_copying() {
    let component =
        RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer::default()))
            .unwrap();
    let actual = actual_with(&component);
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    let guard = color.borrow_mut();
    assert_eq!(
        crate::archive::ArchiveCopyCheck::default().item_ref(&actual),
        Err(crate::archive::ArchiveCopyError::BorrowedComponent)
    );
    assert!(matches!(
        actual.borrow().components.try_archive_copy_unowned(),
        Err(RuntimeError::BorrowedTransient(_))
    ));
    drop(guard);
    assert!(crate::archive::ArchiveCopyCheck::default()
        .item_ref(&actual)
        .is_ok());
    let mut retired = RetiredComponents::new();
    detach_component_for_transfer(&actual, ComponentType::FluidContainer, &mut retired).unwrap();
}

#[test]
fn actual_load_without_entity_flag_retains_factory_component_and_same_owner() {
    let component = RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer {
        rain_catcher: 0.875,
        ..Default::default()
    }))
    .unwrap();
    let actual = actual_with(&component);
    let mut record = Writer::new();
    value().save(&mut record, &PlainEnv).unwrap();
    let mut reader = Reader::new(&record.buf[3..]);
    let retired = load_item_ref(&actual, &mut reader, 249, &PlainEnv).unwrap();
    assert!(retired.is_empty());
    assert!(Rc::ptr_eq(
        &component,
        &actual.borrow().get_fluid_ref().unwrap().unwrap()
    ));
    assert_eq!(component.borrow().fluid().unwrap().rain_catcher, 0.875);
    let mut retired = RetiredComponents::new();
    detach_component_for_transfer(&actual, ComponentType::FluidContainer, &mut retired).unwrap();
}

fn known_fluids(with_properties: bool) -> BasicContext {
    let mut registry = pz_entity::FluidRegistry::default();
    for (name, color, calories) in [
        ("Example.Red", [1.0, 0.0, 0.0, 1.0], 10.0),
        ("Example.Blue", [0.0, 0.0, 1.0, 1.0], 30.0),
    ] {
        registry.insert(
            None,
            pz_entity::FluidInfo {
                name: name.into(),
                color,
                ..Default::default()
            },
        );
        if with_properties {
            registry.set_properties(
                name,
                Some(pz_entity::fluids::SealedFluidProperties {
                    calories,
                    ..Default::default()
                }),
            );
        }
    }
    BasicContext {
        fluids: Some(registry),
        ..Default::default()
    }
}

fn overfilled_entity_bytes(padding: usize) -> Vec<u8> {
    let body = FluidContainer {
        capacity: 1.0,
        fluids: vec![
            pz_entity::FluidInstance::new(pz_entity::FluidRef::Modded("Example.Red".into()), 1.0),
            pz_entity::FluidInstance::new(pz_entity::FluidRef::Modded("Example.Blue".into()), 1.0),
        ],
        ..Default::default()
    };
    let mut writer = pz_entity::io::Writer::new();
    writer.i8(1);
    let start = writer.block_start();
    writer.i16(2);
    body.save(&mut writer);
    writer.bytes(&vec![0x66; padding]);
    writer.block_end(start);
    writer.buf
}

#[test]
fn actual_host_load_computes_color_and_properties_before_native_amount_clamp() {
    let component =
        RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer::default()))
            .unwrap();
    let actual = actual_with(&component);
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    let properties = component.borrow().fluid_properties_slot().unwrap().clone();
    let context = known_fluids(true);
    let mut host = ItemComponentHost::new(&actual, &context);
    let bytes = overfilled_entity_bytes(0);
    let report = load_native_with(
        &mut pz_entity::io::Reader::new(&bytes),
        249,
        &mut host,
        RuntimeLoadOptions {
            collect_loaded: true,
        },
    )
    .unwrap();
    assert!(report.issues.is_empty());
    assert!(Rc::ptr_eq(&report.loaded[0], &component));
    assert_eq!(component.borrow().fluid().unwrap().amount_cache, 1.0);
    assert_eq!(
        component
            .borrow()
            .fluid()
            .unwrap()
            .fluids
            .iter()
            .map(|fluid| fluid.amount)
            .sum::<f32>(),
        2.0
    );
    assert_eq!(
        component.borrow().fluid_runtime().unwrap().percentages(),
        &[0.5, 0.5]
    );
    assert_eq!(*color.borrow(), [0.5, 0.0, 0.5, 1.0]);
    // Native recalculateCaches PCs222–248 weights properties by raw total
    // amount * percentage (one litre of each), before the later cap clamp.
    assert_eq!(properties.borrow().calories, 40.0);
    let mut retired = report.retired;
    detach_component_for_transfer(&actual, ComponentType::FluidContainer, &mut retired).unwrap();
}

#[test]
fn missing_properties_capability_is_fatal_with_completed_data_prefix_and_no_forced_end() {
    let component =
        RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer::default()))
            .unwrap();
    let actual = actual_with(&component);
    let context = known_fluids(false);
    let mut host = ItemComponentHost::new(&actual, &context);
    let bytes = overfilled_entity_bytes(4);
    let mut reader = pz_entity::io::Reader::new(&bytes);
    let failure = load_native_with(
        &mut reader,
        249,
        &mut host,
        RuntimeLoadOptions {
            collect_loaded: true,
        },
    )
    .unwrap_err();
    assert_eq!(
        failure.phase,
        pz_entity::runtime_load::RuntimeLoadPhase::LoadComponent
    );
    assert!(matches!(
        failure.error,
        RuntimeHostError::Storage(RuntimeError::UnsupportedRuntime(_))
    ));
    assert!(
        failure.report.issues.is_empty(),
        "host lookup gaps cannot become caught native block exceptions"
    );
    assert_eq!(
        reader.pos(),
        bytes.len() - 4,
        "fatal host gap cannot run the later native End phase"
    );
    assert_eq!(component.borrow().fluid().unwrap().fluids.len(), 2);
    assert_eq!(component.borrow().fluid().unwrap().amount_cache, 2.0);
    let mut retired = failure.report.retired;
    detach_component_for_transfer(&actual, ComponentType::FluidContainer, &mut retired).unwrap();
}

struct MissingFluidPointer(BasicContext);
impl EntityContext for MissingFluidPointer {
    fn attribute_kind(&self, id: i16) -> Option<pz_entity::AttributeKind> {
        self.0.attribute_kind(id)
    }

    fn fluid_by_name(&self, name: &str) -> pz_entity::Lookup<pz_entity::FluidInfo> {
        self.0.fluid_by_name(name)
    }

    fn fluid_properties(
        &self,
        _: &pz_entity::FluidRef,
    ) -> pz_entity::Lookup<Option<pz_entity::fluids::SealedFluidProperties>> {
        // Resolve the body first, then exercise the actual missing native
        // Fluid pointer branch. A present Fluid's null properties field is
        // instead Found(None), which remains a successful native no-op.
        pz_entity::Lookup::Missing
    }
}

#[test]
fn native_cache_null_fluid_exception_is_caught_and_forces_component_end() {
    let component =
        RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer::default()))
            .unwrap();
    let actual = actual_with(&component);
    let context = MissingFluidPointer(known_fluids(true));
    let mut host = ItemComponentHost::new(&actual, &context);
    let bytes = overfilled_entity_bytes(4);
    let mut reader = pz_entity::io::Reader::new(&bytes);
    let report = load_native_with(
        &mut reader,
        249,
        &mut host,
        RuntimeLoadOptions {
            collect_loaded: true,
        },
    )
    .unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(report.issues.len(), 1);
    assert_eq!(
        report.issues[0].phase,
        pz_entity::runtime_load::RuntimeLoadPhase::LoadComponent
    );
    assert_eq!(
        report.issues[0].exception,
        EntityError("NullPointerException".into())
    );
    assert_eq!(reader.pos(), bytes.len());
    assert_eq!(component.borrow().fluid().unwrap().amount_cache, 2.0);
    assert!(Rc::ptr_eq(
        &actual.borrow().get_fluid_ref().unwrap().unwrap(),
        &component
    ));
    let mut retired = report.retired;
    detach_component_for_transfer(&actual, ComponentType::FluidContainer, &mut retired).unwrap();
}

fn factory_prefix_observer(item: &ItemRef) {
    assert_eq!(
        item.try_borrow().unwrap().id,
        0,
        "actual owner must exist before later id RNG draw"
    );
    OBSERVED.with(|count| count.set(count.get() + 1));
}

#[test]
fn ordinary_script_factory_reuses_actual_pool_ref_and_preserves_fields_script_does_not_write() {
    use crate::component::EntityCodec;
    let manager = pz_scripts::ScriptManager::new();
    manager
        .registries
        .get("item_type")
        .unwrap()
        .borrow_mut()
        .register_id("test:normal", true)
        .unwrap();
    manager
        .statics
        .items
        .borrow_mut()
        .constants
        .insert("item_type", "NORMAL", "test:normal");
    manager.parse_script(pz_scripts::ScriptLoadMode::Init,
        "module Base { item ActualPoolBottle { ItemType = test:normal, Weight = 0.1, component FluidContainer { Capacity = 7.25, RainCatcher = 0.25, } } }").unwrap();
    manager
        .get_module("Base")
        .unwrap()
        .bucket(pz_scripts::ScriptType::Item)
        .load_scripts(&manager, pz_scripts::ScriptLoadMode::Init);
    let script = manager
        .find_item("Base.ActualPoolBottle", false)
        .unwrap()
        .downcast::<pz_scripts::objects::items::Item>()
        .unwrap();
    script.borrow_mut().entity.set_registry_id(73).unwrap();
    struct LiveEnv<'a> {
        codec: EntityCodec<'a>,
        draws: RefCell<Vec<i32>>,
    }
    impl ItemEnv for LiveEnv<'_> {
        fn components(&self) -> &dyn ComponentCodec {
            &self.codec
        }
        fn rand_next(&self, bound: i32) -> i32 {
            self.draws.borrow_mut().push(bound);
            0
        }
    }
    let context = BasicContext::default();
    let env = LiveEnv {
        codec: EntityCodec { ctx: &context },
        draws: RefCell::new(Vec::new()),
    };
    let old_observer = shared::install_item_observer(Some(factory_prefix_observer));
    OBSERVED.with(|count| count.set(0));
    let first = crate::factory::create_item_by_type_ref(
        &env,
        &manager,
        "Base.ActualPoolBottle",
        1.0,
        true,
        &|_| None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        OBSERVED.with(Cell::get),
        3,
        "allocation, fluid insertion, script insertion are observed before owner cycles"
    );
    assert_eq!(*env.draws.borrow(), [i32::MAX, 2146250223]);
    let fluid = first.borrow().get_fluid_ref().unwrap().unwrap();
    let script_component = first
        .borrow()
        .components
        .get_ref(ComponentType::Script)
        .unwrap()
        .unwrap();
    assert!(script_component
        .borrow()
        .script_cache_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&script)));
    let mut first_host = ItemComponentHost::new(&first, &context);
    let mut retired = RetiredComponents::new();
    for component in [&fluid, &script_component] {
        let removed = first_host.remove(component, &mut retired).unwrap().unwrap();
        first_host
            .release_component(&removed, &mut retired)
            .unwrap();
    }
    assert_eq!(
        fluid.borrow().fluid().unwrap().capacity,
        1.0,
        "actual release reset is visible to retained handle"
    );
    let color = fluid.borrow().fluid_color_slot().unwrap().clone();
    fluid.borrow_mut().fluid_runtime_mut().unwrap().temperature = 88.0;
    env.draws.borrow_mut().clear();
    OBSERVED.with(|count| count.set(0));
    let second = crate::factory::create_item_by_type_ref(
        &env,
        &manager,
        "Base.ActualPoolBottle",
        1.0,
        true,
        &|_| None,
    )
    .unwrap()
    .unwrap();
    shared::install_item_observer(old_observer);
    assert_eq!(OBSERVED.with(Cell::get), 3);
    assert_eq!(*env.draws.borrow(), [i32::MAX, 2146250223]);
    assert!(Rc::ptr_eq(
        &fluid,
        &second.borrow().get_fluid_ref().unwrap().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &script_component,
        &second
            .borrow()
            .components
            .get_ref(ComponentType::Script)
            .unwrap()
            .unwrap()
    ));
    assert_eq!(fluid.borrow().fluid().unwrap().capacity, 7.25);
    assert_eq!(
        fluid.borrow().fluid_runtime().unwrap().temperature,
        88.0,
        "readFromScript does not reset temperature of an already reset pooled object"
    );
    assert!(Rc::ptr_eq(
        &color,
        fluid.borrow().fluid_color_slot().unwrap()
    ));
    let mut second_host = ItemComponentHost::new(&second, &context);
    for component in [&fluid, &script_component] {
        if let Some(component) = second_host.remove(component, &mut retired).unwrap() {
            retired.push_component(component);
        }
    }
}

fn sharpness_context() -> (BasicContext, pz_entity::tables::AttributeTypeRef) {
    use pz_entity::tables::{
        AttributeTypeInfo, InitialValue, NumericAttributeBounds, NumericAttributeValidation,
    };
    let mut context = BasicContext::default();
    let kind = context.attributes.insert_with_validation(
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
    (context, kind)
}

fn actual_attributes_script() -> (
    pz_scripts::ScriptManager,
    Rc<RefCell<pz_scripts::objects::items::Item>>,
) {
    actual_attributes_script_from(
        "module Base { item ActualAttributes { ItemType = test:normal, Sharpness = 1.0, } }",
    )
}

fn actual_attributes_script_from(
    source: &str,
) -> (
    pz_scripts::ScriptManager,
    Rc<RefCell<pz_scripts::objects::items::Item>>,
) {
    let manager = pz_scripts::ScriptManager::new();
    manager
        .registries
        .get("item_type")
        .unwrap()
        .borrow_mut()
        .register_id("test:normal", true)
        .unwrap();
    manager
        .statics
        .items
        .borrow_mut()
        .constants
        .insert("item_type", "NORMAL", "test:normal");
    manager
        .parse_script(pz_scripts::ScriptLoadMode::Init, source)
        .unwrap();
    manager
        .get_module("Base")
        .unwrap()
        .bucket(pz_scripts::ScriptType::Item)
        .load_scripts(&manager, pz_scripts::ScriptLoadMode::Init);
    let script = manager
        .find_item("Base.ActualAttributes", false)
        .unwrap()
        .downcast::<pz_scripts::objects::items::Item>()
        .unwrap();
    script.borrow_mut().entity.set_registry_id(74).unwrap();
    (manager, script)
}

struct AttributesEnv<'a>(crate::component::EntityCodec<'a>);
impl ItemEnv for AttributesEnv<'_> {
    fn components(&self) -> &dyn ComponentCodec {
        &self.0
    }
}

fn actual_attributes_item(env: &AttributesEnv<'_>, manager: &pz_scripts::ScriptManager) -> ItemRef {
    crate::factory::create_item_by_type_ref(
        env,
        manager,
        "Base.ActualAttributes",
        1.0,
        true,
        &|_| None,
    )
    .unwrap()
    .unwrap()
}

fn actual_attributes_template(
    script: &Rc<RefCell<pz_scripts::objects::items::Item>>,
) -> ComponentRef {
    use pz_scripts::objects::entity::components::attributes::AttributesScript;
    let script = script.borrow();
    let attributes = script
        .entity
        .component::<AttributesScript>(ComponentType::Attributes)
        .unwrap();
    assert!(attributes.template_created());
    let actual = attributes
        .template_slot()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    actual
        .downcast::<RefCell<RuntimeComponent>>()
        .ok()
        .expect("script cache must retain the actual pooled AttributeContainer component")
}

fn forbidden_debug_query() -> HostResult<bool> {
    Err(RuntimeHostError::Borrowed("unexpected Core.debug query"))
}

struct RestoreDebugQuery(Option<ComponentDebugQuery>);
impl Drop for RestoreDebugQuery {
    fn drop(&mut self) {
        install_component_debug_query(self.0);
    }
}

#[test]
fn actual_attributes_factory_copies_instances_from_one_retained_native_template() {
    use pz_entity::components::attributes::AttributeValue;
    let (manager, script) = actual_attributes_script();
    let (context, kind) = sharpness_context();
    let env = AttributesEnv(crate::component::EntityCodec { ctx: &context });
    let _debug = RestoreDebugQuery(install_component_debug_query(Some(forbidden_debug_query)));
    let first = actual_attributes_item(&env, &manager);
    let template = actual_attributes_template(&script);
    let second = actual_attributes_item(&env, &manager);
    assert!(Rc::ptr_eq(&template, &actual_attributes_template(&script)));
    let first_component = first.borrow().get_attributes_ref().unwrap().unwrap();
    let second_component = second.borrow().get_attributes_ref().unwrap().unwrap();
    assert!(!Rc::ptr_eq(&first_component, &template));
    assert!(!Rc::ptr_eq(&first_component, &second_component));
    assert!(first_component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&first)));
    let first_instance = first_component
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(9)
        .unwrap()
        .unwrap()
        .clone();
    let second_instance = second_component
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(9)
        .unwrap()
        .unwrap()
        .clone();
    let template_instance = template
        .borrow()
        .attributes()
        .unwrap()
        .get_ref(9)
        .unwrap()
        .unwrap()
        .clone();
    assert!(!Rc::ptr_eq(&first_instance, &second_instance));
    assert!(!Rc::ptr_eq(&first_instance, &template_instance));
    let mut retired = RetiredComponents::new();
    first_component
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&kind, AttributeValue::Float(0.25), &mut retired)
        .unwrap();
    assert_eq!(
        first.borrow().try_get_attribute_by_id(9).unwrap(),
        Some(AttributeValue::Float(0.25))
    );
    assert_eq!(
        second.borrow().try_get_attribute_by_id(9).unwrap(),
        Some(AttributeValue::Float(1.0))
    );
    assert_eq!(
        template
            .borrow()
            .attributes()
            .unwrap()
            .get_value(9)
            .unwrap(),
        Some(AttributeValue::Float(1.0))
    );
    let guard = first_instance.borrow_mut();
    assert!(
        first.borrow().try_get_attribute_by_id(9).is_err(),
        "borrowed actual instance cannot become a missing attribute"
    );
    drop(guard);
    for item in [&first, &second] {
        for kind in [ComponentType::Attributes, ComponentType::Script] {
            detach_component_for_transfer(item, kind, &mut retired).unwrap();
        }
    }
}

#[test]
fn actual_attribute_load_inserts_initial_instance_before_truncated_value() {
    use pz_entity::components::attributes::AttributeValue;
    let (manager, _script) = actual_attributes_script();
    let (context, kind) = sharpness_context();
    let env = AttributesEnv(crate::component::EntityCodec { ctx: &context });
    let item = actual_attributes_item(&env, &manager);
    let component = item.borrow().get_attributes_ref().unwrap().unwrap();
    let mut retired = RetiredComponents::new();
    component
        .borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&kind, AttributeValue::Float(0.25), &mut retired)
        .unwrap();
    let mut writer = pz_entity::io::Writer::new();
    writer.i8(1);
    let start = writer.block_start();
    writer.i16(ComponentType::Attributes.id());
    writer.i8(8);
    writer.i16(1);
    writer.i8(9); // Missing float, after native getOrAdd publishes its initial instance.
    writer.block_end(start);
    let mut reader = pz_entity::io::Reader::new(&writer.buf);
    let mut host = ItemComponentHost::new(&item, &context);
    let report = load_native_with(
        &mut reader,
        249,
        &mut host,
        RuntimeLoadOptions {
            collect_loaded: true,
        },
    )
    .unwrap();
    assert_eq!(report.issues.len(), 1);
    assert!(report.loaded.is_empty());
    assert_eq!(reader.pos(), writer.buf.len());
    assert!(Rc::ptr_eq(
        &component,
        &item.borrow().get_attributes_ref().unwrap().unwrap()
    ));
    assert_eq!(
        item.borrow().try_get_attribute_by_id(9).unwrap(),
        Some(AttributeValue::Float(1.0)),
        "native inserts the initial-valued actual instance before its scalar read fails"
    );
    retired.append(report.retired);
    for kind in [ComponentType::Attributes, ComponentType::Script] {
        detach_component_for_transfer(&item, kind, &mut retired).unwrap();
    }
}

fn observe_loaded_attribute_prefix(item: &ItemRef) {
    let component = item
        .try_borrow_mut()
        .expect("load observation must end the receiver guard")
        .get_attributes_ref()
        .unwrap()
        .unwrap();
    let body = component
        .try_borrow_mut()
        .expect("load observation must end the component guard");
    assert!(body
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(item)));
    let instance = body
        .attributes()
        .unwrap()
        .get_ref(9)
        .unwrap()
        .unwrap()
        .clone();
    drop(body);
    let instance = instance
        .try_borrow_mut()
        .expect("load observation must end the new actual instance guard");
    LOADED_ATTRIBUTES.with(|values| values.borrow_mut().push(instance.value_snapshot()));
}

struct RestoreItemObserver(Option<shared::ItemObserver>);
impl Drop for RestoreItemObserver {
    fn drop(&mut self) {
        shared::install_item_observer(self.0);
    }
}

#[test]
fn existing_attributes_load_observes_new_instances_after_guards_on_success_and_failure() {
    use pz_entity::components::attributes::AttributeValue;
    let (manager, _script) = actual_attributes_script();
    let (context, _kind) = sharpness_context();
    let env = AttributesEnv(crate::component::EntityCodec { ctx: &context });
    let item = actual_attributes_item(&env, &manager);
    LOADED_ATTRIBUTES.with(|values| values.borrow_mut().clear());
    let observer = RestoreItemObserver(shared::install_item_observer(Some(
        observe_loaded_attribute_prefix,
    )));
    let mut host = ItemComponentHost::new(&item, &context);
    let mut retired = RetiredComponents::new();
    for complete in [true, false] {
        let mut writer = pz_entity::io::Writer::new();
        writer.i8(1);
        let start = writer.block_start();
        writer.i16(ComponentType::Attributes.id());
        writer.i8(8);
        writer.i16(1);
        writer.i8(9);
        if complete {
            writer.f32(0.75);
        }
        writer.block_end(start);
        let mut reader = pz_entity::io::Reader::new(&writer.buf);
        let report = load_native_with(
            &mut reader,
            249,
            &mut host,
            RuntimeLoadOptions {
                collect_loaded: true,
            },
        )
        .unwrap();
        assert_eq!(report.issues.len(), usize::from(!complete));
        assert_eq!(reader.pos(), writer.buf.len());
        retired.append(report.retired);
    }
    assert_eq!(
        LOADED_ATTRIBUTES.with(|values| values.borrow().clone()),
        [AttributeValue::Float(0.75), AttributeValue::Float(1.0)],
        "both the completed body and its failed getOrAdd prefix must be adopted before returning"
    );
    drop(observer);
    for kind in [ComponentType::Attributes, ComponentType::Script] {
        detach_component_for_transfer(&item, kind, &mut retired).unwrap();
    }
}

struct RestoreNoticeLogger(Option<NativeNoticeLogger>);
impl Drop for RestoreNoticeLogger {
    fn drop(&mut self) {
        install_native_notice_logger(self.0);
    }
}

fn record_factory_notice(notice: &pz_entity::runtime_fluid::RuntimeNativeNotice) -> HostResult<()> {
    record_native_notice(notice)?;
    FACTORY_NOTICES.with(|notices| notices.borrow_mut().push(notice.clone()));
    Ok(())
}

#[test]
fn native_component_factory_exception_keeps_partial_item_and_continues_rng_but_host_fault_does_not()
{
    use pz_entity::components::attributes::AttributeValue;
    use pz_entity::runtime_fluid::RuntimeNativeNotice;
    struct RecordingEnv<'a> {
        codec: crate::component::EntityCodec<'a>,
        draws: RefCell<Vec<i32>>,
    }
    impl ItemEnv for RecordingEnv<'_> {
        fn components(&self) -> &dyn ComponentCodec {
            &self.codec
        }
        fn rand_next(&self, bound: i32) -> i32 {
            self.draws.borrow_mut().push(bound);
            0
        }
    }
    let _observer = RestoreItemObserver(shared::install_item_observer(Some(notice_observer)));
    let _logger = RestoreNoticeLogger(install_native_notice_logger(Some(record_factory_notice)));
    for native_exception in [true, false] {
        let (manager, script) = actual_attributes_script_from(
            "module Base { item ActualAttributes { ItemType = test:normal, component Attributes { Sharpness = 0.5, } component FluidContainer { Capacity = 7.25, } } }",
        );
        NOTICE_SCRIPT.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(&script)));
        NOTICES.with(|notices| notices.borrow_mut().clear());
        FACTORY_NOTICES.with(|notices| notices.borrow_mut().clear());
        let (mut context, _kind) = sharpness_context();
        context.fluids = known_fluids(false).fluids;
        let env = RecordingEnv {
            codec: crate::component::EntityCodec { ctx: &context },
            draws: RefCell::new(Vec::new()),
        };
        let fluid =
            pz_entity::runtime_pool::obtain_component(ComponentType::FluidContainer).unwrap();
        let mut retired = RetiredComponents::new();
        pz_entity::runtime_pool::release_detached_component(&fluid, &mut retired).unwrap();
        {
            // A retained real pooled handle exposes this actual mutated body
            // on the next factory obtain. Null fluid reaches the authenticated
            // native cache NPE; known fluid with unknown properties is a
            // distinct fatal host capability gap, never a caught exception.
            let mut component = fluid.borrow_mut();
            let data = component.fluid_mut().unwrap();
            data.fluids.push(pz_entity::FluidInstance {
                fluid: (!native_exception)
                    .then(|| pz_entity::FluidRef::Modded("Example.Red".into())),
                custom_color: None,
                amount: 1.0,
            });
            data.cache_invalidated = true;
        }
        let result = crate::factory::create_item_by_type_ref(
            &env,
            &manager,
            "Base.ActualAttributes",
            1.0,
            true,
            &|_| None,
        );
        let item = if native_exception {
            let item = result.unwrap().unwrap();
            assert_eq!(
                *env.draws.borrow(),
                [i32::MAX, 2146250223],
                "native internal catch returns the partial item to later InstanceItem phases"
            );
            assert_ne!(item.borrow().id, 0);
            let notices = FACTORY_NOTICES.with(|notices| notices.borrow().clone());
            assert_eq!(notices.len(), 1);
            assert!(
                matches!(&notices[0], RuntimeNativeNotice::ExceptionLogger(message) if message.contains("NullPointerException"))
            );
            item
        } else {
            let mut failure = result.unwrap_err();
            assert!(matches!(
                failure.error,
                RuntimeHostError::Storage(RuntimeError::UnsupportedRuntime(_))
            ));
            assert!(
                env.draws.borrow().is_empty(),
                "host gaps stop before visual/id RNG"
            );
            assert!(FACTORY_NOTICES.with(|notices| notices.borrow().is_empty()));
            let item = failure.item.take().unwrap();
            assert_eq!(item.borrow().id, 0);
            retired.append(failure.retired);
            item
        };
        let attributes = item.borrow().get_attributes_ref().unwrap().unwrap();
        assert!(attributes
            .borrow()
            .owner_slot()
            .unwrap()
            .ptr_eq(&EntityOwner::from_ref(&item)));
        assert_eq!(
            item.borrow().try_get_attribute_by_id(9).unwrap(),
            Some(AttributeValue::Float(0.5))
        );
        assert!(item.borrow().get_fluid_ref().unwrap().is_none());
        assert!(item
            .borrow()
            .components
            .get_ref(ComponentType::Script)
            .unwrap()
            .is_none());
        detach_component_for_transfer(&item, ComponentType::Attributes, &mut retired).unwrap();
        drop(retired);
        drain_retired_components();
    }
}

fn notice_observer(item: &ItemRef) {
    NOTICE_ITEM.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(item)));
}

fn record_native_notice(notice: &pz_entity::runtime_fluid::RuntimeNativeNotice) -> HostResult<()> {
    let item = NOTICE_ITEM.with(|slot| slot.borrow().as_ref().unwrap().upgrade().unwrap());
    let script = NOTICE_SCRIPT.with(|slot| slot.borrow().as_ref().unwrap().upgrade().unwrap());
    assert_eq!(
        item.try_borrow_mut().unwrap().id,
        0,
        "notices precede later constructor RNG and no item guard is held"
    );
    assert!(
        script.try_borrow_mut().is_ok(),
        "factory script guard must end even on failure"
    );
    match notice {
        pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralLog(message)
        | pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralError(message)
        | pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralException(message)
        | pz_entity::runtime_fluid::RuntimeNativeNotice::ExceptionLogger(message) => {
            NOTICES.with(|messages| messages.borrow_mut().push(message.clone()))
        }
    }
    Ok(())
}

#[test]
fn native_factory_warning_prefix_is_emitted_outside_guards_on_success_and_later_error() {
    use crate::component::EntityCodec;
    let manager = pz_scripts::ScriptManager::new();
    manager
        .registries
        .get("item_type")
        .unwrap()
        .borrow_mut()
        .register_id("test:normal", true)
        .unwrap();
    manager
        .statics
        .items
        .borrow_mut()
        .constants
        .insert("item_type", "NORMAL", "test:normal");
    manager.parse_script(pz_scripts::ScriptLoadMode::Init,
        "module Base { item WarnSuccess { ItemType = test:normal, component FluidContainer { Capacity = 1, fluids { fluid = Example.Missing:1, } } } item WarnFailure { ItemType = test:normal, component FluidContainer { Capacity = 1, fluids { fluid = Example.Missing:1, fluid = Example.Red:1, fluid = Example.Blue:1, } } } }").unwrap();
    manager
        .get_module("Base")
        .unwrap()
        .bucket(pz_scripts::ScriptType::Item)
        .load_scripts(&manager, pz_scripts::ScriptLoadMode::Init);
    let context = known_fluids(false);
    struct NoticeEnv<'a>(EntityCodec<'a>);
    impl ItemEnv for NoticeEnv<'_> {
        fn components(&self) -> &dyn ComponentCodec {
            &self.0
        }
    }
    let env = NoticeEnv(EntityCodec { ctx: &context });
    let old_observer = shared::install_item_observer(Some(notice_observer));
    let old_logger = install_native_notice_logger(Some(record_native_notice));
    for (name, fails) in [("WarnSuccess", false), ("WarnFailure", true)] {
        let full = format!("Base.{name}");
        let script = manager
            .find_item(&full, false)
            .unwrap()
            .downcast::<pz_scripts::objects::items::Item>()
            .unwrap();
        NOTICE_SCRIPT.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(&script)));
        NOTICES.with(|messages| messages.borrow_mut().clear());
        let built =
            crate::factory::create_item_by_type_ref(&env, &manager, &full, 1.0, true, &|_| None);
        assert_eq!(
            NOTICES.with(|messages| messages.borrow().clone()),
            [format!(
                "FluidContainer -> initial Fluid 'Example.Missing' not found! [{name}]"
            )]
        );
        match built {
            Ok(Some(item)) => {
                assert!(!fails);
                let mut retired = RetiredComponents::new();
                for kind in [ComponentType::FluidContainer, ComponentType::Script] {
                    detach_component_for_transfer(&item, kind, &mut retired).unwrap();
                }
            }
            Err(failure) => {
                assert!(fails);
                assert!(matches!(
                    failure.error,
                    RuntimeHostError::Storage(RuntimeError::UnsupportedRuntime(_))
                ));
                failure.into_error_and_retire();
            }
            Ok(None) => panic!("registered script disappeared"),
        }
    }
    install_native_notice_logger(old_logger);
    shared::install_item_observer(old_observer);
    NOTICE_SCRIPT.with(|slot| slot.borrow_mut().take());
    NOTICE_ITEM.with(|slot| slot.borrow_mut().take());
    drain_retired_components();
}

#[test]
fn opaque_factory_only_accepts_proven_component_free_definition() {
    let item = crate::env::create_from_template_ref(&PlainEnv, value().template, 1).unwrap();
    assert!(item.borrow().components.is_empty());
}

#[test]
fn fieldless_context_menu_factory_uses_actual_pool_and_current_script_owner() {
    let (manager, script) = actual_attributes_script_from(
        "module Base { item ActualAttributes { ItemType = test:normal, component ContextMenuConfig { } } }",
    );
    let context = BasicContext::default();
    let env = AttributesEnv(crate::component::EntityCodec { ctx: &context });
    let first = actual_attributes_item(&env, &manager);
    let component = first
        .borrow()
        .components
        .get_ref(ComponentType::ContextMenuConfig)
        .unwrap()
        .unwrap();
    assert!(matches!(
        component.borrow().value_snapshot().unwrap(),
        Component::Empty(ComponentType::ContextMenuConfig)
    ));
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&first)));
    let first_script = first
        .borrow()
        .components
        .get_ref(ComponentType::Script)
        .unwrap()
        .unwrap();
    assert!(first_script
        .borrow()
        .script_cache_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&script)));
    let mut retired = RetiredComponents::new();
    let mut first_host = ItemComponentHost::new(&first, &context);
    let removed = first_host
        .remove(&component, &mut retired)
        .unwrap()
        .unwrap();
    first_host
        .release_component(&removed, &mut retired)
        .unwrap();
    assert!(component.borrow().owner_slot().is_none());
    let second = actual_attributes_item(&env, &manager);
    let reused = second
        .borrow()
        .components
        .get_ref(ComponentType::ContextMenuConfig)
        .unwrap()
        .unwrap();
    assert!(Rc::ptr_eq(&component, &reused));
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&second)));
    // No entries are copied into the fieldless component. Exposing its real
    // script-owned ArrayList is a separate, still unsupported operation.
    for item in [&first, &second] {
        for kind in [ComponentType::ContextMenuConfig, ComponentType::Script] {
            detach_component_for_transfer(item, kind, &mut retired).unwrap();
        }
    }
}

#[test]
fn durability_factory_reuses_native_body_and_overwrites_material_with_script_null() {
    use pz_scripts::objects::entity::components::combat::MaterialType;
    let (first_manager, _) = actual_attributes_script_from(
        "module Base { item ActualAttributes { ItemType = test:normal, component Durability { MaxHitPoints = 7.25, Material = Glass, } } }",
    );
    let (second_manager, _) = actual_attributes_script_from(
        "module Base { item ActualAttributes { ItemType = test:normal, component Durability { MaxHitPoints = 2.5, } } }",
    );
    let context = BasicContext::default();
    let env = AttributesEnv(crate::component::EntityCodec { ctx: &context });
    let first = actual_attributes_item(&env, &first_manager);
    let component = first
        .borrow()
        .components
        .get_ref(ComponentType::Durability)
        .unwrap()
        .unwrap();
    {
        let body = component.borrow();
        let durability = body.durability().unwrap();
        assert_eq!(durability.current_hit_points, 7.25);
        assert_eq!(durability.max_hit_points, 7.25);
        assert_eq!(durability.material, Some(MaterialType::Glass));
    }
    {
        let mut body = component.borrow_mut();
        let durability = body.durability_mut().unwrap();
        durability.current_hit_points = -3.0;
        durability.max_hit_points = 99.0;
        durability.material = Some(MaterialType::Metal);
    }
    let mut retired = RetiredComponents::new();
    let mut first_host = ItemComponentHost::new(&first, &context);
    let removed = first_host
        .remove(&component, &mut retired)
        .unwrap()
        .unwrap();
    first_host
        .release_component(&removed, &mut retired)
        .unwrap();
    {
        let body = component.borrow();
        let durability = body.durability().unwrap();
        assert_eq!(durability.current_hit_points, -3.0);
        assert_eq!(durability.max_hit_points, 99.0);
        assert_eq!(durability.material, Some(MaterialType::Metal));
    }
    let second = actual_attributes_item(&env, &second_manager);
    let reused = second
        .borrow()
        .components
        .get_ref(ComponentType::Durability)
        .unwrap()
        .unwrap();
    assert!(Rc::ptr_eq(&component, &reused));
    {
        let body = component.borrow();
        let durability = body.durability().unwrap();
        assert_eq!(durability.current_hit_points, 2.5);
        assert_eq!(durability.max_hit_points, 2.5);
        assert_eq!(
            durability.material, None,
            "the native script null must overwrite the retained pooled value"
        );
        assert!(matches!(
            body.value_snapshot(),
            Err(RuntimeError::UnsupportedRuntime(_))
        ));
    }
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&second)));
    for item in [&first, &second] {
        for kind in [ComponentType::Durability, ComponentType::Script] {
            detach_component_for_transfer(item, kind, &mut retired).unwrap();
        }
    }
}

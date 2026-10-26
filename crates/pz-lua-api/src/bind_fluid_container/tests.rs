use super::*;
use pz_entity::{Component, FluidContainer, FluidInstance, FluidRef};
use pz_inventory::shared::{self, ItemRef};
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_kahlua::TableRef;

struct Restore {
    env: Rc<dyn crate::bind_items::ItemLuaEnv>,
    item: Option<pz_inventory::shared::ItemObserver>,
    object: Option<fn(&pz_world::object_slot::IsoObjectRef)>,
    fluid: Option<pz_inventory::component_host::ItemFluidObserver>,
    white: Rgba,
}

impl Drop for Restore {
    fn drop(&mut self) {
        crate::bind_items::install_item_env(self.env.clone());
        shared::install_item_observer(self.item);
        pz_world::object_slot::install_object_observer(self.object);
        pz_inventory::component_host::install_item_fluid_observer(self.fluid);
        *pz_entity::runtime_fluid::fluid_default_color_slot().borrow_mut() = self.white;
        AUTHORED_OWNER.with(|owner| *owner.borrow_mut() = None);
        pz_inventory::component_host::drain_retired_components();
        crate::component_gc::retry_pending();
        let _ = pz_kahlua::gc::collect_tables();
    }
}

fn context() -> pz_entity::BasicContext {
    let mut registry = pz_entity::FluidRegistry::from_definitions(&["Water".into()], []);
    // An explicit authored registry entry; this is not a substituted answer
    // for an unavailable native property table.
    assert!(registry.set_properties("Water", Some(Default::default())));
    pz_entity::BasicContext {
        fluids: Some(registry),
        ..Default::default()
    }
}

fn setup() -> (Restore, Thread, pz_entity::BasicContext) {
    let restore = Restore {
        env: crate::bind_items::item_env(),
        item: shared::install_item_observer(Some(crate::component_gc::observe_item)),
        object: pz_world::object_slot::install_object_observer(Some(
            crate::component_gc::observe_object,
        )),
        fluid: pz_inventory::component_host::install_item_fluid_observer(Some(
            crate::bind_item_container::on_fluid_update,
        )),
        white: *pz_entity::runtime_fluid::fluid_default_color_slot().borrow(),
    };
    let ctx = context();
    let mut env = crate::bind_items::GameItemEnv::new(None);
    env.entity = Some(Rc::new(ctx.clone()));
    crate::bind_items::install_item_env(Rc::new(env));
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_misc_classes::register(&mut thread, None);
    crate::bind_items::register(&mut thread);
    register(&mut thread);
    // BaseLib registers error/pcall; the installed Lua stdlib normally adds
    // assert. Keep this fixture self-contained, as the other binding suites do.
    run(
        &mut thread,
        "function assert(value, message) if not value then error(message or 'assertion failed') end return value end",
    )
    .unwrap();
    (restore, thread, ctx)
}

fn fluid_item(ctx: &dyn pz_entity::EntityContext, name: &str) -> (ItemRef, ComponentRef) {
    let body = FluidContainer {
        capacity: 4.0,
        fluids: vec![FluidInstance {
            fluid: Some(FluidRef::Type(
                pz_scripts::objects::definitions::FluidType::Water.id(),
            )),
            amount: 1.0,
            custom_color: Some([0.25, 0.5, 0.75]),
        }],
        ..Default::default()
    };
    let component =
        pz_entity::RuntimeComponent::new_unowned(Component::FluidContainer(body)).unwrap();
    let outcome = component
        .borrow_mut()
        .fluid_recalculate_data(ctx, true)
        .unwrap();
    assert!(
        outcome.owner.is_none(),
        "constructor cache initialization precedes owner publication"
    );
    let mut item = InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", name)),
        1,
    );
    item.components
        .runtime_mut()
        .push_ref_if_absent(component.clone())
        .unwrap();
    let item = shared::try_new_item(item).unwrap();
    (item, component)
}

fn run(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("actual-fluid-binding-test"),
        thread.globals().clone(),
    )
    .unwrap();
    thread.call(&function, &[])
}

fn expose_item(thread: &Thread, name: &str, item: &ItemRef) {
    thread
        .globals()
        .set_str(name, crate::bind_items::item_value(item));
}

#[test]
fn actual_item_getter_owner_and_aliases_keep_one_component_userdata() {
    let (_restore, mut thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "Identity");
    expose_item(&thread, "item", &item);
    expose_item(&thread, "alias", &item);
    run(
        &mut thread,
        r#"
        held = item:getFluidContainer()
        assert(held == alias:getFluidContainer())
        assert(held:getOwner() == item)
        assert(held:getCapacity() == 4 and held:getAmount() == 1)
        assert(not held:isEmpty() and not held:isFull())
        assert(held:getFreeCapacity() == 3 and held:getFilledRatio() == 0.25)
    "#,
    )
    .unwrap();
    component.borrow_mut().fluid_mut().unwrap().capacity = 8.0;
    run(
        &mut thread,
        "assert(held:getCapacity() == 8 and held:getFilledRatio() == 0.125)",
    )
    .unwrap();
    assert!(Rc::ptr_eq(
        &component,
        &item.borrow().get_fluid_ref().unwrap().unwrap()
    ));
}

#[test]
fn retained_component_userdata_keeps_identity_after_actual_item_owner_transfer() {
    let (_restore, mut thread, ctx) = setup();
    let (source, component) = fluid_item(&ctx, "Source");
    let target = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Target")),
        2,
    ))
    .unwrap();
    expose_item(&thread, "source", &source);
    expose_item(&thread, "target", &target);
    run(
        &mut thread,
        "held = source:getFluidContainer(); assert(held:getOwner() == source)",
    )
    .unwrap();
    let mut retired = pz_entity::RetiredComponents::new();
    let mut host = pz_inventory::component_host::ItemComponentHost::new(&target, &ctx);
    assert!(host.add(&component, &mut retired).unwrap());
    host.connect(&mut retired).unwrap();
    run(
        &mut thread,
        r#"
        assert(source:getFluidContainer() == nil)
        assert(held == target:getFluidContainer())
        assert(held:getOwner() == target and held:getAmount() == 1)
    "#,
    )
    .unwrap();
    assert!(source.borrow().get_fluid_ref().unwrap().is_none());
    assert!(Rc::ptr_eq(
        &target.borrow().get_fluid_ref().unwrap().unwrap(),
        &component
    ));
    drop(retired);
}

#[test]
fn encoded_component_error_is_distinct_from_a_genuinely_absent_component() {
    let (_restore, mut thread, _) = setup();
    let absent = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Absent")),
        1,
    ))
    .unwrap();
    let mut encoded = InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Encoded")),
        2,
    );
    encoded.components = pz_inventory::item_components::ItemComponents::from_encoded_blobs(
        vec![pz_inventory::component::ComponentBlob {
            type_id: pz_entity::ComponentType::FluidContainer.id(),
            data: vec![0, 0],
        }],
        249,
    )
    .unwrap();
    let encoded = shared::try_new_item(encoded).unwrap();
    expose_item(&thread, "absent", &absent);
    expose_item(&thread, "encoded", &encoded);
    run(
        &mut thread,
        r#"
        assert(absent:getFluidContainer() == nil)
        local ok, err = pcall(function() return encoded:getFluidContainer() end)
        assert(not ok and err ~= nil)
    "#,
    )
    .unwrap();
    assert!(matches!(
        encoded.borrow().get_fluid_ref(),
        Err(RuntimeError::NeedsPromotion { .. })
    ));
}

#[test]
fn actual_cached_color_is_mutable_canonical_and_interoperates_with_existing_color_class() {
    let (_restore, mut thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "Color");
    expose_item(&thread, "item", &item);
    let actual = component.borrow().fluid_color_slot().unwrap().clone();
    run(
        &mut thread,
        r#"
        local fluid = item:getFluidContainer()
        heldColor = fluid:getColor()
        assert(heldColor == fluid:getColor())
        heldColor:set(0.125, 0.25, 0.5, 0.75)
        assert(fluid:getColor() == heldColor and heldColor:getR() == 0.125)
        local copy = Color.new(heldColor)
        assert(copy ~= heldColor and copy:equals(heldColor))
        local immutable = ImmutableColor.new(heldColor)
        assert(immutable:getRedFloat() == 0.125)
        local result = Color.abgrToColor(0xFF204080, heldColor)
        assert(result == heldColor)
        Color.HSBtoRGB(0, 1, 1, heldColor)
        assert(heldColor:getR() == 1 and heldColor:getG() == 0 and heldColor:getB() == 0)
        assert(tostring(heldColor) == heldColor:toString())
    "#,
    )
    .unwrap();
    assert_eq!(*actual.borrow(), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        Rc::strong_count(&actual),
        3,
        "component, explicit raw color and ONE userdata slot; channel selectors are weak"
    );
    component.borrow_mut().fluid_mut().unwrap().fluids[0].custom_color = Some([0.5, 0.25, 0.125]);
    run(&mut thread, "assert(heldColor == item:getFluidContainer():getColor()); assert(heldColor:getG() == 0.25)").unwrap();
}

#[test]
fn borrowed_actual_color_returns_lua_errors_and_does_not_become_a_null_or_copied_color() {
    let (_restore, mut thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "BorrowedColor");
    expose_item(&thread, "item", &item);
    run(
        &mut thread,
        "heldColor = item:getFluidContainer():getColor()",
    )
    .unwrap();
    let actual = component.borrow().fluid_color_slot().unwrap().clone();
    let guard = actual.borrow_mut();
    run(
        &mut thread,
        r#"
        local a = pcall(function() return heldColor:getR() end)
        local b = pcall(function() return Color.new(heldColor) end)
        local c = pcall(function() return tostring(heldColor) end)
        assert(not a and not b and not c)
    "#,
    )
    .unwrap();
    drop(guard);
    run(
        &mut thread,
        "assert(heldColor == item:getFluidContainer():getColor())",
    )
    .unwrap();
}

#[test]
fn getter_can_return_same_identity_under_a_body_guard_but_body_methods_error() {
    let (_restore, mut thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "BorrowedBody");
    expose_item(&thread, "item", &item);
    run(&mut thread, "held = item:getFluidContainer()").unwrap();
    let guard = component.borrow_mut();
    run(
        &mut thread,
        r#"
        assert(held == item:getFluidContainer())
        assert(not pcall(function() return held:getAmount() end))
    "#,
    )
    .unwrap();
    drop(guard);
    run(&mut thread, "assert(held:getAmount() == 1)").unwrap();
}

#[test]
fn native_recalculation_dirties_immediate_bag_only_when_outermost_actual_parent_is_player() {
    let (_restore, _thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "NestedFluid");
    let actor = Rc::new(RefCell::new(
        crate::bind_character_sim::LuaGameCharacter::new(
            &pz_scripts::ScriptManager::new(),
            pz_characters::CharacterKind::Player,
        ),
    ));
    let inventory = actor.borrow().items.inventory.clone();
    let actor_value = Value::Object(Rc::new(crate::bind_character_sim::CharacterObject(
        actor.clone(),
    )));
    crate::bind_item_container::set_parent(&inventory, actor_value);
    let bag = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Container, "Base", "Bag")),
        2,
    ))
    .unwrap();
    shared::add_item(&inventory, &bag);
    let inside = shared::inventory_of(&bag.borrow()).unwrap();
    shared::add_item(&inside, &item);
    for container in [&inside, &inventory] {
        let mut body = container.borrow_mut();
        body.dirty = false;
        body.draw_dirty = false;
    }
    component.borrow_mut().fluid_mut().unwrap().fluids[0].amount = 2.5;
    assert_eq!(amount(&component).unwrap(), 2.5);
    assert!(inside.borrow().dirty && inside.borrow().draw_dirty);
    assert!(!inventory.borrow().dirty && !inventory.borrow().draw_dirty);
    inside.borrow_mut().dirty = false;
    inside.borrow_mut().draw_dirty = false;
    assert_eq!(amount(&component).unwrap(), 2.5);
    assert!(
        !inside.borrow().dirty && !inside.borrow().draw_dirty,
        "unchanged caches skip the owner phase"
    );
    inventory.borrow_mut().parent = None;
    shared::remove_item(&inside, &item);
    shared::remove_item(&inventory, &bag);
}

thread_local! {
    static AUTHORED_OWNER: RefCell<Option<ComponentRef>> = const { RefCell::new(None) };
}

fn change_cached_amount_after_data_phase(_: &ItemRef) -> pz_entity::runtime_load::HostResult<()> {
    if let Some(component) = AUTHORED_OWNER.with(|owner| owner.borrow_mut().take()) {
        let mut body = component.borrow_mut();
        let fluid = body.fluid_mut().unwrap();
        fluid.amount_cache = 0.75;
        fluid.capacity = 9.0;
    }
    Ok(())
}

#[test]
fn amount_reads_actual_cache_after_guard_free_owner_phase_without_extra_recalculation() {
    let (_restore, _thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "OwnerPrefix");
    AUTHORED_OWNER.with(|owner| *owner.borrow_mut() = Some(component.clone()));
    pz_inventory::component_host::install_item_fluid_observer(Some(
        change_cached_amount_after_data_phase,
    ));
    component
        .borrow_mut()
        .fluid_mut()
        .unwrap()
        .cache_invalidated = true;
    assert_eq!(amount(&component).unwrap(), 0.75, "native getAmount reads the cache after owner work, even when that work invalidated it again");
    assert_eq!(component.borrow().fluid().unwrap().capacity, 9.0);
    assert!(component.borrow().fluid().unwrap().cache_invalidated);
    assert!(component
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&pz_entity::EntityOwner::from_ref(&item)));
}

#[test]
fn free_capacity_captures_capacity_before_native_amount_owner_phase() {
    let (_restore, mut thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "CapacityPrefix");
    expose_item(&thread, "item", &item);
    AUTHORED_OWNER.with(|owner| *owner.borrow_mut() = Some(component.clone()));
    pz_inventory::component_host::install_item_fluid_observer(Some(
        change_cached_amount_after_data_phase,
    ));
    component
        .borrow_mut()
        .fluid_mut()
        .unwrap()
        .cache_invalidated = true;
    let values = run(
        &mut thread,
        "return item:getFluidContainer():getFreeCapacity()",
    )
    .unwrap();
    assert_eq!(
        values[0].as_num(),
        Some(3.25),
        "captured4.0 minus post-owner cached0.75; currentcapacity9 mustnotreplacecapturedvalue"
    );
}

#[test]
fn existing_core_color_and_actual_color_cache_share_the_canonical_typed_userdata() {
    let (_restore, _thread, _) = setup();
    let actual = Rc::new(RefCell::new([0.1, 0.2, 0.3, 0.4]));
    let existing = Rc::new(Color::from_actual(&actual));
    let first = existing_color_value(&existing);
    let second = color_value(&actual).unwrap();
    assert_eq!(first.ptr_id(), second.ptr_id());
    let Value::Object(object) = &second else {
        panic!("native Color expected")
    };
    assert!(object.as_any().is::<Color>());
    assert_eq!(
        Rc::strong_count(&actual),
        2,
        "the actual body has one explicit handle and one Color userdata owner"
    );
}

#[test]
fn mutable_static_white_alias_and_empty_reset_update_the_existing_final_color_allocation() {
    let (_restore, mut thread, ctx) = setup();
    let white = default_color_value();
    thread.globals().set_str("actualWhite", white);
    run(&mut thread, "actualWhite:set(0.125, 0.25, 0.5, 0.75)").unwrap();
    assert_eq!(
        *pz_entity::runtime_fluid::fluid_default_color_slot().borrow(),
        [0.125, 0.25, 0.5, 0.75]
    );
    let empty =
        pz_entity::RuntimeComponent::new_unowned(Component::FluidContainer(Default::default()))
            .unwrap();
    let actual = empty.borrow().fluid_color_slot().unwrap().clone();
    assert_eq!(
        *actual.borrow(),
        [0.125, 0.25, 0.5, 0.75],
        "native constructor copies current RGBA from static white"
    );
    let value = color_value(&actual).unwrap();
    thread.globals().set_str("retained", value);
    run(&mut thread, "actualWhite:set(0.75, 0.5, 0.25, 0.125)").unwrap();
    let outcome = empty.borrow_mut().fluid_empty_data(&ctx).unwrap();
    pz_inventory::component_host::notify_fluid_update(&outcome).unwrap();
    assert!(Rc::ptr_eq(
        &actual,
        empty.borrow().fluid_color_slot().unwrap()
    ));
    run(
        &mut thread,
        "assert(retained:getR() == 0.75 and retained:getAlphaFloat() == 0.125)",
    )
    .unwrap();
}

#[test]
fn native_color_null_destinations_keep_null_pointer_errors() {
    let (_restore, mut thread, ctx) = setup();
    // Invoke through Lua, which preserves the native exception class. A
    // direct host call to a NativeFunction deliberately wraps it in RuntimeException.
    let abgr = pz_kahlua::stdlib::load_string(
        "return Color.abgrToColor(0, nil)",
        Some("native-color-null-destination"),
        thread.globals().clone(),
    )
    .unwrap();
    let (_, _, error) = thread.pcall_ex(&abgr, &[]).unwrap_err();
    assert!(matches!(
        error,
        LuaError::Runtime {
            class: "java.lang.NullPointerException",
            ..
        }
    ));
    let (_item, component) = fluid_item(&ctx, "NullDestination");
    let color = cached_color(&component).unwrap();
    thread.globals().set_str("color", color);
    let interp = pz_kahlua::stdlib::load_string(
        "return color:interp(color, 0.5, nil)",
        Some("native-color-null-destination"),
        thread.globals().clone(),
    )
    .unwrap();
    let (_, _, error) = thread.pcall_ex(&interp, &[]).unwrap_err();
    assert!(matches!(
        error,
        LuaError::Runtime {
            class: "java.lang.NullPointerException",
            ..
        }
    ));
}

#[test]
fn owner_getter_reports_cold_borrow_error_and_retains_cached_identity_without_body_read() {
    let (_restore, mut thread, ctx) = setup();
    let (item, component) = fluid_item(&ctx, "ColdOwner");
    thread
        .globals()
        .set_str("held", fluid_value(&component).unwrap());
    let owner_guard = item.borrow_mut();
    run(
        &mut thread,
        "assert(not pcall(function() return held:getOwner() end))",
    )
    .unwrap();
    drop(owner_guard);
    expose_item(&thread, "item", &item);
    let owner_guard = item.borrow_mut();
    run(&mut thread, "assert(held:getOwner() == item)").unwrap();
    drop(owner_guard);
}

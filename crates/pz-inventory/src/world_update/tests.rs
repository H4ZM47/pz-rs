use super::*;
use crate::io::{Reader, Writer};
use crate::{ItemEnv, ItemTemplate};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

struct WorldState {
    index: i32,
    outside: Option<bool>,
}

struct Context {
    item: Weak<RefCell<InventoryItem>>,
    server: RefCell<bool>,
    client: RefCell<bool>,
    days: RefCell<i32>,
    calls: RefCell<Vec<&'static str>>,
}
impl Context {
    fn new(item: &ItemRef) -> Self {
        Self {
            item: Rc::downgrade(item),
            server: RefCell::new(false),
            client: RefCell::new(false),
            days: RefCell::new(-1),
            calls: RefCell::new(Vec::new()),
        }
    }
    fn phase(&self, name: &'static str) {
        if let Some(item) = self.item.upgrade() {
            assert!(item.try_borrow_mut().is_ok(), "inventory guard in {name}");
        }
        self.calls.borrow_mut().push(name);
    }
    fn world(&self, owner: &EntityOwner) -> HostResult<Rc<RefCell<WorldState>>> {
        owner
            .try_handle::<WorldState>()
            .ok_or(RuntimeHostError::InvalidHostState("test actual world type"))
    }
}
impl WorldItemUpdateContext for Context {
    fn is_server(&self) -> HostResult<bool> {
        self.phase("server");
        self.server
            .try_borrow()
            .map(|v| *v)
            .map_err(|_| RuntimeHostError::Borrowed("test server"))
    }
    fn is_client(&self) -> HostResult<bool> {
        self.phase("client");
        self.client
            .try_borrow()
            .map(|v| *v)
            .map_err(|_| RuntimeHostError::Borrowed("test client"))
    }
    fn rotten_food_removal_days(&self) -> HostResult<i32> {
        self.phase("rotten-days");
        self.days
            .try_borrow()
            .map(|v| *v)
            .map_err(|_| RuntimeHostError::Borrowed("test sandbox"))
    }
    fn world_object_index(&self, owner: &EntityOwner) -> HostResult<i32> {
        self.phase("index");
        let world = self.world(owner)?;
        let body = world
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("test world index"))?;
        Ok(body.index)
    }
    fn world_square_outside(&self, owner: &EntityOwner) -> HostResult<Option<bool>> {
        self.phase("outside");
        let world = self.world(owner)?;
        let body = world
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("test world square"))?;
        Ok(body.outside)
    }
}
fn item(class: ItemClass, ty: &str) -> ItemRef {
    crate::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(class, "Base", ty)),
        1,
    ))
    .unwrap()
}
fn link(item: &ItemRef, index: i32, outside: Option<bool>) -> Rc<RefCell<WorldState>> {
    let world = Rc::new(RefCell::new(WorldState { index, outside }));
    crate::world_item::set_world_item_ref(item, Some(&EntityOwner::from_ref(&world))).unwrap();
    crate::component_host::drain_retired_components();
    world
}
fn with_food(item: &ItemRef, f: impl FnOnce(&mut crate::item::FoodData)) {
    let mut body = item.borrow_mut();
    let ItemKind::Food(food) = &mut body.kind else {
        panic!("food fixture")
    };
    f(food);
}

#[test]
fn plain_bandage_constructor_branch_is_false_without_world_or_unneeded_sandbox() {
    let item = item(ItemClass::Normal, "Bandage");
    let cx = Context::new(&item);
    let _sandbox = cx.days.borrow_mut();
    assert_eq!(item.borrow().item_heat, 1.0);
    assert_eq!(item.borrow().drainable_heat, None);
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(false));
    assert_eq!(*cx.calls.borrow(), ["server", "client", "client"]);
    assert!(item.borrow().world_item_slot().is_none());
    assert!(item.borrow().table.is_none());
}

#[test]
fn base_heat_ordered_clamp_and_unordered_predicate_preserve_real_float_state() {
    let item = item(ItemClass::Normal, "Bandage");
    let cx = Context::new(&item);
    let nan = f32::from_bits(0x7fc0_0123);
    for (input, expected) in [(4.0, 3.0), (-1.0, 0.0), (-0.0, -0.0), (nan, nan)] {
        item.borrow_mut().set_item_heat(input);
        assert_eq!(item.borrow().get_item_heat().to_bits(), expected.to_bits());
    }
    let _client = cx.client.borrow_mut();
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
    assert_eq!(*cx.calls.borrow(), ["server"]);
    *cx.server.borrow_mut() = true;
    assert!(matches!(
        should_update_in_world_ref(&item, &cx),
        Err(RuntimeHostError::Borrowed("test client"))
    ));
}

#[test]
fn alarm_and_animal_virtual_overrides_bypass_all_base_context_reads() {
    for class in [
        ItemClass::AlarmClock,
        ItemClass::AlarmClockClothing,
        ItemClass::Animal,
    ] {
        let item = item(class, "Override");
        item.borrow_mut().item_heat = 2.0;
        let cx = Context::new(&item);
        let _server = cx.server.borrow_mut();
        let _client = cx.client.borrow_mut();
        assert_eq!(
            should_update_in_world_ref(&item, &cx),
            Ok(class == ItemClass::Animal)
        );
        {
            let mut body = item.borrow_mut();
            match &mut body.kind {
                ItemKind::AlarmClock(alarm) | ItemKind::AlarmClockClothing(_, alarm) => {
                    alarm.alarm_set = true
                }
                _ => {}
            }
        }
        assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
        assert!(cx.calls.borrow().is_empty());
    }
}

#[test]
fn food_override_uses_actual_captured_replacement_then_sandbox_heat_and_fertilized() {
    let mut template = ItemTemplate::new(ItemClass::Food, "Base", "Apple");
    template.days_totally_rotten = 4;
    template.replace_on_rotten = Some("Base.RottenApple".into());
    let item = crate::shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    // Later template mutation cannot replace the mutable instance field.
    let mut changed = (*item.borrow().template).clone();
    changed.replace_on_rotten = None;
    item.borrow_mut().template = Rc::new(changed);
    let cx = Context::new(&item);
    {
        let _sandbox = cx.days.borrow_mut();
        assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
        assert_eq!(*cx.calls.borrow(), ["client"]);
        with_food(&item, |food| food.replace_on_rotten = Some(String::new()));
        assert!(matches!(
            should_update_in_world_ref(&item, &cx),
            Err(RuntimeHostError::Borrowed("test sandbox"))
        ));
    }
    cx.calls.borrow_mut().clear();
    with_food(&item, |food| food.fertilized = true);
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
    assert_eq!(*cx.calls.borrow(), ["client", "rotten-days", "client"]);
    *cx.client.borrow_mut() = true;
    with_food(&item, |food| food.heat = f32::from_bits(0x7fc0_4321));
    cx.calls.borrow_mut().clear();
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
    assert_eq!(*cx.calls.borrow(), ["client"]);
    with_food(&item, |food| food.heat = 1.0);
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(false));
}

#[test]
fn food_never_rots_skips_sandbox_and_base_heat_world_state_entirely() {
    let item = item(ItemClass::Food, "Apple");
    item.borrow_mut().item_heat = 2.0;
    let cx = Context::new(&item);
    let _sandbox = cx.days.borrow_mut();
    let _server = cx.server.borrow_mut();
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(false));
    assert_eq!(*cx.calls.borrow(), ["client", "client"]);
    with_food(&item, |food| food.off_age_max = 1_000_000_001);
    assert!(matches!(
        should_update_in_world_ref(&item, &cx),
        Err(RuntimeHostError::Borrowed("test sandbox"))
    ));
}

#[test]
fn rain_reads_real_link_then_exact_minus_one_index_outside_and_fulltype_keys() {
    let item = item(ItemClass::Normal, "BathTowelWet");
    let cx = Context::new(&item);
    let world = link(&item, -1, Some(true));
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(false));
    assert_eq!(*cx.calls.borrow(), ["client", "index"]);
    world.borrow_mut().index = -2;
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(true));
    item.borrow_mut().type_ = "bathtowelwet".into();
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(false));
    item.borrow_mut().type_ = "DishClothWet".into();
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(true));
    world.borrow_mut().outside = Some(false);
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(false));
    world.borrow_mut().outside = None;
    assert!(matches!(
        should_update_in_rain_ref(&item, &cx),
        Err(RuntimeHostError::Unsupported { .. })
    ));
    world.borrow_mut().outside = Some(true);
    let _body = world.borrow_mut();
    assert!(matches!(
        should_update_in_rain_ref(&item, &cx),
        Err(RuntimeHostError::Borrowed("test world index"))
    ));
}

#[test]
fn food_rain_tainted_flag_is_actual_mutable_state_not_food_world_override() {
    let item = item(ItemClass::Food, "Apple");
    let cx = Context::new(&item);
    let _world = link(&item, 0, Some(true));
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(true));
    with_food(&item, |food| food.tainted = true);
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(false));
    item.borrow_mut().type_ = "BathTowelWet".into();
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(true));
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(false));
}

#[test]
fn drainable_virtual_heat_and_dry_rain_keys_are_distinct_from_inherited_wet_keys() {
    let item = item(ItemClass::Drainable, "BathTowel");
    let cx = Context::new(&item);
    let _world = link(&item, 0, Some(true));
    item.borrow_mut().item_heat = 2.0;
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(false));
    item.borrow_mut().type_ = "BathTowelWet".into();
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(false));
    assert_eq!(should_update_in_rain_ref(&item, &cx), Ok(true));
    item.borrow_mut().drainable_heat = Some(f32::NAN);
    let _client = cx.client.borrow_mut();
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
}

#[test]
fn real_fluid_membership_reads_no_component_body_and_null_world_is_not_false() {
    use pz_entity::{Component, RuntimeComponent};
    let item = item(ItemClass::Normal, "Bottle");
    let fluid =
        RuntimeComponent::new_unowned(Component::FluidContainer(Default::default())).unwrap();
    item.borrow_mut()
        .components
        .runtime_mut()
        .push_ref_if_absent(fluid.clone())
        .unwrap();
    let cx = Context::new(&item);
    let _fluid_body = fluid.borrow_mut();
    assert!(matches!(
        should_update_in_world_ref(&item, &cx),
        Err(RuntimeHostError::Unsupported { .. })
    ));
    let world = link(&item, -1, None);
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(false));
    world.borrow_mut().outside = Some(true);
    assert_eq!(should_update_in_world_ref(&item, &cx), Ok(true));
    assert!(!cx.calls.borrow().contains(&"index"));
    let _world_body = world.borrow_mut();
    assert!(matches!(
        should_update_in_world_ref(&item, &cx),
        Err(RuntimeHostError::Borrowed("test world square"))
    ));
}

#[test]
fn constructor_runtime_fields_are_not_wire_fields_and_load_preserves_existing_values() {
    struct Env;
    impl ItemEnv for Env {}
    for class in [ItemClass::Normal, ItemClass::Drainable, ItemClass::Food] {
        let mut template = ItemTemplate::new(class, "Base", "RuntimeState");
        template.replace_on_rotten = Some("initial".into());
        let mut item = InventoryItem::create(Rc::new(template), 1);
        assert_eq!(item.item_heat, 1.0);
        assert_eq!(
            item.drainable_heat,
            (class == ItemClass::Drainable).then_some(1.0)
        );
        let mut first = Writer::new();
        item.save(&mut first, &Env).unwrap();
        item.item_heat = 2.25;
        if class == ItemClass::Drainable {
            item.drainable_heat = Some(1.75);
        }
        if let ItemKind::Food(food) = &mut item.kind {
            food.replace_on_rotten = Some("actual-mutated".into());
        }
        let mut second = Writer::new();
        item.save(&mut second, &Env).unwrap();
        assert_eq!(first.buf, second.buf);
        let copy = item.try_archive_copy().unwrap();
        assert_eq!(copy.item_heat, 2.25);
        assert_eq!(copy.drainable_heat, item.drainable_heat);
        assert!(item.load(&mut Reader::new(&[]), 249, &Env).is_err());
        item.load(&mut Reader::new(&first.buf[3..]), 249, &Env)
            .unwrap();
        assert_eq!(item.item_heat, 2.25);
        if class == ItemClass::Drainable {
            assert_eq!(item.drainable_heat, Some(1.75));
        }
        if let ItemKind::Food(food) = &item.kind {
            assert_eq!(food.replace_on_rotten.as_deref(), Some("actual-mutated"));
        }
    }
}

#[test]
fn held_item_is_a_typed_fault_before_context_and_unknown_owner_stays_pinned_after_error() {
    let item = item(ItemClass::Normal, "BathTowelWet");
    let cx = Context::new(&item);
    {
        let _guard = item.borrow_mut();
        assert!(matches!(
            should_update_in_world_ref(&item, &cx),
            Err(RuntimeHostError::Borrowed("inventory world-update item"))
        ));
        assert!(cx.calls.borrow().is_empty());
    }
    struct Probe(Rc<RefCell<()>>, Rc<Cell<usize>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            assert!(self.0.try_borrow_mut().is_ok());
            self.1.set(self.1.get() + 1);
        }
    }
    let foreign = Rc::new(RefCell::new(()));
    let drops = Rc::new(Cell::new(0));
    let owner = Rc::new(RefCell::new(Probe(foreign.clone(), drops.clone())));
    crate::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&owner))).unwrap();
    drop(owner);
    crate::component_host::drain_retired_components();
    {
        let _guard = foreign.borrow_mut();
        assert!(matches!(
            should_update_in_rain_ref(&item, &cx),
            Err(RuntimeHostError::InvalidHostState("test actual world type"))
        ));
        drop(item);
        assert_eq!(
            drops.get(),
            0,
            "failed context must not finally drop owner under outer guard"
        );
    }
    crate::component_host::drain_retired_components();
    assert_eq!(drops.get(), 1);
}

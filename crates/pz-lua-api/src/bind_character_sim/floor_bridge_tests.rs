use super::*;
use pz_inventory::shared::{self, ItemRef};
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use std::cell::Cell;

struct Restore {
    floor: Option<FloorWorldRef>,
    floor_generation: u64,
    context: Rc<dyn SimContext>,
    sink: Option<EventSink>,
    item_environment: Rc<dyn crate::bind_items::ItemLuaEnv>,
}

impl Restore {
    fn new() -> Self {
        let floor_generation = floor_world_generation();
        Self {
            floor: reset_floor_world(),
            floor_generation,
            context: current_sim_context(),
            sink: SINK.with(|sink| sink.borrow().clone()),
            item_environment: crate::bind_items::item_env(),
        }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        let retired = install_floor_world(self.floor.take());
        FLOOR_GENERATION.with(|generation| generation.set(self.floor_generation));
        install_sim_context(self.context.clone());
        let retired_sink =
            SINK.with(|sink| std::mem::replace(&mut *sink.borrow_mut(), self.sink.take()));
        crate::bind_items::install_item_env(self.item_environment.clone());
        drop(retired);
        drop(retired_sink);
        pz_inventory::component_host::drain_retired_components();
    }
}

fn setup() -> (
    Thread,
    LuaGameCharacterRef,
    Rc<RefCell<Vec<String>>>,
    ItemRef,
) {
    let (mut thread, character, log) = tests::setup_with(ScriptManager::new());
    crate::bind_items::register(&mut thread);
    install_sim_context(Rc::new(DefaultSimContext));
    character.borrow_mut().path.current_square = Some((4, 6, 0));
    let item = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "FloorBridge")),
        1,
    ))
    .unwrap();
    let inventory = character.borrow().items.inventory.clone();
    shared::add_item(&inventory, &item);
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    (thread, character, log, item)
}

struct Tutorial;
impl SimContext for Tutorial {
    fn game_mode(&self) -> &str {
        "Tutorial"
    }
}

struct RecordingFloor {
    name: &'static str,
    calls: Rc<RefCell<Vec<String>>>,
    replacement: Option<FloorWorldRef>,
}

impl FloorWorld for RecordingFloor {
    fn solid_floor_ref_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> pz_inventory::floor_drop::HostResult<Option<pz_inventory::floor_drop::FloorSquare>> {
        self.solid_floor_at(x, y, z)
            .map(|sq| sq.map(pz_inventory::floor_drop::FloorSquare::data_projection))
    }
    fn apparent_z_on(
        &self,
        sq: &pz_inventory::floor_drop::FloorSquare,
        x: f32,
        y: f32,
    ) -> pz_inventory::floor_drop::HostResult<f32> {
        assert!(
            sq.owner_slot().is_none(),
            "explicit coordinate-only test profile"
        );
        self.apparent_z(sq.position(), x, y)
    }
    fn add_world_inventory_item_on(
        &mut self,
        sq: &pz_inventory::floor_drop::FloorSquare,
        item: &pz_inventory::shared::ItemRef,
        x: f32,
        y: f32,
        h: f32,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        assert!(
            sq.owner_slot().is_none(),
            "explicit coordinate-only test profile"
        );
        self.add_world_inventory_item(sq.position(), item, x, y, h)
    }

    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> HostResult<Option<(i32, i32, i32)>> {
        self.calls.borrow_mut().push(format!("{}:floor", self.name));
        if let Some(replacement) = &self.replacement {
            drop(install_floor_world(Some(replacement.clone())));
        }
        Ok(Some((x, y, z)))
    }
    fn apparent_z(&self, square: (i32, i32, i32), _: f32, _: f32) -> HostResult<f32> {
        self.calls
            .borrow_mut()
            .push(format!("{}:height", self.name));
        Ok(square.2 as f32)
    }
    fn rand_float(&mut self, min: f32, _: f32) -> HostResult<f32> {
        self.calls
            .borrow_mut()
            .push(format!("{}:random", self.name));
        Ok(min)
    }
    fn add_world_inventory_item(
        &mut self,
        _: (i32, i32, i32),
        _: &ItemRef,
        _: f32,
        _: f32,
        _: f32,
    ) -> HostResult<()> {
        self.calls.borrow_mut().push(format!("{}:place", self.name));
        Ok(())
    }
}

#[test]
fn real_bound_drop_preserves_native_noops_but_missing_or_held_needed_service_is_typed() {
    let _restore = Restore::new();
    let (mut thread, character, log, item) = setup();
    tests::run(&mut thread, "player:dropHandItems()").unwrap();
    tests::run(&mut thread, "player:setPrimaryHandItem(item)").unwrap();
    let inventory = character.borrow().items.inventory.clone();
    log.borrow_mut().clear();
    let error = tests::run(
        &mut thread,
        "player:dropHandItems(); error('false success')",
    )
    .unwrap_err();
    assert!(
        error.contains("pz.host.Unsupported") && error.contains("floor service is not installed")
    );
    assert!(!error.contains("false success"));
    assert!(Rc::ptr_eq(
        &shared::container_of(&item).unwrap(),
        &inventory
    ));
    assert_eq!(
        character.borrow().items.primary_hand_item,
        Some(item.borrow().id)
    );
    assert!(log.borrow().is_empty());

    install_sim_context(Rc::new(Tutorial));
    tests::run(&mut thread, "player:dropHandItems()").unwrap();
    install_sim_context(Rc::new(DefaultSimContext));
    character.borrow_mut().in_players = false;
    tests::run(&mut thread, "player:dropHandItems()").unwrap();
    character.borrow_mut().in_players = true;

    let calls = Rc::new(RefCell::new(Vec::new()));
    let floor: FloorWorldRef = Rc::new(RefCell::new(RecordingFloor {
        name: "held",
        calls: calls.clone(),
        replacement: None,
    }));
    drop(install_floor_world(Some(floor.clone())));
    let guard = floor.borrow_mut();
    tests::run(
        &mut thread,
        "player:setPrimaryHandItem(nil); player:dropHandItems()",
    )
    .unwrap();
    tests::run(&mut thread, "player:setPrimaryHandItem(item)").unwrap();
    install_sim_context(Rc::new(Tutorial));
    tests::run(&mut thread, "player:dropHandItems()").unwrap();
    install_sim_context(Rc::new(DefaultSimContext));
    let events_before_drop = log.borrow().clone();
    let error = tests::run(
        &mut thread,
        "player:dropHandItems(); error('false success')",
    )
    .unwrap_err();
    assert!(error.contains("pz.host.Borrowed") && !error.contains("false success"));
    assert!(calls.borrow().is_empty());
    assert_eq!(*log.borrow(), events_before_drop);
    assert_eq!(
        character.borrow().items.primary_hand_item,
        Some(item.borrow().id)
    );
    assert!(Rc::ptr_eq(
        &shared::container_of(&item).unwrap(),
        &inventory
    ));
    drop(guard);
}

#[test]
fn bound_drop_keeps_one_captured_service_when_registry_changes_during_its_query() {
    let _restore = Restore::new();
    let (mut thread, character, _, item) = setup();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let replacement: FloorWorldRef = Rc::new(RefCell::new(RecordingFloor {
        name: "replacement",
        calls: calls.clone(),
        replacement: None,
    }));
    let initial: FloorWorldRef = Rc::new(RefCell::new(RecordingFloor {
        name: "initial",
        calls: calls.clone(),
        replacement: Some(replacement.clone()),
    }));
    drop(install_floor_world(Some(initial)));
    tests::run(
        &mut thread,
        "player:setPrimaryHandItem(item); player:dropHandItems()",
    )
    .unwrap();
    assert!(Rc::ptr_eq(&floor_world().unwrap(), &replacement));
    let calls = calls.borrow();
    assert!(calls.iter().any(|call| call == "initial:place"));
    assert!(calls.iter().all(|call| call.starts_with("initial:")));
    assert!(character.borrow().items.primary_hand_item.is_none());
    assert!(shared::container_of(&item).is_none());
}

struct DropProbe(Rc<Cell<bool>>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        assert!(FloorAccess::capture().unwrap().service.is_none());
        self.0.set(true);
    }
}
impl FloorWorld for DropProbe {
    fn solid_floor_ref_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> pz_inventory::floor_drop::HostResult<Option<pz_inventory::floor_drop::FloorSquare>> {
        self.solid_floor_at(x, y, z)
            .map(|sq| sq.map(pz_inventory::floor_drop::FloorSquare::data_projection))
    }
    fn apparent_z_on(
        &self,
        sq: &pz_inventory::floor_drop::FloorSquare,
        x: f32,
        y: f32,
    ) -> pz_inventory::floor_drop::HostResult<f32> {
        assert!(
            sq.owner_slot().is_none(),
            "explicit coordinate-only test profile"
        );
        self.apparent_z(sq.position(), x, y)
    }
    fn add_world_inventory_item_on(
        &mut self,
        sq: &pz_inventory::floor_drop::FloorSquare,
        item: &pz_inventory::shared::ItemRef,
        x: f32,
        y: f32,
        h: f32,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        assert!(
            sq.owner_slot().is_none(),
            "explicit coordinate-only test profile"
        );
        self.add_world_inventory_item(sq.position(), item, x, y, h)
    }

    fn solid_floor_at(&self, _: i32, _: i32, _: i32) -> HostResult<Option<(i32, i32, i32)>> {
        Err(FloorAccess::unavailable())
    }
    fn apparent_z(&self, _: (i32, i32, i32), _: f32, _: f32) -> HostResult<f32> {
        Err(FloorAccess::unavailable())
    }
    fn rand_float(&mut self, _: f32, _: f32) -> HostResult<f32> {
        Err(FloorAccess::unavailable())
    }
    fn add_world_inventory_item(
        &mut self,
        _: (i32, i32, i32),
        _: &ItemRef,
        _: f32,
        _: f32,
        _: f32,
    ) -> HostResult<()> {
        Err(FloorAccess::unavailable())
    }
}

#[test]
fn resetting_floor_service_moves_old_owner_out_before_its_reentrant_destructor() {
    let _restore = Restore::new();
    let dropped = Rc::new(Cell::new(false));
    assert!(install_floor_world(Some(Rc::new(RefCell::new(DropProbe(dropped.clone()))))).is_none());
    let cell_generation = crate::bind_world::cell_generation();
    let registered = floor_world_generation();
    let retired = reset_floor_world();
    assert_ne!(floor_world_generation(), registered);
    assert_eq!(crate::bind_world::cell_generation(), cell_generation);
    assert!(!dropped.get());
    assert!(FloorAccess::capture().unwrap().service.is_none());
    drop(retired);
    assert!(dropped.get());
}

#[test]
fn bound_worn_force_preserves_native_event_absence_and_releases_all_host_guards() {
    let _restore = Restore::new();
    let (mut thread, character, _) = tests::setup_with(tests::clothing_scripts());
    crate::bind_items::install_item_env(Rc::new(crate::bind_items::GameItemEnv::new(None)));
    crate::bind_items::register(&mut thread);
    crate::bind_item_container::register(&mut thread);
    crate::bind_item_visual::register(&mut thread);
    install_sim_context(Rc::new(DefaultSimContext));
    let group = pz_characters::BodyLocationGroup::new("Human").unwrap();
    {
        let mut character = character.borrow_mut();
        character.init_worn_items(&shared_script_manager(), Rc::new(RefCell::new(group)));
        character.path.current_square = Some((4, 6, 0));
        character.items.inventory.borrow_mut().capacity = 1;
    }
    tests::run(&mut thread, "local L=ItemBodyLocation; player:getBodyLocationGroup():getOrCreateLocation(L.JACKET); local inv=player:getInventory(); coat=inv:AddItem('Base.TestCoat'); tee=inv:AddItem('Base.TestTee'); player:setWornItem(L.JACKET,coat,false)").unwrap();
    for item in character.borrow().items.inventory_items() {
        item.borrow_mut().name = "Invented".into();
    }
    let calls = Rc::new(RefCell::new(Vec::new()));
    let floor: FloorWorldRef = Rc::new(RefCell::new(RecordingFloor {
        name: "worn",
        calls: calls.clone(),
        replacement: None,
    }));
    drop(install_floor_world(Some(floor.clone())));
    {
        let _guard = floor.borrow_mut();
        tests::run(&mut thread, "player:setWornItem(ItemBodyLocation.JACKET,tee,false); player:setWornItem(ItemBodyLocation.JACKET,coat,false)").unwrap();
    }
    assert!(
        calls.borrow().is_empty(),
        "force=false does not touch a held service"
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let population = crate::bind_appearance::shared_population();
    install_event_sink(Rc::new(move |_, name, _| {
        emitted.borrow_mut().push(name.to_owned());
    }));
    tests::run(
        &mut thread,
        "player:setWornItem(ItemBodyLocation.JACKET,tee)",
    )
    .unwrap();
    assert!(
        events.borrow().is_empty(),
        "native forced worn placement does not emit hand-drop callbacks"
    );
    assert!(character.try_borrow_mut().is_ok());
    assert!(floor.try_borrow_mut().is_ok());
    assert!(population.try_borrow_mut().is_ok());
    assert!(calls.borrow().iter().any(|call| call == "worn:place"));
    let values = pz_kahlua::stdlib::load_string(
        "assert(coat:getContainer()==nil and player:getWornItem(ItemBodyLocation.JACKET)==tee)",
        None,
        thread.globals().clone(),
    )
    .unwrap();
    thread.pcall(&values, &[]).unwrap();
}

#[test]
fn shared_action_thread_collector_flushes_only_its_completed_prefix_before_typed_error() {
    let _restore = Restore::new();
    let (mut thread, character, _, _) = setup();
    let population = crate::bind_appearance::shared_population();
    let floor: FloorWorldRef = Rc::new(RefCell::new(RecordingFloor {
        name: "collector",
        calls: Rc::new(RefCell::new(Vec::new())),
        replacement: None,
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let check_character = character.clone();
    let check_floor = floor.clone();
    install_event_sink(Rc::new(move |_, name, _| {
        assert!(check_character.try_borrow_mut().is_ok());
        assert!(check_floor.try_borrow_mut().is_ok());
        assert!(population.try_borrow_mut().is_ok());
        emitted.borrow_mut().push(name.to_owned());
    }));
    let value = character_value(&character);
    thread.globals().set_str(
        "authoredActionPrefix",
        Value::Native(pz_kahlua::NativeFunction::new(
            "authoredActionPrefix",
            move |call| {
                with_ctx_action_thread(call, &value, |thread, context| {
                    crate::bind_appearance::with_styles(thread, |_| {
                        let _character = character
                            .try_borrow_mut()
                            .map_err(|_| RuntimeHostError::Borrowed("authored character guard"))?;
                        let _floor = floor
                            .try_borrow_mut()
                            .map_err(|_| RuntimeHostError::Borrowed("authored service guard"))?;
                        // This explicitly authored action prefix tests the shared
                        // collector, not an invented native setWornItem callback.
                        context.event(SimEvent::ContainerUpdate);
                        Err::<(), CharacterActionError>(
                            RuntimeHostError::Borrowed("authored action error after prefix").into(),
                        )
                    })
                })?;
                Ok(0)
            },
        )),
    );
    let error = tests::run(
        &mut thread,
        "authoredActionPrefix(); error('false success after typed action error')",
    )
    .unwrap_err();
    assert!(
        error.contains("pz.host.Borrowed") && error.contains("authored action error after prefix")
    );
    assert!(!error.contains("false success"));
    assert_eq!(&*events.borrow(), &["OnContainerUpdate".to_owned()]);
}

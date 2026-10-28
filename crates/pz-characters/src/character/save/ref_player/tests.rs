use super::*;
use crate::character::tests::{player, Recorder};
use crate::character::thermoregulator::body_tests::body_fixture;
use pz_inventory::env::ItemEnv;
use pz_inventory::item_texture::{CapturedScriptTextures, ItemTextures};
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_world::save::SaveTables;

fn tables() -> SaveTables {
    SaveTables {
        factory: vec![
            (0, "IsoObject".into()),
            (1, "Player".into()),
            (2, "Zombie".into()),
        ],
        blood_body_parts: 18,
        character_stats: Vec::new(),
    }
}
struct Env {
    template: Rc<ItemTemplate>,
    target: std::rc::Weak<RefCell<GameCharacter<()>>>,
    manager: std::rc::Weak<RefCell<Characters>>,
    calls: Cell<usize>,
}
impl ItemEnv for Env {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        assert!(
            self.target.upgrade().unwrap().try_borrow_mut().is_ok(),
            "factory held character guard"
        );
        assert!(
            self.manager.upgrade().unwrap().try_borrow_mut().is_ok(),
            "factory held manager guard"
        );
        self.calls.set(self.calls.get() + 1);
        (id == 1).then(|| self.template.clone())
    }
}
fn fixture() -> (
    ScriptManager,
    Rc<RefCell<Characters>>,
    GameCharacterRef<()>,
    Rc<ItemTemplate>,
) {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    let group = chars.body_locations.get_group("Human").unwrap();
    c.init_worn_items(&sm, group);
    let mut template = ItemTemplate::new(ItemClass::Key, "Base", "FixtureKey");
    template.texture_capture = Some(CapturedScriptTextures::new(None, vec![]));
    (
        sm,
        Rc::new(RefCell::new(chars)),
        Rc::new(RefCell::new(c)),
        Rc::new(template),
    )
}
fn archive_bytes(
    sm: &ScriptManager,
    chars: &Rc<RefCell<Characters>>,
    template: &Rc<ItemTemplate>,
) -> Vec<u8> {
    let mut source = player(sm, &mut chars.borrow_mut());
    source.init_worn_items(
        sm,
        chars
            .borrow_mut()
            .body_locations
            .get_group("Human")
            .unwrap(),
    );
    let mut item = InventoryItem::create(template.clone(), 1);
    item.id = 47;
    source.items.add_item(item);
    let mut writer = Writer::new();
    source
        .save_player(
            sm,
            &chars.borrow(),
            &Recorder::default(),
            &SaveEnv {
                items: &WireEnv(template.clone()),
                tables: &tables(),
            },
            &mut writer,
        )
        .unwrap();
    writer.buf
}
struct WireEnv(Rc<ItemTemplate>);
impl ItemEnv for WireEnv {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        (id == 1).then(|| self.0.clone())
    }
}

#[test]
fn actual_player_load_retains_list_and_container_and_constructs_live_items_without_outer_guards() {
    let (sm, chars, target, template) = fixture();
    let bytes = archive_bytes(&sm, &chars, &template);
    let list = target
        .borrow()
        .saved_inventory_items
        .actual_refs()
        .unwrap()
        .clone();
    let container = target.borrow().items.inventory.clone();
    let env = Env {
        template,
        target: Rc::downgrade(&target),
        manager: Rc::downgrade(&chars),
        calls: Cell::new(0),
    };
    let save_tables = tables();
    let mut reader = Reader::new(&bytes);
    load_player_ref(
        &target,
        &sm,
        &chars,
        &Recorder::default(),
        &SaveEnv {
            items: &env,
            tables: &save_tables,
        },
        &mut reader,
        249,
        &mut |_| panic!("not on fire"),
    )
    .unwrap();
    assert_eq!(reader.pos, bytes.len());
    assert!(env.calls.get() > 0);
    assert!(Rc::ptr_eq(&container, &target.borrow().items.inventory));
    assert!(Rc::ptr_eq(
        &list,
        target.borrow().saved_inventory_items.actual_refs().unwrap()
    ));
    let item = container.borrow().items[0].clone();
    assert!(Rc::ptr_eq(list.borrow()[0].as_ref().unwrap(), &item));
    assert!(Rc::ptr_eq(
        container.borrow().including_obsolete_items[0]
            .as_ref()
            .unwrap(),
        &item
    ));
    assert!(matches!(item.borrow().textures, ItemTextures::Live(_)));
    assert!(Rc::ptr_eq(
        &pz_inventory::shared::try_container_of(&item)
            .unwrap()
            .unwrap(),
        &container
    ));
    let mut writer = Writer::new();
    assert!(
        !item
            .borrow()
            .table
            .as_ref()
            .unwrap()
            .try_is_empty()
            .unwrap(),
        "Key.load's live customName entry must survive into bounded comparison save"
    );
    save_player_ref(
        &target,
        &sm,
        &chars,
        &Recorder::default(),
        &SaveEnv {
            items: &env,
            tables: &save_tables,
        },
        &mut writer,
    )
    .unwrap();
    assert!(!writer.buf.is_empty());
    assert!(Rc::ptr_eq(
        &list,
        target.borrow().saved_inventory_items.actual_refs().unwrap()
    ));
    assert!(Rc::ptr_eq(list.borrow()[0].as_ref().unwrap(), &item));
    pz_inventory::shared::try_set_container(&item, None).unwrap();
}

#[test]
fn failed_inventory_load_preserves_old_saved_list_after_native_inventory_prefix() {
    let (sm, chars, target, template) = fixture();
    let bytes = archive_bytes(&sm, &chars, &template);
    let list = target
        .borrow()
        .saved_inventory_items
        .actual_refs()
        .unwrap()
        .clone();
    let old = pz_inventory::shared::new_item(InventoryItem::create(template.clone(), 1));
    list.borrow_mut().push(Some(old.clone()));
    let env = Env {
        template,
        target: Rc::downgrade(&target),
        manager: Rc::downgrade(&chars),
        calls: Cell::new(0),
    };
    // Stop inside the actual container's record. Locate its position from the
    // genuine prefix reader instead of inventing a byte offset.
    let mut prefix = Reader::new(&bytes);
    prefix.i8().unwrap();
    prefix.i8().unwrap();
    target
        .borrow_mut()
        .load_game_prefix(
            &sm,
            &mut chars.borrow_mut(),
            &SaveEnv {
                items: &env,
                tables: &tables(),
            },
            &mut prefix,
            249,
        )
        .unwrap();
    let truncated = &bytes[..prefix.pos + 7];
    let mut reader = Reader::new(truncated);
    let error = load_player_ref(
        &target,
        &sm,
        &chars,
        &Recorder::default(),
        &SaveEnv {
            items: &env,
            tables: &tables(),
        },
        &mut reader,
        249,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        !error.is_host_fault(),
        "buffer underflow belongs to the Native catch domain"
    );
    assert!(Rc::ptr_eq(list.borrow()[0].as_ref().unwrap(), &old));
    assert!(target.borrow().items.inventory.borrow().items.is_empty());
}

#[test]
fn actual_saved_indices_use_pointer_identity_nulls_and_current_obsolete_list() {
    let (_, _, character, template) = fixture();
    let a = pz_inventory::shared::new_item(InventoryItem::create(template.clone(), 1));
    a.borrow_mut().id = 11;
    let b = pz_inventory::shared::new_item(InventoryItem::create(template, 1));
    b.borrow_mut().id = 22;
    let mut body = character.borrow_mut();
    let inventory = body.items.inventory.clone();
    inventory.borrow_mut().items = vec![a.clone(), b.clone()];
    inventory.borrow_mut().including_obsolete_items = vec![Some(b.clone()), None];
    let list = body.saved_inventory_items.actual_refs().unwrap().clone();
    *list.borrow_mut() = vec![None, Some(a.clone())];
    assert_eq!(body.persistence_base_item(0, true).unwrap(), Some(Some(22)));
    assert_eq!(
        body.persistence_saved_item(1, true).unwrap(),
        Some(Some(11))
    );
    assert_eq!(body.persistence_saved_index(None, true).unwrap(), 0);
    assert_eq!(body.persistence_main_index(None, true).unwrap(), -1);
    assert_eq!(body.persistence_base_item(1, true).unwrap(), Some(None));
    assert_eq!(body.persistence_saved_item(99, true).unwrap(), None);
    b.borrow_mut().id = 11;
    assert!(body
        .persistence_saved_item(1, true)
        .unwrap_err()
        .is_host_fault());
    body.items.primary_hand_item = None;
}

#[test]
fn actual_moving_moddata_held_save_reports_host_after_header_and_load_retains_partial_fields() {
    let (_, _, character, _) = fixture();
    let table = character.borrow_mut().get_mod_data();
    table.set_str("old", true);
    let guard = table.try_borrow_body_mut().unwrap();
    let mut writer = Writer::new();
    let error = moving::save(
        &character,
        &SaveEnv {
            items: &WireEnv(Rc::new(ItemTemplate::new(ItemClass::Key, "Base", "Key"))),
            tables: &tables(),
        },
        &mut writer,
    )
    .unwrap_err();
    assert!(error.is_host_fault());
    assert_eq!(
        writer.buf.len(),
        26,
        "held table must not erase the already written moving header or write a presence byte"
    );
    drop(guard);
    let mut wire = Writer::new();
    for f in [0.0, 0.0, 5.0, 6.0, 99.0] {
        wire.f32(f);
    }
    wire.i32(2);
    wire.bool(true);
    wire.i32(1);
    wire.i8(0);
    wire.string("new");
    wire.i8(1); // missing double payload
    let mut reader = Reader::new(&wire.buf);
    let error = moving::load(&character, &mut reader, 249).unwrap_err();
    assert!(!error.is_host_fault());
    assert_eq!(reader.pos, wire.buf.len());
    let body = character.borrow();
    assert_eq!(
        (body.vars.x, body.vars.next_x, body.vars.last_x),
        (5.0, 5.0, 5.0)
    );
    assert_eq!(
        (body.vars.y, body.vars.next_y, body.vars.last_y),
        (6.0, 6.0, 6.0)
    );
    assert_eq!((body.vars.z, body.vars.last_z), (31.0, 31.0));
    assert!(table.ptr_eq(body.mod_data.as_ref().unwrap()));
    assert!(
        table.is_empty(),
        "native count/wipe prefix must survive later underflow"
    );
}

#[test]
fn invalid_worn_location_skips_held_item_before_reference_to_id_bridge() {
    use crate::character::tests::item_constant;
    let (sm, chars, character, template) = fixture();
    let hat = item_constant(&sm, "item_body_location", "HAT");
    let item = pz_inventory::shared::new_item(InventoryItem::create(template, 1));
    let list = character
        .borrow()
        .saved_inventory_items
        .actual_refs()
        .unwrap()
        .clone();
    list.borrow_mut().push(Some(item.clone()));
    let mut wire = Writer::new();
    wire.i8(1);
    wire.string(&hat.location.id());
    wire.i16(0);
    let held = item.borrow_mut();
    character
        .borrow_mut()
        .load_worn_refs(&sm, &mut chars.borrow_mut(), &mut Reader::new(&wire.buf))
        .unwrap();
    drop(held);
}

#[test]
fn actual_character_entry_pins_receivers_until_outer_guard_ends_even_on_early_table_failure() {
    let (sm, chars, character, template) = fixture();
    let table = character.borrow_mut().get_mod_data();
    table.set_str("held", true);
    let table_guard = table.try_borrow_body_mut().unwrap();
    let weak_character = Rc::downgrade(&character);
    let weak_manager = Rc::downgrade(&chars);
    let outer = RefCell::new(());
    let outer_guard = outer.borrow_mut();
    let error = save_player_ref(
        &character,
        &sm,
        &chars,
        &Recorder::default(),
        &SaveEnv {
            items: &WireEnv(template),
            tables: &tables(),
        },
        &mut Writer::new(),
    )
    .unwrap_err();
    assert!(error.is_host_fault());
    drop(character);
    drop(chars);
    assert!(weak_character.upgrade().is_some());
    assert!(weak_manager.upgrade().is_some());
    drop(table_guard);
    drop(outer_guard);
    pz_inventory::component_host::drain_retired_components();
    assert!(weak_character.upgrade().is_none());
    assert!(weak_manager.upgrade().is_none());
}

use std::cell::Cell;
use std::rc::Rc;

use pz_inventory::io::Reader;
use pz_inventory::{ItemClass, ItemTemplate};

use super::*;

struct Env {
    templates: Vec<Rc<ItemTemplate>>,
    draws: Cell<usize>,
    lookups: Cell<usize>,
}

impl Env {
    fn new() -> Self {
        Self {
            templates: [
                (ItemClass::Normal, "Rock"),
                (ItemClass::Container, "Bag"),
                (ItemClass::Key, "Key1"),
            ]
            .into_iter()
            .map(|(class, name)| Rc::new(ItemTemplate::new(class, "Base", name)))
            .collect(),
            draws: Cell::new(0),
            lookups: Cell::new(0),
        }
    }

    fn item(&self, registry_id: i16, id: i32) -> ItemRef {
        let mut item = InventoryItem::create(
            self.templates[registry_id as usize - 1].clone(),
            registry_id,
        );
        item.id = id;
        shared::new_item(item)
    }
}

impl ItemEnv for Env {
    fn template(&self, registry_id: i16) -> Option<Rc<ItemTemplate>> {
        self.lookups.set(self.lookups.get() + 1);
        self.templates.get(registry_id as usize - 1).cloned()
    }

    fn rand_next(&self, _: i32) -> i32 {
        self.draws.set(self.draws.get() + 1);
        17
    }
}

#[test]
fn floor_item_retains_identity_even_with_duplicate_numeric_ids() {
    let env = Env::new();
    let first = env.item(1, 42);
    let second = env.item(1, 42);
    let floor = FloorItem::retain(&first);
    assert!(Rc::ptr_eq(&floor.item_ref().unwrap(), &first));
    assert!(!Rc::ptr_eq(&floor.item_ref().unwrap(), &second));
    first.borrow_mut().condition = 3;
    assert_eq!(floor.borrow_item().unwrap().condition, 3);
    floor.borrow_item_mut().unwrap().favorite = true;
    assert!(first.borrow().favorite);
    assert!(!second.borrow().favorite);
}

#[test]
fn floor_item_consumes_decoded_bag_and_links_its_actual_inventory() {
    let env = Env::new();
    let decoded = InventoryItem::create(env.templates[1].clone(), 2);
    let contents = shared::inventory_of(&decoded).unwrap();
    let child = env.item(1, 7);
    shared::add_item(&contents, &child);
    let parent: Rc<dyn std::any::Any> = Rc::new("retained decoded owner");
    contents.borrow_mut().parent = Some(parent.clone());
    let floor = FloorItem::from_saved(SavedItem::Item(Box::new(decoded)));
    let actual = floor.item_ref().unwrap();
    assert!(Rc::ptr_eq(
        &shared::inventory_of(&actual.borrow()).unwrap(),
        &contents
    ));
    assert!(Rc::ptr_eq(&contents.borrow().items[0], &child));
    assert!(Rc::ptr_eq(
        &contents
            .borrow()
            .containing_item
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap(),
        &actual
    ));
    assert!(Rc::ptr_eq(
        contents.borrow().parent.as_ref().unwrap(),
        &parent
    ));
    assert_eq!(env.draws.get(), 0, "promotion must not recreate the item");
    assert_eq!(env.lookups.get(), 0);
}

#[test]
fn floor_item_archive_clone_and_snapshot_deep_copy_nested_bags() {
    let env = Env::new();
    let bag = env.item(2, 8);
    let nested = env.item(2, 9);
    let child = env.item(1, 10);
    let contents = shared::inventory_of(&bag.borrow()).unwrap();
    let inner = shared::inventory_of(&nested.borrow()).unwrap();
    shared::add_item(&contents, &nested);
    shared::add_item(&inner, &child);
    contents.borrow_mut().parent = Some(Rc::new("opaque owner"));
    contents.borrow_mut().source_grid = Some(Rc::new("opaque square"));
    let floor = FloorItem::retain(&bag);
    let copy = floor.clone();
    assert_eq!(floor, copy);
    let copied_bag = copy.item_ref().unwrap();
    assert!(!Rc::ptr_eq(&bag, &copied_bag));
    let copied_contents = shared::inventory_of(&copied_bag.borrow()).unwrap();
    assert!(!Rc::ptr_eq(&contents, &copied_contents));
    assert!(copied_contents.borrow().parent.is_none());
    assert!(copied_contents.borrow().source_grid.is_none());
    assert!(Rc::ptr_eq(
        &copied_contents
            .borrow()
            .containing_item
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap(),
        &copied_bag
    ));
    let copied_nested = copied_contents.borrow().items[0].clone();
    assert!(!Rc::ptr_eq(&nested, &copied_nested));
    let copied_inner = shared::inventory_of(&copied_nested.borrow()).unwrap();
    let copied_child = copied_inner.borrow().items[0].clone();
    assert!(!Rc::ptr_eq(&child, &copied_child));
    copied_child.borrow_mut().condition = 2;
    assert_ne!(child.borrow().condition, 2);
    let snapshot = floor.to_saved_snapshot();
    let snapshot_contents = shared::inventory_of(snapshot.as_item().unwrap()).unwrap();
    assert!(!Rc::ptr_eq(&contents, &snapshot_contents));
    assert!(Rc::ptr_eq(&floor.item_ref().unwrap(), &bag));
}

#[test]
fn floor_item_raw_record_preserves_bytes_framing_and_never_promotes() {
    let env = Env::new();
    let raw = vec![0, 3, 0xff, 0, 0, 0, 42, 0xde, 0xad, 0, 0xff];
    let saved = SavedItem::Raw(raw.clone());
    let floor = FloorItem::from_saved(saved.clone());
    assert_eq!(floor.registry_id(), 3);
    assert!(floor.item_ref().is_none());
    assert!(floor.borrow_item().is_none());
    assert!(floor.borrow_item_mut().is_none());
    assert_eq!(floor.record(&env).unwrap(), raw);
    assert_eq!(floor.clone().to_saved_snapshot(), saved);
    let mut actual = Writer::new();
    let mut expected = Writer::new();
    floor.save(&mut actual, &env).unwrap();
    saved.save(&mut expected, &env).unwrap();
    assert_eq!(actual.buf, expected.buf);
    assert_eq!(
        i32::from_be_bytes(actual.buf[..4].try_into().unwrap()),
        raw.len() as i32
    );
    assert_eq!(&actual.buf[4..], &raw);
    assert_eq!((env.draws.get(), env.lookups.get()), (0, 0));
}

#[test]
fn floor_item_live_mutation_is_serialized_and_decoded_owner_is_reused() {
    let env = Env::new();
    let item = env.item(3, 19);
    let floor = FloorItem::retain(&item);
    item.borrow_mut().set_key_id(913, &env);
    item.borrow_mut().favorite = true;
    item.borrow_mut().uses = 4;
    let mut bytes = Writer::new();
    floor.save(&mut bytes, &env).unwrap();
    let mut expected = Writer::new();
    item.borrow().save_with_size(&mut expected, &env).unwrap();
    assert_eq!(bytes.buf, expected.buf);
    let mut r = Reader::new(&bytes.buf);
    let saved = SavedItem::load(&mut r, 249, &env).unwrap().unwrap();
    assert_eq!(r.remaining(), 0);
    let draws_after_decode = env.draws.get();
    let lookups_after_decode = env.lookups.get();
    let loaded = FloorItem::from_saved(saved);
    let alias = loaded.item_ref().unwrap();
    assert!(
        !Rc::ptr_eq(&alias, &item),
        "a disk load creates a new object"
    );
    assert_eq!(alias.borrow().get_key_id(), 913);
    assert_eq!(alias.borrow().uses, 4);
    assert!(alias.borrow().favorite);
    alias.borrow_mut().set_key_id(707, &env);
    assert_eq!(loaded.borrow_item().unwrap().get_key_id(), 707);
    assert_eq!(env.draws.get(), draws_after_decode);
    assert_eq!(env.lookups.get(), lookups_after_decode);
}

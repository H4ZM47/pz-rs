//! Hand-written item records: round trips and the load-time changes the
//! original makes.

use std::collections::HashMap;
use std::rc::Rc;

use crate::container::{load_items, load_items_saved, save_items, save_items_saved, ItemContainer};
use crate::env::ItemEnv;
use crate::io::{Reader, Writer};
use crate::item::{InventoryItem, ItemKind};
use crate::lua_table::{LuaKey, LuaValue};
use crate::saved::SavedItem;
use crate::template::{ItemClass, ItemTemplate};

struct Env {
    templates: HashMap<i16, Rc<ItemTemplate>>,
    age: f64,
}

impl ItemEnv for Env {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        self.templates.get(&id).cloned()
    }
    fn exists(&self, id: i16) -> bool {
        id != 99
    }
    fn world_age_hours(&self) -> f64 {
        self.age
    }
    fn blood_body_parts(&self) -> usize {
        4
    }
    fn rand_next(&self, _bound: i32) -> i32 {
        1234
    }
}

fn env() -> Env {
    let mut templates = HashMap::new();
    let mut add = |id: i16, class, ty: &str| {
        let mut t = ItemTemplate::new(class, "Base", ty);
        t.display_name = format!("{ty} name");
        templates.insert(id, Rc::new(t));
    };
    add(1, ItemClass::Normal, "Rock");
    add(2, ItemClass::Food, "Apple");
    add(3, ItemClass::Clothing, "Shirt");
    add(4, ItemClass::Weapon, "Axe");
    add(5, ItemClass::Literature, "Book");
    add(6, ItemClass::Key, "Key1");
    add(7, ItemClass::Container, "Bag");
    add(8, ItemClass::WeaponPart, "Scope");
    add(9, ItemClass::Radio, "Radio");
    add(10, ItemClass::AlarmClock, "Watch");
    Env {
        templates,
        age: 12.5,
    }
}

/// Saves, loads and saves again; the second load must re-save the same
/// bytes (the first load may apply the original's load-time changes).
fn round_trip(item: &InventoryItem, e: &Env) -> InventoryItem {
    let decode = |rec: &[u8]| match SavedItem::decode(rec, 249, e) {
        SavedItem::Item(i) => *i,
        SavedItem::Raw(_) => panic!("{} stayed raw", item.get_full_type()),
    };
    let mut w = Writer::new();
    item.save(&mut w, e).unwrap();
    let once = decode(&w.buf);
    let mut w2 = Writer::new();
    once.save(&mut w2, e).unwrap();
    let twice = decode(&w2.buf);
    let mut w3 = Writer::new();
    twice.save(&mut w3, e).unwrap();
    assert_eq!(w3.buf, w2.buf, "re-save of {}", item.get_full_type());
    twice
}

fn new_item(e: &Env, id: i16) -> InventoryItem {
    let mut i = crate::env::create_item(e, id).unwrap();
    i.id = 1000 + id as i32;
    i
}

#[test]
fn every_class_round_trips() {
    let e = env();
    for id in [1, 2, 4, 5, 8, 9, 10] {
        let mut i = new_item(&e, id);
        i.uses = 3;
        i.condition = 4;
        i.favorite = true;
        i.world_scale = 2.0;
        i.extra_items = Some(vec![1, 2]);
        i.byte_data = Some(vec![1, 2, 3]);
        let back = round_trip(&i, &e);
        assert_eq!(back.uses, 3);
        assert_eq!(back.condition, 4);
        assert!(back.favorite);
        assert_eq!(back.extra_items, Some(vec![1, 2]));
    }
}

#[test]
fn food_and_weapon_fields() {
    let e = env();
    let mut f = new_item(&e, 2);
    if let ItemKind::Food(x) = &mut f.kind {
        x.age = 3.0;
        x.cooked = true;
        x.spices = Some(vec!["Base.Salt".into()]);
        x.chef = Some("Kate".into());
        x.off_age = 4;
    }
    let back = round_trip(&f, &e);
    let ItemKind::Food(x) = &back.kind else {
        panic!()
    };
    assert!(x.cooked && x.age == 3.0 && x.off_age == 4);
    assert_eq!(x.chef.as_deref(), Some("Kate"));

    let mut w = new_item(&e, 4);
    let part = new_item(&e, 8);
    if let ItemKind::Weapon(x) = &mut w.kind {
        x.attachments.push(part);
        x.blood_level = 0.5;
        x.weapon_sprite = Some("Other".into());
    }
    let back = round_trip(&w, &e);
    let ItemKind::Weapon(x) = &back.kind else {
        panic!()
    };
    assert_eq!(x.attachments.len(), 1);
    assert_eq!(x.weapon_sprite.as_deref(), Some("Other"));
}

#[test]
fn literature_page_sizes() {
    let e = env();
    for pages in [5, 300, 40_000] {
        let mut l = new_item(&e, 5);
        if let ItemKind::Literature(x) = &mut l.kind {
            x.number_of_pages = pages;
            x.already_read_pages = pages - 1;
            x.custom_pages = Some(vec!["a".into(), "b".into()]);
        }
        let back = round_trip(&l, &e);
        let ItemKind::Literature(x) = &back.kind else {
            panic!()
        };
        assert_eq!(
            (x.number_of_pages, x.already_read_pages),
            (pages, pages - 1)
        );
    }
}

#[test]
fn load_time_changes() {
    let e = env();
    // Clothing: lastWetnessUpdate becomes the world age.
    let mut c = new_item(&e, 3);
    if let ItemKind::Clothing(x) = &mut c.kind {
        x.last_wetness_update = 1.0;
        x.wetness = 2.0;
    }
    let mut w = Writer::new();
    c.save(&mut w, &e).unwrap();
    let SavedItem::Item(back) = SavedItem::decode(&w.buf, 249, &e) else {
        panic!()
    };
    let ItemKind::Clothing(x) = &back.kind else {
        panic!()
    };
    assert_eq!(x.last_wetness_update, 12.5);
    assert_eq!(x.wetness, 2.0);

    // The bits block makes setCustomName put the name in the mod data.
    let mut r = new_item(&e, 1);
    r.activated = true;
    r.name = "My rock".into();
    let mut w = Writer::new();
    r.save(&mut w, &e).unwrap();
    let SavedItem::Item(back) = SavedItem::decode(&w.buf, 249, &e) else {
        panic!()
    };
    let t = back.table.as_ref().unwrap();
    assert_eq!(
        t.rawget(&LuaKey::Str("customName".into())),
        Some(&LuaValue::Str("My rock".into()))
    );

    // A key saved with id -1 draws a new one; the base keyId is not used.
    let k = new_item(&e, 6);
    let mut w = Writer::new();
    k.save(&mut w, &e).unwrap();
    let SavedItem::Item(back) = SavedItem::decode(&w.buf, 249, &e) else {
        panic!()
    };
    assert!(matches!(back.kind, ItemKind::Key { key_id: 1234, .. }));
    assert_eq!(back.key_id, -1);

    // A condition above the maximum is clamped.
    let mut big = new_item(&e, 1);
    big.condition = 20;
    let mut w = Writer::new();
    big.save(&mut w, &e).unwrap();
    let SavedItem::Item(back) = SavedItem::decode(&w.buf, 249, &e) else {
        panic!()
    };
    assert_eq!(back.condition, 10);
}

#[test]
fn identical_items_group_by_type() {
    let e = env();
    let mut a = new_item(&e, 1);
    let b = new_item(&e, 2);
    let mut a2 = a.clone();
    a2.id = 7;
    let mut a3 = a.clone();
    a3.id = 8;
    a3.uses = 5;
    a.id = 6;
    let items: Vec<SavedItem> = [a, b, a2, a3]
        .into_iter()
        .map(|i| SavedItem::Item(Box::new(i)))
        .collect();
    let mut w = Writer::new();
    save_items(&mut w, &items, &e).unwrap();
    let mut r = Reader::new(&w.buf);
    // Rock x2 (ids 6, 7), Rock with other uses, Apple: three runs.
    assert_eq!(r.i16().unwrap(), 3);
    assert_eq!(r.i32().unwrap(), 2);
    let back = load_items(&mut Reader::new(&w.buf), 249, &e).unwrap();
    let ids: Vec<i32> = back.iter().map(SavedItem::id).collect();
    assert_eq!(ids, vec![6, 7, 8, 1002]);
}

#[test]
fn containers_nest_and_unknown_items_drop() {
    let e = env();
    let mut bag = new_item(&e, 7);
    if let ItemKind::Container(c) = &mut bag.kind {
        c.container.borrow_mut().kind = "Bag".into();
        c.container.borrow_mut().id = 55;
        crate::shared::add_item(&c.container, &crate::shared::new_item(new_item(&e, 1)));
    }
    let back = round_trip(&bag, &e);
    let ItemKind::Container(c) = &back.kind else {
        panic!()
    };
    assert_eq!(c.container.borrow().id, 55);
    assert_eq!(c.container.borrow().items.len(), 1);
    // The script's wetCooldown (0, not the -1 `load` resets to) is saved,
    // so the bits block is read and `setCustomName` puts the name in the
    // mod data, as `InventoryItem.load` does (the bag and the item in it);
    // everything else is equal.
    let custom_name = |i: &InventoryItem| {
        i.table
            .as_ref()
            .and_then(|t| t.rawget(&LuaKey::Str("customName".into())).cloned())
    };
    assert_eq!(bag.wet_cooldown, 0.0);
    assert!(bag.table.is_none());
    assert_eq!(custom_name(&back), Some(LuaValue::Str("Bag name".into())));
    let inner = c.container.borrow().items[0].clone();
    assert_eq!(
        custom_name(&inner.borrow()),
        Some(LuaValue::Str("Rock name".into()))
    );
    inner.borrow_mut().table = None;
    let mut rest = back.clone();
    rest.table = None;
    assert_eq!(rest, bag);

    let mut cont = ItemContainer::new(Some("crate"));
    let mut unknown = Writer::new();
    unknown.i16(99);
    unknown.i8(-1);
    unknown.i32(5);
    unknown.i8(0);
    cont.items.push(SavedItem::Raw(unknown.buf));
    cont.items.push(SavedItem::Item(Box::new(new_item(&e, 1))));
    let mut w = Writer::new();
    cont.save(&mut w, &e).unwrap();
    let mut again = ItemContainer::new(None);
    again.load(&mut Reader::new(&w.buf), 249, &e).unwrap();
    assert_eq!(again.items.len(), 1);
}

#[test]
fn records_that_do_not_decode_stay_raw() {
    let e = env();
    let mut w = Writer::new();
    new_item(&e, 1).save(&mut w, &e).unwrap();
    w.i8(42); // trailing byte the class does not read
    assert!(matches!(
        SavedItem::decode(&w.buf, 249, &e),
        SavedItem::Raw(_)
    ));
}

/// `Item.InstanceItem` for a moveable: the weight is the moveable's after
/// `ReadFromWorldSprite`, so a sprite that is not a moveable leaves the
/// default 1.0 rather than the script's `Weight`.
#[test]
fn created_moveable_weight_follows_world_sprite() {
    struct MovEnv(HashMap<i16, Rc<ItemTemplate>>);
    impl ItemEnv for MovEnv {
        fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
            self.0.get(&id).cloned()
        }
        fn rand_next(&self, _bound: i32) -> i32 {
            7
        }
        fn world_sprite(&self, name: &str) -> Option<crate::env::WorldSpriteInfo> {
            (name == "fixtures_01_0").then(|| crate::env::WorldSpriteInfo {
                pick_up_weight: Some(150.0),
                custom_name: Some("Table".into()),
                ..Default::default()
            })
        }
    }
    let mut templates = HashMap::new();
    for (id, ty, sprite) in [
        (1, "Anvil", Some("crafted_01_35")),
        (2, "Table", Some("fixtures_01_0")),
        (3, "Moveable", None),
    ] {
        let mut t = ItemTemplate::new(ItemClass::Moveable, "Base", ty);
        t.display_name = format!("{ty} name");
        t.actual_weight = 40.0;
        t.world_object_sprite = sprite.map(str::to_string);
        templates.insert(id, Rc::new(t));
    }
    let e = MovEnv(templates);
    let weights = |id| {
        let i = crate::env::create_item(&e, id).unwrap();
        (i.weight, i.actual_weight)
    };
    assert_eq!(weights(1), (1.0, 1.0));
    assert_eq!(weights(2), (15.0, 15.0));
    assert_eq!(weights(3), (1.0, 1.0));
}

/// `CompressIdenticalItems.load` loads every copy of a run again from the
/// record, so each copy makes its own `CreateItem` draws (the `OutfitRNG`
/// seed and the item id here), and a run of 0 still loads its item once.
#[test]
fn identical_copies_load_with_their_own_draws() {
    struct Counting(Env, std::cell::Cell<i32>);
    impl ItemEnv for Counting {
        fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
            self.0.template(id)
        }
        fn rand_next(&self, _bound: i32) -> i32 {
            self.1.set(self.1.get() + 1);
            self.1.get()
        }
    }
    let e = Counting(env(), std::cell::Cell::new(0));
    let rock = |id| {
        let mut i = crate::env::create_item(&e, 1).unwrap();
        i.id = id;
        SavedItem::Item(Box::new(i))
    };
    let mut w = Writer::new();
    save_items(&mut w, &[rock(6), rock(7), rock(8)], &e).unwrap();
    let mut r = Reader::new(&w.buf);
    assert_eq!(r.i16().unwrap(), 1);
    assert_eq!(r.i32().unwrap(), 3);
    e.1.set(0);
    let back = load_items(&mut Reader::new(&w.buf), 249, &e).unwrap();
    assert_eq!(e.1.get(), 6);
    let ids: Vec<i32> = back.iter().map(SavedItem::id).collect();
    assert_eq!(ids, vec![6, 7, 8]);

    // The same run saved with a count of 0: one load, nothing added, no ids.
    let mut zero = w.buf.clone();
    zero[2..6].copy_from_slice(&0i32.to_be_bytes());
    let rec_end = 6 + 4 + i32::from_be_bytes(zero[6..10].try_into().unwrap()) as usize;
    zero.truncate(rec_end);
    e.1.set(0);
    let back = load_items(&mut Reader::new(&zero), 249, &e).unwrap();
    assert!(back.is_empty());
    assert_eq!(e.1.get(), 2);
}

/// `CompressIdenticalItems.save(output, items, noCompress)`: a run never
/// starts at an equipped item, but an equipped item after the run's first
/// item joins it; `savedItems` lists the items in written order.
#[test]
fn equipped_items_start_no_run() {
    let e = env();
    let rock = |id| {
        let mut i = new_item(&e, 1);
        i.id = id;
        SavedItem::Item(Box::new(i))
    };
    let items = [rock(6), rock(7), rock(8)];
    let runs = |equipped: &[i32]| {
        let f = |i: &SavedItem| equipped.contains(&i.id());
        let mut w = Writer::new();
        let saved = save_items_saved(&mut w, &items, &e, Some(&f)).unwrap();
        let (back, loaded) = load_items_saved(&mut Reader::new(&w.buf), 249, &e).unwrap();
        let ids: Vec<i32> = back.iter().map(SavedItem::id).collect();
        assert_eq!(ids, vec![6, 7, 8]);
        assert_eq!(loaded, vec![Some(0), Some(1), Some(2)]);
        (Reader::new(&w.buf).i16().unwrap(), saved)
    };
    assert_eq!(runs(&[]), (1, vec![0, 1, 2]));
    // Rock 6 equipped: its own run, then 7 and 8 together.
    assert_eq!(runs(&[6]), (2, vec![0, 1, 2]));
    // Rock 7 equipped: it joins 6's run, and 8 follows.
    assert_eq!(runs(&[7]), (1, vec![0, 1, 2]));
}

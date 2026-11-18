use super::*;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};

fn item(name: &str) -> SavedItem {
    let t = ItemTemplate::new(ItemClass::Clothing, "Base", name);
    SavedItem::Item(Box::new(InventoryItem::create(Rc::new(t), 1)))
}

fn name(i: &Option<ItemRef>) -> String {
    i.as_ref().map_or(String::new(), |i| i.borrow().template.full_type().to_string())
}

/// A corpse whose saved container is `[Shirt, raw record, Pants]`, with the
/// shirt and pants worn and the raw record "attached".
fn body() -> IsoDeadBody {
    let mut c = ItemContainer::new(Some("inventorymale"));
    c.id = 7;
    c.items = vec![item("Shirt"), SavedItem::Raw(vec![0, 1, 2]), item("Pants")];
    let mut b = IsoDeadBody::for_tests(1.5, 2.5, 0.0);
    b.container = Some(c);
    b.worn_items = vec![("base:shirt".into(), 0), ("base:pants".into(), 2)];
    b.attached_items = vec![("base:back".into(), 1)];
    b
}

#[test]
fn the_first_get_container_moves_the_saved_inventory_into_live_items() {
    let mut b = body();
    let c = b.runtime_container().unwrap();
    assert!(b.container.is_none() && b.worn_items.is_empty() && b.attached_items.is_empty());
    assert_eq!(c.borrow().kind, "inventorymale");
    assert_eq!(c.borrow().items.len(), 2);
    assert_eq!(c.borrow().raw_items.len(), 1);
    let r = b.runtime.as_ref().unwrap();
    let worn: Vec<_> = r.worn.iter().map(|(l, i)| (l.as_str(), name(i))).collect();
    assert_eq!(worn, [("base:shirt", "Base.Shirt".into()), ("base:pants", "Base.Pants".into())]);
    // The raw record has no item object.
    assert!(r.attached[0].1.is_none());
    // Later calls give the same container.
    assert!(Rc::ptr_eq(&b.runtime_container().unwrap(), &c));
}

#[test]
fn the_snapshot_names_the_items_current_positions() {
    let mut b = body();
    let before = b.inventory_snapshot().unwrap().unwrap();
    assert_eq!(before.worn, [("base:shirt".into(), 0), ("base:pants".into(), 2)]);
    assert!(!before.positions);
    let c = b.runtime_container().unwrap();
    let shirt = c.borrow_mut().items.remove(0);
    pz_inventory::shared::set_container(&shirt, None);
    let s = b.inventory_snapshot().unwrap().unwrap();
    assert!(s.positions);
    assert_eq!(s.container.id, 7);
    assert_eq!(s.container.items.len(), 2);
    assert_eq!(s.worn, [("base:shirt".into(), -1), ("base:pants".into(), 0)]);
    assert_eq!(s.attached, [("base:back".into(), -1)]);
}

#[test]
fn a_copy_has_its_own_container_and_entries() {
    let mut b = body();
    let c = b.runtime_container().unwrap();
    let copy = b.clone();
    let r = copy.runtime.as_ref().unwrap();
    assert!(!Rc::ptr_eq(&r.container, &c));
    assert_eq!(copy, b);
    let worn = r.worn[1].1.as_ref().unwrap();
    assert!(Rc::ptr_eq(worn, &r.container.borrow().items[1]));
}

#[test]
fn check_clothing_drops_the_entries_of_items_taken_out() {
    let mut b = body();
    // Nothing to check before Lua reached the container.
    b.check_clothing();
    assert_eq!(b.worn_items.len(), 2);
    let c = b.runtime_container().unwrap();
    let shirt = c.borrow_mut().items.remove(0);
    pz_inventory::shared::set_container(&shirt, None);
    b.check_clothing();
    let r = b.runtime.as_ref().unwrap();
    let worn: Vec<_> = r.worn.iter().map(|(l, i)| (l.as_str(), name(i))).collect();
    assert_eq!(worn, [("base:pants", "Base.Pants".into())]);
    assert!(r.attached.is_empty());
}

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::template::ItemTemplate;

fn item(class: ItemClass, ty: &str, weight: f32, id: i32) -> ItemRef {
    let mut t = ItemTemplate::new(class, "Base", ty);
    t.actual_weight = weight;
    let mut i = InventoryItem::create(Rc::new(t), 1);
    i.id = id;
    Rc::new(RefCell::new(i))
}

fn tagged(ty: &str, tag: &str, id: i32) -> ItemRef {
    let r = item(ItemClass::Normal, ty, 1.0, id);
    let mut t = (*r.borrow().template).clone();
    t.tags.push(tag.into());
    r.borrow_mut().template = Rc::new(t);
    r
}

struct Chr {
    inv: ContainerRef,
    player: bool,
    vehicle: bool,
    square: Option<SquarePos>,
    primary: Option<ItemRef>,
    secondary: Option<ItemRef>,
    log: Vec<String>,
}

impl Chr {
    fn new(items: &[&ItemRef]) -> Self {
        let inv = SharedContainer::default().into_ref();
        for i in items {
            shared::add_item(&inv, i);
        }
        Chr {
            inv,
            player: true,
            vehicle: false,
            square: Some((10, 20, 1)),
            primary: None,
            secondary: None,
            log: Vec::new(),
        }
    }
}

impl DropCharacter for Chr {
    fn inventory(&self) -> ContainerRef {
        self.inv.clone()
    }
    fn is_player(&self) -> bool {
        self.player
    }
    fn is_in_vehicle(&self) -> bool {
        self.vehicle
    }
    fn current_square(&self) -> Option<SquarePos> {
        self.square
    }
    fn capacity_weight(&self) -> f32 {
        self.inv
            .borrow()
            .items
            .iter()
            .map(|i| i.borrow().template.actual_weight)
            .sum()
    }
    fn unequipped_weight(&self, item: &ItemRef) -> f32 {
        item.borrow().template.actual_weight
    }
    fn primary_hand_item(&self) -> Option<ItemRef> {
        self.primary.clone()
    }
    fn secondary_hand_item(&self) -> Option<ItemRef> {
        self.secondary.clone()
    }
    fn set_primary_hand_item(&mut self, item: Option<&ItemRef>) {
        self.log.push("primary=null".into());
        self.primary = item.cloned();
    }
    fn set_secondary_hand_item(&mut self, item: Option<&ItemRef>) {
        self.log.push("secondary=null".into());
        self.secondary = item.cloned();
    }
    fn on_container_update(&mut self) {
        self.log.push("OnContainerUpdate".into());
    }
    fn on_item_fall(&mut self, item: &ItemRef) {
        self.log.push(format!("onItemFall {}", item.borrow().id));
    }
    fn play_drop_item_sound(&mut self, item: &ItemRef) {
        self.log.push(format!("sound {}", item.borrow().id));
    }
}

/// Solid floor on level 0 only; draws come from a list.
struct World {
    draws: Vec<f32>,
    added: Vec<(SquarePos, i32, f32, f32, f32)>,
}

impl FloorWorld for World {
    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> Option<SquarePos> {
        (z >= 0).then_some((x, y, 0))
    }
    fn apparent_z(&self, sq: SquarePos, dx: f32, _dy: f32) -> f32 {
        sq.2 as f32 + dx / 2.0
    }
    fn rand_float(&mut self, min: f32, max: f32) -> f32 {
        min + self.draws.remove(0) * (max - min)
    }
    fn add_world_inventory_item(
        &mut self,
        sq: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        h: f32,
    ) -> HostResult<()> {
        assert!(shared::container_of(item).is_none());
        self.added.push((sq, item.borrow().id, x, y, h));
        Ok(())
    }
}

fn world() -> World {
    World {
        draws: vec![0.5, 0.25, 0.0, 1.0, 0.75, 0.5],
        added: Vec::new(),
    }
}

#[test]
fn heavy_items() {
    let bag = item(ItemClass::Container, "Bag_Schoolbag", 0.5, 1);
    assert!(matches!(bag.borrow().kind, ItemKind::Container(_)));
    assert!(is_heavy_item(Some(&bag.borrow())));
    assert!(is_heavy_item(Some(
        &tagged("Anvil", "base:HeavyItem", 2).borrow()
    )));
    for t in [
        "CorpseMale",
        "CorpseFemale",
        "Animal",
        "CorpseAnimal",
        "Generator",
    ] {
        assert!(
            is_heavy_item(Some(&item(ItemClass::Normal, t, 1.0, 3).borrow())),
            "{t}"
        );
    }
    // `isHeavyItem` compares the type exactly; `isHumanCorpse` ignores case.
    let lower = item(ItemClass::Normal, "corpsemale", 1.0, 4);
    assert!(!is_heavy_item(Some(&lower.borrow())));
    assert!(lower.borrow().is_human_corpse());
    assert!(!is_heavy_item(Some(
        &item(ItemClass::Normal, "Hammer", 1.0, 5).borrow()
    )));
    assert!(!is_heavy_item(None));
}

#[test]
fn capacity_and_rounding() {
    assert_eq!(floating_point_correction(1.234), 1.23);
    assert_eq!(floating_point_correction(1.235), 1.24);
    let mut c = SharedContainer {
        capacity: 120,
        ..Default::default()
    };
    assert_eq!(get_capacity(&c), 100);
    let bag = item(ItemClass::Container, "Bag", 0.5, 1);
    c.containing_item = Some(Rc::downgrade(&bag));
    assert_eq!(get_capacity(&c), 50);
}

#[test]
fn room_in_the_inventory() {
    let a = item(ItemClass::Normal, "Brick", 31.0, 1);
    let b = item(ItemClass::Normal, "Plank", 10.0, 2);
    let mut chr = Chr::new(&[&a, &b]);
    // 41 carried + 10 > 50.
    assert!(!has_room_for(&mut chr, &b));
    let small = item(ItemClass::Normal, "Nail", 0.01, 3);
    assert!(has_room_for(&mut chr, &small));
    // Exactly at the capacity fits.
    let nine = item(ItemClass::Normal, "Log", 9.0, 4);
    assert!(has_room_for(&mut chr, &nine));
    let heavy = tagged("Anvil", "base:heavyitem", 5);
    chr.vehicle = true;
    assert!(!has_room_for(&mut chr, &heavy));
    assert!(has_room_for(&mut chr, &small));
    chr.vehicle = false;
    let animal = item(ItemClass::Animal, "Animal", 0.0, 6);
    assert!(!has_room_for(&mut chr, &animal));
    chr.inv.borrow_mut().only_accept_category = Some("food".into());
    assert!(!has_room_for(&mut chr, &small));
    let food = item(ItemClass::Food, "Apple", 0.1, 7);
    assert!(has_room_for(&mut chr, &food));
}

#[test]
fn worn_item_without_room_drops() {
    let coat = item(ItemClass::Clothing, "Coat", 30.0, 1);
    let rock = item(ItemClass::Normal, "Rock", 25.0, 2);
    let mut chr = Chr::new(&[&coat, &rock]);
    chr.primary = Some(coat.clone());
    let mut w = world();
    // Not asked to, or not a player: nothing.
    assert_eq!(
        force_drop_too_heavy(&mut chr, &mut w, false, Some(&coat)),
        Ok(None)
    );
    chr.player = false;
    assert_eq!(
        force_drop_too_heavy(&mut chr, &mut w, true, Some(&coat)),
        Ok(None)
    );
    chr.player = true;
    assert_eq!(force_drop_too_heavy(&mut chr, &mut w, true, None), Ok(None));
    assert!(w.added.is_empty());
    let d = force_drop_too_heavy(&mut chr, &mut w, true, Some(&coat))
        .unwrap()
        .unwrap();
    assert_eq!(d.square, (10, 20, 0));
    let r = 0.9f32 - 0.1;
    let (x, y) = (0.1 + 0.5 * r, 0.1 + 0.25 * r);
    assert_eq!((d.x, d.y, d.z), (x, y, x / 2.0));
    assert_eq!(w.added, vec![((10, 20, 0), 1, x, y, x / 2.0)]);
    assert_eq!(chr.inv.borrow().items.len(), 1);
    assert!(chr.primary.is_none(), "Remove takes it out of the hands");
    // With room left it stays.
    let mut w = world();
    assert_eq!(
        force_drop_too_heavy(&mut chr, &mut w, true, Some(&rock)),
        Ok(None)
    );
    // Without a floor nothing is removed.
    let mut chr = Chr::new(&[&coat, &rock]);
    chr.square = Some((1, 1, -1));
    assert_eq!(
        force_drop_too_heavy(&mut chr, &mut w, true, Some(&coat)),
        Ok(None)
    );
    assert_eq!(chr.inv.borrow().items.len(), 2);
}

#[test]
fn heavy_hand_items_drop() {
    let gen = item(ItemClass::Normal, "Generator", 20.0, 1);
    let hammer = item(ItemClass::Normal, "Hammer", 1.0, 2);
    let mut chr = Chr::new(&[&gen, &hammer]);
    chr.primary = Some(gen.clone());
    chr.secondary = Some(hammer.clone());
    let mut w = world();
    let d = drop_heavy_items(&mut chr, &mut w).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(
        chr.log,
        [
            "primary=null",
            "OnContainerUpdate",
            "onItemFall 1",
            "sound 1"
        ]
    );
    assert!(chr.secondary.is_some());
    assert_eq!(chr.inv.borrow().items.len(), 1);
    // `dropHandItems` drops the rest.
    chr.log.clear();
    let d = drop_hand_items(&mut chr, &mut w, false).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].square, (10, 20, 0));
    assert_eq!(
        chr.log,
        [
            "secondary=null",
            "OnContainerUpdate",
            "onItemFall 2",
            "sound 2"
        ]
    );
    assert!(chr.inv.borrow().items.is_empty());
    assert_eq!(w.added.iter().map(|a| a.1).collect::<Vec<_>>(), [1, 2]);
}

#[test]
fn two_handed_item_drops_once() {
    let bag = item(ItemClass::Container, "Duffel", 2.0, 1);
    let mut chr = Chr::new(&[&bag]);
    chr.primary = Some(bag.clone());
    chr.secondary = Some(bag.clone());
    let mut w = world();
    assert_eq!(drop_heavy_items(&mut chr, &mut w).unwrap().len(), 1);
    assert_eq!(
        chr.log,
        [
            "primary=null",
            "OnContainerUpdate",
            "onItemFall 1",
            "sound 1",
            "secondary=null"
        ]
    );
    assert!(chr.primary.is_none() && chr.secondary.is_none());
    // Tutorial and remote players keep their items.
    let mut chr = Chr::new(&[&bag]);
    chr.primary = Some(bag.clone());
    assert!(drop_hand_items(&mut chr, &mut w, true).unwrap().is_empty());
    assert!(chr.primary.is_some());
    // No square, or empty hands: nothing.
    chr.square = None;
    assert!(drop_heavy_items(&mut chr, &mut w).unwrap().is_empty());
}

struct FailingWorld {
    inner: World,
    fail_at: usize,
    attempts: Vec<i32>,
}
impl FloorWorld for FailingWorld {
    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> Option<SquarePos> {
        self.inner.solid_floor_at(x, y, z)
    }
    fn apparent_z(&self, sq: SquarePos, dx: f32, dy: f32) -> f32 {
        self.inner.apparent_z(sq, dx, dy)
    }
    fn rand_float(&mut self, min: f32, max: f32) -> f32 {
        self.inner.rand_float(min, max)
    }
    fn add_world_inventory_item(
        &mut self,
        sq: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        h: f32,
    ) -> HostResult<()> {
        self.attempts.push(item.borrow().id);
        if self.attempts.len() == self.fail_at {
            return Err(RuntimeHostError::Borrowed("actual floor allocation"));
        }
        self.inner.add_world_inventory_item(sq, item, x, y, h)
    }
}

#[test]
fn first_placement_fault_preserves_remove_prefix_and_stops_second_hand_and_events() {
    let first = item(ItemClass::Normal, "Generator", 1.0, 1);
    let second = item(ItemClass::Normal, "Rock", 1.0, 2);
    let mut chr = Chr::new(&[&first, &second]);
    chr.primary = Some(first.clone());
    chr.secondary = Some(second.clone());
    let mut w = FailingWorld {
        inner: world(),
        fail_at: 1,
        attempts: Vec::new(),
    };
    assert_eq!(
        drop_hand_items(&mut chr, &mut w, false),
        Err(RuntimeHostError::Borrowed("actual floor allocation"))
    );
    assert_eq!(w.attempts, [1]);
    assert!(w.inner.added.is_empty());
    assert_eq!(w.inner.draws.len(), 4, "only the first x/y draws execute");
    assert!(chr.primary.is_none());
    assert!(Rc::ptr_eq(chr.secondary.as_ref().unwrap(), &second));
    assert!(shared::container_of(&first).is_none());
    assert!(shared::container_of(&second).is_some());
    assert_eq!(chr.log, ["primary=null"]);
}

#[test]
fn second_placement_fault_preserves_first_success_events_and_second_remove_prefix() {
    let first = item(ItemClass::Normal, "Generator", 1.0, 1);
    let second = item(ItemClass::Normal, "Generator", 1.0, 2);
    let mut chr = Chr::new(&[&first, &second]);
    chr.primary = Some(first.clone());
    chr.secondary = Some(second.clone());
    let mut w = FailingWorld {
        inner: world(),
        fail_at: 2,
        attempts: Vec::new(),
    };
    assert!(matches!(
        drop_heavy_items(&mut chr, &mut w),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert_eq!(w.attempts, [1, 2]);
    assert_eq!(w.inner.added.iter().map(|v| v.1).collect::<Vec<_>>(), [1]);
    assert_eq!(w.inner.draws.len(), 2);
    assert!(chr.primary.is_none() && chr.secondary.is_none());
    assert!(chr.inv.borrow().items.is_empty());
    assert_eq!(
        chr.log,
        [
            "primary=null",
            "OnContainerUpdate",
            "onItemFall 1",
            "sound 1",
            "secondary=null"
        ]
    );
}

#[test]
fn forced_clothing_drop_fault_is_distinct_from_no_drop_after_inventory_removal() {
    let coat = item(ItemClass::Clothing, "Coat", 60.0, 1);
    let mut chr = Chr::new(&[&coat]);
    chr.primary = Some(coat.clone());
    let mut w = FailingWorld {
        inner: world(),
        fail_at: 1,
        attempts: Vec::new(),
    };
    assert_eq!(
        force_drop_too_heavy(&mut chr, &mut w, true, Some(&coat)),
        Err(RuntimeHostError::Borrowed("actual floor allocation"))
    );
    assert_eq!(w.attempts, [1]);
    assert!(chr.inv.borrow().items.is_empty());
    assert!(chr.primary.is_none());
    assert_eq!(chr.log, ["primary=null"]);
}

#[test]
fn rejected_last_hand_item_is_pinned_until_outer_foreign_guard_ends() {
    struct Probe(Rc<RefCell<()>>, Rc<std::cell::Cell<usize>>);
    impl pz_kahlua::LuaObject for Probe {
        fn class_name(&self) -> &str {
            "test.RejectedFloorDrop"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            assert!(
                self.0.try_borrow_mut().is_ok(),
                "item final drop under foreign guard"
            );
            self.1.set(self.1.get() + 1);
        }
    }
    crate::component_host::drain_retired_components();
    let foreign = Rc::new(RefCell::new(()));
    let drops = Rc::new(std::cell::Cell::new(0));
    let first = item(ItemClass::Normal, "Generator", 1.0, 1);
    crate::moddata::set_mod_data_value_ref(
        &first,
        "probe",
        Some(pz_kahlua::Value::Object(Rc::new(Probe(
            foreign.clone(),
            drops.clone(),
        )))),
    )
    .unwrap();
    let mut chr = Chr::new(&[&first]);
    chr.primary = Some(first.clone());
    drop(first);
    crate::component_host::drain_retired_components();
    let mut w = FailingWorld {
        inner: world(),
        fail_at: 1,
        attempts: Vec::new(),
    };
    {
        let _guard = foreign.borrow_mut();
        assert!(drop_heavy_items(&mut chr, &mut w).is_err());
        assert!(chr.inv.borrow().items.is_empty() && chr.primary.is_none());
        assert_eq!(drops.get(), 0);
    }
    crate::component_host::drain_retired_components();
    assert_eq!(drops.get(), 1);
}

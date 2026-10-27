use super::*;
use crate::assets::{ClothingItem, ItemAssets};
use crate::context::{ItemContext, NoOwner};
use crate::env::ItemEnv;
use crate::factory::create_item_by_type;
use crate::shared;
use crate::visual::NoVisualRng;
use crate::{InventoryItem, ItemTemplate, SharedContainer};
use pz_scripts::objects::items::Item;
use pz_scripts::{ScriptLoadMode, ScriptManager, ScriptType};

struct FactoryEnv;
impl ItemEnv for FactoryEnv {}

struct Context;
impl ItemContext for Context {
    fn world_age_hours(&self) -> f64 {
        0.0
    }
    fn multiplier(&self) -> f32 {
        1.0
    }
    fn rain_intensity(&self) -> f32 {
        0.0
    }
    fn is_raining(&self) -> bool {
        false
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        0.3
    }
    fn basic_mask_defense(&self) -> f32 {
        0.0
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn script(&self, _: &str) -> Option<Rc<ItemTemplate>> {
        None
    }
}

struct Host {
    sm: ScriptManager,
    assets: ItemAssets,
    worn: InventoryWornItemsRef,
    log: Vec<String>,
    made: Vec<std::rc::Weak<RefCell<InventoryItem>>>,
    factory_error: Option<String>,
    sync_host_error_at: Option<usize>,
    break_error: bool,
    append_on_add: Option<ItemRef>,
    change_max_in_holes: bool,
    create_parent: Option<Rc<dyn std::any::Any>>,
}

impl Host {
    fn new() -> Self {
        let sm = ScriptManager::new();
        for (field, id) in [
            ("CLOTHING", "test:clothing"),
            ("ALARM_CLOCK_CLOTHING", "test:alarm_clock_clothing"),
            ("CONTAINER", "test:container"),
            ("NORMAL", "test:normal"),
        ] {
            sm.registries
                .get("item_type")
                .unwrap()
                .borrow_mut()
                .register_id(id, false)
                .unwrap();
            sm.statics
                .items
                .borrow_mut()
                .constants
                .insert("item_type", field, id);
        }
        let mut group = super::super::BodyLocationGroup::new("TestHuman").unwrap();
        for id in ["test:chest", "test:back", "test:wrist"] {
            let key = sm
                .registries
                .get("item_body_location")
                .unwrap()
                .borrow_mut()
                .register_id(id, false)
                .unwrap();
            group.get_or_create_location(Some(key)).unwrap();
        }
        sm.parse_script(ScriptLoadMode::Init, "module Base {
            item Shirt { ItemType = test:clothing, BodyLocation = test:chest, CanBeEquipped = test:back, ClothingItem = ShirtAsset, ConditionMax = 10, }
            item Coat { ItemType = test:clothing, BodyLocation = test:chest, ClothingItem = CoatAsset, ConditionMax = 12, OnBreak = FixtureBreak, }
            item Bag { ItemType = test:container, BodyLocation = test:chest, CanBeEquipped = test:back, ClothingItem = BagAsset, ConditionMax = 9, }
            item Invisible { ItemType = test:clothing, BodyLocation = test:chest, ConditionMax = 10, }
            item Unwearable { ItemType = test:normal, }
            item Watch { ItemType = test:alarm_clock_clothing, BodyLocation = test:wrist, ClothingItem = WatchAsset, ConditionMax = 8, }
        }").unwrap();
        sm.get_module("Base")
            .unwrap()
            .bucket(ScriptType::Item)
            .load_scripts(&sm, ScriptLoadMode::Init);
        for (name, id) in [
            ("Shirt", 71),
            ("Coat", 72),
            ("Bag", 73),
            ("Invisible", 74),
            ("Unwearable", 75),
            ("Watch", 76),
        ] {
            sm.find_item(&format!("Base.{name}"), false)
                .unwrap()
                .downcast::<Item>()
                .unwrap()
                .borrow_mut()
                .entity
                .set_registry_id(id)
                .unwrap();
        }
        let mut assets = ItemAssets::default();
        for name in ["ShirtAsset", "CoatAsset", "BagAsset", "WatchAsset"] {
            let mut asset = ClothingItem::new(name);
            asset.allow_random_tint = true;
            assets.clothing_items.insert(name, name, Some(asset));
        }
        Self {
            sm,
            assets,
            worn: Rc::new(RefCell::new(WornItems::new(Rc::new(RefCell::new(group))))),
            log: Vec::new(),
            made: Vec::new(),
            factory_error: None,
            sync_host_error_at: None,
            break_error: false,
            append_on_add: None,
            change_max_in_holes: false,
            create_parent: None,
        }
    }
    fn free_guards(&self, item: Option<&ItemRef>) {
        let worn = self
            .worn
            .try_borrow_mut()
            .expect("host called under worn guard");
        assert!(worn.body_location_group().try_borrow_mut().is_ok());
        if let Some(item) = item {
            assert!(item.try_borrow_mut().is_ok());
        }
    }
    fn script(&self, item: &ItemRef) -> Rc<RefCell<Item>> {
        item.borrow().template.script.as_ref().unwrap().0.clone()
    }
    fn raw_create(&self, ty: &str) -> Option<ItemRef> {
        create_item_by_type(&FactoryEnv, &self.sm, ty, 1.0, true, &|_| None).map(shared::new_item)
    }
}

impl WornItemsInventoryHost for Host {
    fn create_item(&mut self, ty: &str) -> WornInventoryResult<Option<ItemRef>> {
        self.free_guards(None);
        self.log
            .push(format!("factory {ty} worn={}", self.worn.borrow().size()));
        if self.factory_error.as_deref() == Some(ty) {
            return Err(ScriptError::new("authored factory error").into());
        }
        let item = self.raw_create(ty);
        if let Some(item) = &item {
            self.made.push(Rc::downgrade(item));
            if let Some(parent) = self.create_parent.take() {
                shared::inventory_of(&item.borrow())
                    .unwrap()
                    .borrow_mut()
                    .parent = Some(parent);
            }
        }
        Ok(item)
    }
    fn copy_visual_and_sync(&mut self, item: &ItemRef, source: &ItemVisual) -> WornInventoryResult {
        self.free_guards(Some(item));
        self.log
            .push(format!("visual {}", item.borrow().get_full_type()));
        if self.sync_host_error_at
            == Some(
                self.log
                    .iter()
                    .filter(|line| line.starts_with("visual "))
                    .count(),
            )
        {
            return Err(RuntimeHostError::Borrowed("actual worn texture provider").into());
        }
        let mut item = item.borrow_mut();
        if let Some(visual) = item.get_visual(Some(&self.assets), &NoOwner, &NoVisualRng) {
            *visual = source.clone();
            item.synch_with_visual(Some(&self.assets), &NoOwner, &NoVisualRng);
        }
        Ok(())
    }
    fn body_location(
        &mut self,
        item: &ItemRef,
        clothing: bool,
    ) -> WornInventoryResult<Option<Rc<RegistryKey>>> {
        self.free_guards(Some(item));
        self.log.push(format!(
            "location {} clothing={clothing}",
            item.borrow().get_full_type()
        ));
        let result = if clothing {
            self.script(item).borrow().body_location.clone()
        } else {
            item.borrow().can_be_equipped()
        };
        Ok(result)
    }
    fn get_holes_number(&mut self, item: &ItemRef) -> WornInventoryResult<i32> {
        self.free_guards(Some(item));
        self.log
            .push(format!("holes {}", item.borrow().get_full_type()));
        let mut item = item.borrow_mut();
        if self.change_max_in_holes {
            item.condition_max = 15;
        }
        item.get_visual(Some(&self.assets), &NoOwner, &NoVisualRng)
            .map(|v| v.get_holes_number())
            .ok_or_else(super::super::npe)
            .map_err(Into::into)
    }
    fn set_condition_no_sound(&mut self, item: &ItemRef, condition: i32) -> WornInventoryResult {
        self.free_guards(Some(item));
        self.log.push(format!(
            "condition {} {condition}",
            item.borrow().get_full_type()
        ));
        let outcome = item
            .borrow_mut()
            .set_condition_no_sound(condition, &NoOwner, &Context);
        assert!(outcome.sounds.is_empty());
        self.free_guards(Some(item));
        if self.break_error && !outcome.on_break_calls.is_empty() {
            return Err(ScriptError::new("authored host condition failure").into());
        }
        Ok(())
    }
    fn add_item(&mut self, container: &ContainerRef, item: &ItemRef) -> WornInventoryResult {
        self.free_guards(Some(item));
        self.log
            .push(format!("add {}", item.borrow().get_full_type()));
        shared::add_item(container, item);
        if let Some(next) = self.append_on_add.take() {
            let clothing = matches!(
                next.borrow().kind,
                ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
            );
            let key = self.body_location(&next, clothing)?;
            self.worn.borrow_mut().set_item(key, Some(next))?;
        }
        Ok(())
    }
}

fn visual(ty: &str, holes: usize) -> ItemVisual {
    let mut v = ItemVisual::new(ty);
    v.tint = Some([0.2, 0.4, 0.6]);
    v.base_texture = 2;
    v.texture_choice = 3;
    v.hue = 0.75;
    v.decal = Some("authored-decal".into());
    for part in 0..holes {
        v.set_hole(part, 17);
    }
    v
}

#[test]
fn factory_alarm_clock_clothing_uses_actual_wrist_body_location() {
    let mut host = Host::new();
    let worn = host.worn.clone();
    set_from_item_visuals(&worn, &[visual("Base.Watch", 0)], &mut host).unwrap();
    let watch = worn.borrow().item_by_index(0).unwrap();
    assert!(matches!(
        watch.borrow().kind,
        ItemKind::AlarmClockClothing(..)
    ));
    assert_eq!(watch.borrow().registry_id, 76);
    assert!(host.script(&watch).borrow().can_be_equipped.is_none());
    let key = host.script(&watch).borrow().body_location.clone().unwrap();
    assert!(Rc::ptr_eq(
        worn.borrow().get(0).unwrap().location.as_ref().unwrap(),
        &key
    ));
    assert_eq!(key.location.to_string(), "test:wrist");
    assert!(host
        .log
        .iter()
        .any(|line| line == "location Base.Watch clothing=true"));
    assert!(Rc::ptr_eq(&host.made[0].upgrade().unwrap(), &watch));
}

#[test]
fn visual_conversion_uses_factory_real_registry_keys_and_same_inventory_items() {
    let mut h = Host::new();
    let worn = h.worn.clone();
    let visuals = [
        visual("Base.Bag", 1),
        visual("Base.Missing", 0),
        visual("Base.Shirt", 2),
    ];
    set_from_item_visuals(&worn, &visuals, &mut h).unwrap();
    assert_eq!(worn.borrow().size(), 2);
    let shirt = worn.borrow().item_by_index(0).unwrap();
    let bag = worn.borrow().item_by_index(1).unwrap();
    assert_eq!(shirt.borrow().registry_id, 71);
    assert_eq!(bag.borrow().registry_id, 73);
    assert_eq!(
        worn.borrow()
            .get(0)
            .unwrap()
            .location
            .as_ref()
            .unwrap()
            .location
            .to_string(),
        "test:chest"
    );
    assert_eq!(
        worn.borrow()
            .get(1)
            .unwrap()
            .location
            .as_ref()
            .unwrap()
            .location
            .to_string(),
        "test:back"
    );
    assert!(Rc::ptr_eq(&h.made[0].upgrade().unwrap(), &bag));
    assert!(Rc::ptr_eq(&h.made[1].upgrade().unwrap(), &shirt));
    let copied = shirt.borrow().visual.clone().unwrap();
    assert_eq!(copied.tint, visuals[2].tint);
    assert_eq!(
        (
            copied.base_texture,
            copied.texture_choice,
            copied.hue,
            copied.decal.clone()
        ),
        (2, 3, 0.75, Some("authored-decal".into()))
    );
    assert_eq!(copied.get_holes_number(), 2);
    assert_eq!(shirt.borrow().col, [0.2, 0.4, 0.6, 1.0]);
    assert_eq!(
        &h.log[..],
        [
            "factory Base.Bag worn=0",
            "visual Base.Bag",
            "location Base.Bag clothing=false",
            "factory Base.Missing worn=1",
            "factory Base.Shirt worn=1",
            "visual Base.Shirt",
            "location Base.Shirt clothing=true"
        ]
    );
    h.log.clear();
    let inventory = SharedContainer::default().into_ref();
    add_items_to_item_container(&worn, &inventory, &mut h).unwrap();
    assert_eq!(shirt.borrow().condition, 4);
    assert_eq!(bag.borrow().condition, 6);
    assert!(Rc::ptr_eq(&inventory.borrow().items[0], &shirt));
    assert!(Rc::ptr_eq(&inventory.borrow().items[1], &bag));
    assert!(Rc::ptr_eq(
        &shared::container_of(&shirt).unwrap(),
        &inventory
    ));
    assert_eq!(
        h.log,
        [
            "holes Base.Shirt",
            "condition Base.Shirt 4",
            "add Base.Shirt",
            "holes Base.Bag",
            "condition Base.Bag 6",
            "add Base.Bag"
        ]
    );
}

#[test]
fn conversion_clears_first_replaces_body_location_and_keeps_error_prefix() {
    let mut h = Host::new();
    let worn = h.worn.clone();
    set_from_item_visuals(&worn, &[visual("Base.Shirt", 0)], &mut h).unwrap();
    h.log.clear();
    set_from_item_visuals(
        &worn,
        &[visual("Base.Shirt", 0), visual("Base.Coat", 0)],
        &mut h,
    )
    .unwrap();
    assert_eq!(worn.borrow().size(), 1);
    assert_eq!(
        worn.borrow()
            .item_by_index(0)
            .unwrap()
            .borrow()
            .get_full_type(),
        "Base.Coat"
    );
    assert_eq!(h.log[0], "factory Base.Shirt worn=0");
    h.factory_error = Some("Base.Coat".into());
    assert!(set_from_item_visuals(
        &worn,
        &[
            visual("Base.Shirt", 0),
            visual("Base.Coat", 0),
            visual("Base.Bag", 0)
        ],
        &mut h
    )
    .is_err());
    assert_eq!(worn.borrow().size(), 1);
    assert_eq!(
        worn.borrow()
            .item_by_index(0)
            .unwrap()
            .borrow()
            .get_full_type(),
        "Base.Shirt"
    );
    // Non-clothing with no canBeEquipped passes null to real setItem: no
    // invented registry key, fallback slot, or silently skipped item.
    h.factory_error = None;
    assert!(set_from_item_visuals(
        &worn,
        &[visual("Base.Shirt", 0), visual("Base.Unwearable", 0)],
        &mut h
    )
    .is_err());
    assert_eq!(worn.borrow().size(), 1);
}

#[test]
fn null_visual_skips_copy_then_errors_before_container_insertion() {
    let mut h = Host::new();
    let worn = h.worn.clone();
    set_from_item_visuals(&worn, &[visual("Base.Invisible", 2)], &mut h).unwrap();
    let item = worn.borrow().item_by_index(0).unwrap();
    assert!(item.borrow().visual.is_none());
    let inventory = SharedContainer::default().into_ref();
    assert!(add_items_to_item_container(&worn, &inventory, &mut h).is_err());
    assert!(inventory.borrow().items.is_empty());
    assert_eq!(item.borrow().condition, 10);
}

#[test]
fn holes_clamp_condition_and_preserve_prefix_when_host_stops_after_break_outcome() {
    let mut h = Host::new();
    let worn = h.worn.clone();
    set_from_item_visuals(
        &worn,
        &[visual("Base.Coat", 5), visual("Base.Bag", 1)],
        &mut h,
    )
    .unwrap();
    h.break_error = true;
    let inventory = SharedContainer::default().into_ref();
    assert!(add_items_to_item_container(&worn, &inventory, &mut h).is_err());
    assert_eq!(
        worn.borrow().item_by_index(0).unwrap().borrow().condition,
        0
    );
    assert!(inventory.borrow().items.is_empty());
    assert_eq!(
        worn.borrow().item_by_index(1).unwrap().borrow().condition,
        9
    );
    h.break_error = false;
    add_items_to_item_container(&worn, &inventory, &mut h).unwrap();
    assert_eq!(inventory.borrow().items.len(), 2);
    assert_eq!(inventory.borrow().items[0].borrow().condition, 0);
    assert_eq!(inventory.borrow().items[1].borrow().condition, 6);
}

#[test]
fn container_loop_reads_live_worn_size_and_condition_max_after_holes() {
    let mut h = Host::new();
    let worn = h.worn.clone();
    set_from_item_visuals(&worn, &[visual("Base.Shirt", 2)], &mut h).unwrap();
    let bag = h.raw_create("Base.Bag").unwrap();
    h.copy_visual_and_sync(&bag, &visual("Base.Bag", 1))
        .unwrap();
    h.append_on_add = Some(bag.clone());
    h.change_max_in_holes = true;
    let inventory = SharedContainer::default().into_ref();
    add_items_to_item_container(&worn, &inventory, &mut h).unwrap();
    assert_eq!(inventory.borrow().items.len(), 2);
    assert_eq!(inventory.borrow().items[0].borrow().condition, 9);
    assert_eq!(bag.borrow().condition, 12);
    assert!(Rc::ptr_eq(&inventory.borrow().items[1], &bag));
}

#[test]
fn clear_and_same_location_eviction_release_opaque_parents_outside_worn_guards() {
    struct Probe {
        worn: std::rc::Weak<RefCell<WornItems<ItemRef>>>,
        drops: Rc<std::cell::Cell<usize>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            let worn = self.worn.upgrade().unwrap();
            let worn = worn
                .try_borrow_mut()
                .expect("item dropped under worn guard");
            assert!(worn.body_location_group().try_borrow_mut().is_ok());
            self.drops.set(self.drops.get() + 1);
        }
    }
    for clear in [false, true] {
        let mut h = Host::new();
        let worn = h.worn.clone();
        let drops = Rc::new(std::cell::Cell::new(0));
        h.create_parent = Some(Rc::new(Probe {
            worn: Rc::downgrade(&worn),
            drops: drops.clone(),
        }));
        if clear {
            set_from_item_visuals(&worn, &[visual("Base.Bag", 0)], &mut h).unwrap();
            assert_eq!(drops.get(), 0);
            set_from_item_visuals(&worn, &[visual("Base.Shirt", 0)], &mut h).unwrap();
        } else {
            set_from_item_visuals(
                &worn,
                &[visual("Base.Bag", 0), visual("Base.Bag", 0)],
                &mut h,
            )
            .unwrap();
        }
        assert_eq!(drops.get(), 1);
        assert_eq!(worn.borrow().size(), 1);
    }
}

#[test]
fn borrowed_worn_owner_fails_before_factory_without_changing_membership() {
    let mut h = Host::new();
    let worn = h.worn.clone();
    set_from_item_visuals(&worn, &[visual("Base.Shirt", 0)], &mut h).unwrap();
    let original = worn.borrow().item_by_index(0).unwrap();
    h.log.clear();
    let held = worn.borrow();
    assert!(set_from_item_visuals(&worn, &[visual("Base.Bag", 0)], &mut h).is_err());
    assert!(h.log.is_empty());
    assert!(Rc::ptr_eq(&held.item_by_index(0).unwrap(), &original));
    drop(held);
}

#[test]
fn archive_item_mapping_preserves_membership_after_group_rules_change() {
    let mut h = Host::new();
    let worn = h.worn.clone();
    set_from_item_visuals(
        &worn,
        &[visual("Base.Shirt", 0), visual("Base.Bag", 0)],
        &mut h,
    )
    .unwrap();
    let group = worn.borrow().body_location_group().clone();
    let chest = worn.borrow().get(0).unwrap().location.clone();
    let back = worn.borrow().get(1).unwrap().location.clone();
    group
        .borrow_mut()
        .set_exclusive(chest.clone(), back.clone())
        .unwrap();
    let mapped = worn
        .borrow()
        .try_map_items(|item| Ok::<_, ()>(item.borrow().registry_id as i32))
        .unwrap();
    assert!(Rc::ptr_eq(mapped.body_location_group(), &group));
    assert_eq!(mapped.size(), 2);
    assert_eq!(mapped.item_by_index(0), Some(71));
    assert_eq!(mapped.item_by_index(1), Some(73));
    assert!(Rc::ptr_eq(
        mapped.get(0).unwrap().location.as_ref().unwrap(),
        chest.as_ref().unwrap()
    ));
    assert!(Rc::ptr_eq(
        mapped.get(1).unwrap().location.as_ref().unwrap(),
        back.as_ref().unwrap()
    ));
    let calls = std::cell::Cell::new(0);
    let result = worn.borrow().try_map_items(|_| {
        calls.set(calls.get() + 1);
        Err::<(), _>("authored map error")
    });
    assert_eq!(result.unwrap_err(), "authored map error");
    assert_eq!(calls.get(), 1);
    assert_eq!(worn.borrow().size(), 2);
}

#[test]
fn typed_texture_host_error_keeps_first_worn_conversion_and_stops_later_factory() {
    let mut host = Host::new();
    let worn = host.worn.clone();
    host.sync_host_error_at = Some(2);
    let result = set_from_item_visuals(
        &worn,
        &[
            visual("Base.Shirt", 0),
            visual("Base.Coat", 0),
            visual("Base.Bag", 0),
        ],
        &mut host,
    );
    assert_eq!(
        result,
        Err(WornInventoryError::Host(RuntimeHostError::Borrowed(
            "actual worn texture provider"
        )))
    );
    assert_eq!(worn.borrow().size(), 1);
    assert_eq!(
        worn.borrow()
            .item_by_index(0)
            .unwrap()
            .borrow()
            .get_full_type(),
        "Base.Shirt"
    );
    assert!(host.log.iter().all(|line| !line.contains("Base.Bag")));
    assert!(worn.try_borrow_mut().is_ok());
    let mut script_host = Host::new();
    script_host.factory_error = Some("Base.Shirt".into());
    let script_worn = script_host.worn.clone();
    assert!(matches!(
        set_from_item_visuals(&script_worn, &[visual("Base.Shirt", 0)], &mut script_host),
        Err(WornInventoryError::Script(_))
    ));
}

//! Hand-authored items for the gameplay methods.

use std::collections::HashMap;
use std::rc::Rc;

use crate::assets::{BloodClothingTypes, ClothingItem, ClothingItems, ItemAssets};
use crate::context::{ItemContext, ItemOwner, ItemPlace, Loose, NoOwner};
use crate::gameplay::tags;
use crate::item::{ClothingPatch, InventoryItem, ItemKind};
use crate::lua_table::{LuaKey, LuaValue};
use crate::saved::SavedItem;
use crate::template::{ItemClass, ItemTemplate};
use crate::visual::NoVisualRng;
use crate::ItemContainer;

struct Ctx {
    scripts: HashMap<String, Rc<ItemTemplate>>,
    assets: ItemAssets,
    raining: bool,
    rain: f32,
    age: f64,
}

impl Ctx {
    fn new() -> Self {
        Ctx {
            scripts: HashMap::new(),
            assets: assets(),
            raining: false,
            rain: 0.0,
            age: 10.0,
        }
    }
    fn add(&mut self, t: ItemTemplate) -> Rc<ItemTemplate> {
        let t = Rc::new(t);
        self.scripts.insert(t.full_type(), t.clone());
        t
    }
}

impl ItemContext for Ctx {
    fn world_age_hours(&self) -> f64 {
        self.age
    }
    fn multiplier(&self) -> f32 {
        2.0
    }
    fn rain_intensity(&self) -> f32 {
        self.rain
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        0.3
    }
    fn basic_mask_defense(&self) -> f32 {
        25.0
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        85.0
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        100.0
    }
    fn script(&self, name: &str) -> Option<Rc<ItemTemplate>> {
        let full = if name.contains('.') {
            name.to_string()
        } else {
            format!("Base.{name}")
        };
        self.scripts.get(&full).cloned()
    }
    fn create_item(&self, full: &str) -> Option<InventoryItem> {
        let t = self.script(full)?;
        let mut i = InventoryItem::create(t, 1);
        i.id = 77;
        Some(i)
    }
    fn assets(&self) -> Option<&ItemAssets> {
        Some(&self.assets)
    }
}

/// Three body parts: Hand (0), Neck (1), Torso (2).
fn assets() -> ItemAssets {
    let mut clothing = ClothingItems::default();
    let mut shirt = ClothingItem::new("Shirt");
    shirt.texture_choices = vec!["a".into(), "b".into()];
    shirt.allow_random_tint = true;
    clothing.insert("Shirt", "game-1", Some(shirt));
    clothing.insert(
        "MaskNoFilter",
        "game-2",
        Some(ClothingItem::new("MaskNoFilter")),
    );
    clothing.insert("Mask", "game-3", Some(ClothingItem::new("Mask")));
    clothing.insert("Broken", "game-4", None);
    clothing.insert("Glove_R", "game-5", Some(ClothingItem::new("Glove_R")));
    ItemAssets {
        clothing_items: clothing,
        blood_clothing_types: BloodClothingTypes::new(
            vec![("Shirt".into(), vec![1, 2]), ("Hands".into(), vec![0])],
            vec!["Hand".into(), "Neck".into(), "Torso".into(), "MAX".into()],
        ),
    }
}

fn shirt() -> ItemTemplate {
    let mut t = ItemTemplate::new(ItemClass::Clothing, "Base", "Tshirt");
    t.clothing_item = Some("Shirt".into());
    t.blood_clothing_type = Some(vec!["Shirt".into()]);
    t.scratch_defense = 30.0;
    t.bite_defense = 120.0;
    t.bullet_defense = 5.0;
    t.neck_protection_modifier = 0.5;
    t.actual_weight = 0.4;
    t.display_name = "T-shirt".into();
    t
}

fn item(t: ItemTemplate) -> InventoryItem {
    let mut i = InventoryItem::create(Rc::new(t), 1);
    i.id = 5;
    i
}

struct Holder {
    equipped: bool,
    worn: bool,
    primary: bool,
    player: bool,
    place: ItemPlace,
}

impl ItemOwner for Holder {
    fn is_equipped(&self, _: &InventoryItem) -> bool {
        self.equipped
    }
    fn is_primary_hand_item(&self, _: &InventoryItem) -> bool {
        self.primary
    }
    fn is_worn(&self, _: &InventoryItem) -> bool {
        self.worn
    }
    fn is_in_player_inventory(&self, _: &InventoryItem) -> bool {
        self.player
    }
    fn place(&self, _: &InventoryItem) -> ItemPlace {
        self.place
    }
}

fn holder() -> Holder {
    Holder {
        equipped: false,
        worn: false,
        primary: false,
        player: true,
        place: ItemPlace::Character {
            sheltered_square: Some(false),
            in_tent: false,
            in_roofed_vehicle: false,
        },
    }
}

#[test]
fn visual_from_asset() {
    let ctx = Ctx::new();
    let mut i = item(shirt());
    let v = i.get_visual(ctx.assets(), &NoOwner, &NoVisualRng).unwrap();
    assert_eq!(v.clothing_item_name, "Shirt");
    assert_eq!(v.full_type, "Base.Tshirt");
    assert_eq!((v.texture_choice, v.base_texture, v.hue), (0, -1, 0.0));
    assert_eq!(v.tint, Some([1.0, 1.0, 1.0]));
    v.set_blood(2, 0.5, 3);
    // A missing or failed asset drops the visual.
    let mut t = shirt();
    t.clothing_item = Some("Broken".into());
    let mut b = item(t);
    b.visual = i.visual.clone();
    assert!(b.get_visual(ctx.assets(), &NoOwner, &NoVisualRng).is_none());
    assert!(b.visual.is_none());
    // synchWithVisual: blood and dirt levels over the covered parts.
    i.synch_with_visual(ctx.assets(), &NoOwner, &NoVisualRng);
    let ItemKind::Clothing(c) = &i.kind else {
        panic!()
    };
    assert_eq!(c.blood_level, (127.0 / 255.0) * 100.0 / 2.0);
    assert_eq!(i.get_covered_parts(&ctx.assets).unwrap(), vec![1, 2]);
}

#[test]
fn alternate_model_names() {
    let ctx = Ctx::new();
    let mut t = shirt();
    t.tags = vec![tags::GAS_MASK.into()];
    t.clothing_item = Some("Mask".into());
    t.without_drainable = Some("Base.MaskEmpty".into());
    t.without_drainable_clothing_item = Some("MaskNoFilter".into());
    let mut i = item(t);
    assert_eq!(
        i.get_alternate_model_name(ctx.assets(), &NoOwner),
        Some("WithoutDrainable")
    );
    assert_eq!(
        i.get_clothing_item(ctx.assets(), &NoOwner).unwrap().name,
        "MaskNoFilter"
    );
    i.set_filter_type("Base.Filter");
    assert!(i.has_filter());
    assert_eq!(i.get_alternate_model_name(ctx.assets(), &NoOwner), None);
    assert_eq!(
        i.get_visual(ctx.assets(), &NoOwner, &NoVisualRng)
            .unwrap()
            .clothing_item_name,
        "Mask"
    );
    let mut g = ItemTemplate::new(ItemClass::Normal, "Base", "Glove");
    g.replace_primary_hand = Some(Some("Glove_R".into()));
    let g = item(g);
    let h = Holder {
        primary: true,
        ..holder()
    };
    assert_eq!(g.get_alternate_model_name(None, &h), Some("RightHand"));
    assert_eq!(
        g.get_clothing_item(ctx.assets(), &h).unwrap().name,
        "Glove_R"
    );
    assert_eq!(g.get_alternate_model_name(None, &Loose), None);
}

#[test]
fn actual_weight_rules() {
    let mut ctx = Ctx::new();
    let mut i = item(shirt());
    assert_eq!(i.get_actual_weight(), 0.4);
    i.name = "Base.Tshirt".into();
    assert_eq!(i.get_actual_weight(), 0.0);
    // Weapons: script weight plus the parts' modifiers.
    let mut part = ItemTemplate::new(ItemClass::WeaponPart, "Base", "Scope");
    part.weight_modifier = 0.25;
    let mut w = item(ItemTemplate::new(ItemClass::Weapon, "Base", "Rifle"));
    w.name = "Base.Rifle".into();
    if let ItemKind::Weapon(d) = &mut w.kind {
        d.attachments.push(item(part));
    }
    assert_eq!(w.get_actual_weight(), 1.25);
    // Food: scaled by the hunger left, then by the empty item.
    let mut f = ItemTemplate::new(ItemClass::Food, "Base", "Soup");
    f.hunger_change = -20.0;
    f.actual_weight = 2.0;
    f.replace_on_use = Some("Bowl".into());
    let mut bowl = ItemTemplate::new(ItemClass::Normal, "Base", "Bowl");
    bowl.actual_weight = 0.5;
    ctx.add(bowl);
    let mut food = item(f);
    if let ItemKind::Food(d) = &mut food.kind {
        d.hung_change = -0.1;
    }
    assert_eq!(food.get_actual_weight_with(&ctx), (2.0 - 0.5) * 0.5 + 0.5);
    food.custom_weight = true;
    food.actual_weight = 0.9;
    assert_eq!(food.get_actual_weight_with(&ctx), 0.9);
}

#[test]
fn weights_and_equipped() {
    let mut ctx = Ctx::new();
    let mut ammo = ItemTemplate::new(ItemClass::Normal, "Base", "Bullets");
    ammo.actual_weight = 0.01;
    ctx.add(ammo);
    let mut mag = ItemTemplate::new(ItemClass::Normal, "Base", "Mag");
    mag.actual_weight = 0.2;
    ctx.add(mag);
    let mut g = ItemTemplate::new(ItemClass::Weapon, "Base", "Pistol");
    g.ammo_item = Some("Base.Bullets".into());
    g.magazine_type = Some("Base.Mag".into());
    let mut gun = item(g);
    gun.current_ammo_count = 10;
    if let ItemKind::Weapon(w) = &mut gun.kind {
        w.round_chambered = true;
        w.contains_clip = true;
    }
    let o = NoOwner;
    let contents = gun.get_contents_weight_with(&ctx, &o);
    assert!((contents - (0.01 + 0.2 + 0.1)).abs() < 1e-6);
    assert_eq!(gun.get_unequipped_weight(&ctx, &o), 1.0 + contents);
    assert_eq!(gun.get_equipped_weight(&ctx, &o), (1.0 + contents) * 0.3);
    assert_eq!(
        gun.get_hotbar_equipped_weight(&ctx, &o),
        (1.0 + contents) * 0.7
    );
    // A bag: contents with the weight reduction when equipped.
    let mut b = ItemTemplate::new(ItemClass::Container, "Base", "Bag");
    b.weight_reduction = 40;
    b.actual_weight = 1.0;
    let mut bag = item(b);
    if let ItemKind::Container(c) = &mut bag.kind {
        c.container
            .items
            .push(SavedItem::Item(Box::new(item(shirt()))));
    }
    assert_eq!(bag.get_unequipped_weight(&ctx, &o), 1.4);
    assert_eq!(bag.get_equipped_weight(&ctx, &o), 0.3 + 0.4 * 0.6);
    // Key rings are fake-equipped only directly in a player's inventory.
    let mut k = ItemTemplate::new(ItemClass::Container, "Base", "KeyRing");
    k.tags = vec![tags::KEY_RING.into()];
    let ring = item(k);
    assert!(ring.is_fake_equipped(&holder()));
    assert!(!ring.is_fake_equipped(&Holder {
        player: false,
        ..holder()
    }));
    assert!(!bag.is_fake_equipped(&holder()));
}

#[test]
fn condition_side_effects() {
    let ctx = Ctx::new();
    let mut t = shirt();
    t.break_sound = Some("Rip".into());
    t.damaged_sound = Some("Tear".into());
    t.on_break = Some("OnBreak_Shirt".into());
    let mut i = item(t);
    assert_eq!(i.get_current_condition(), 100.0);
    let h = Holder {
        worn: true,
        ..holder()
    };
    let o = i.set_condition(4, &h, &ctx);
    assert_eq!(
        (i.condition, o.sounds.clone()),
        (4, vec!["Tear".to_string()])
    );
    assert!(o.on_break_calls.is_empty() && !o.unwear_and_drop);
    assert_eq!(i.get_current_condition(), 40.0);
    let o = i.set_condition(-3, &h, &ctx);
    assert_eq!(i.condition, 0);
    assert_eq!(o.sounds, vec!["Rip".to_string()]);
    // Clothing breaks twice (setCondition, then setBroken again).
    assert_eq!(o.on_break_calls.len(), 2);
    assert!(o.unwear_and_drop && i.is_broken());
    assert_eq!(i.get_scratch_defense(), 0.0);
    assert_eq!(i.get_bite_defense(), 0.0);
    assert_eq!(i.get_corpse_sickness_defense(), 0.0);
    let o = i.set_condition(99, &NoOwner, &ctx);
    assert_eq!((i.condition, o.sounds.len()), (10, 0));
}

#[test]
fn def_for_part_with_patches() {
    let ctx = Ctx::new();
    let a = &ctx.assets;
    let mut i = item(shirt());
    assert!(i.get_def_for_part(2, false, false, a).is_err());
    i.get_visual(Some(a), &NoOwner, &NoVisualRng);
    assert_eq!(i.get_def_for_part(2, false, false, a), Ok(30.0));
    // Bite defense is capped at 100; the neck halves it.
    assert_eq!(i.get_def_for_part(2, true, false, a), Ok(100.0));
    assert_eq!(i.get_def_for_part(1, true, false, a), Ok(50.0));
    assert_eq!(i.get_def_for_part(2, false, true, a), Ok(5.0));
    let patch = |hole| ClothingPatch {
        tailor_lvl: 5,
        fabric_type: 2,
        scratch_defense: 5,
        bite_defense: 2,
        has_hole: hole,
        condition_gain: 0,
    };
    if let ItemKind::Clothing(c) = &mut i.kind {
        c.patches = Some(vec![(2, patch(false)), (1, patch(true))]);
    }
    assert_eq!(i.get_def_for_part(2, false, false, a), Ok(35.0));
    assert_eq!(i.get_def_for_part(2, false, true, a), Ok(7.0));
    assert_eq!(i.get_def_for_part(1, false, false, a), Ok(5.0));
    i.visual.as_mut().unwrap().set_hole(2, 3);
    assert_eq!(i.get_def_for_part(2, false, false, a), Ok(0.0));
}

#[test]
fn wetness() {
    let mut ctx = Ctx::new();
    let mut i = item(shirt());
    i.set_wetness(150.0);
    assert_eq!(i.get_wetness(), 100.0);
    // Equipped clothing is skipped unless ignored.
    let eq = Holder {
        equipped: true,
        ..holder()
    };
    if let ItemKind::Clothing(c) = &mut i.kind {
        c.last_wetness_update = 9.0;
    }
    i.set_wetness(50.0);
    i.update_wetness_with(false, &eq, &ctx);
    assert_eq!(i.get_wetness(), 50.0);
    // Dry outside: 20 per hour, twice when equipped.
    i.update_wetness_with(true, &eq, &ctx);
    assert_eq!(i.get_wetness(), 10.0);
    // Wetter in the rain when equipped outside.
    ctx.raining = true;
    ctx.rain = 0.5;
    ctx.age = 10.5;
    i.update_wetness_with(true, &eq, &ctx);
    assert_eq!(i.get_wetness(), 35.0);
    // Less than a minute: nothing.
    ctx.age = 10.51;
    i.update_wetness_with(true, &eq, &ctx);
    assert_eq!(i.get_wetness(), 35.0);
    // flushWetness: updates, then marks it not updated.
    ctx.age = 11.5;
    i.flush_wetness(&NoOwner, &ctx);
    assert_eq!(i.get_wetness(), 15.0);
    let ItemKind::Clothing(c) = &i.kind else {
        panic!()
    };
    assert_eq!(c.last_wetness_update, -1.0);
    // BreakWhenWet at 100.
    let mut t = shirt();
    t.tags = vec![tags::BREAK_WHEN_WET.into()];
    let mut p = item(t);
    p.set_wetness(100.0);
    p.update_wetness_with(true, &NoOwner, &ctx);
    assert_eq!(p.condition, 0);
    // Without a blood clothing type it dries at once.
    let mut t = shirt();
    t.blood_clothing_type = None;
    t.waterresist = 0.3;
    let mut d = item(t);
    d.set_wetness(80.0);
    assert_eq!(d.get_water_resistance(), 0.3);
    d.update_wetness();
    assert_eq!(d.get_wetness(), 0.0);
}

#[test]
fn gas_masks() {
    let mut ctx = Ctx::new();
    let mut filter = ItemTemplate::new(ItemClass::Drainable, "Base", "Filter");
    filter.use_delta = 0.1;
    ctx.add(filter);
    let mut t = shirt();
    t.tags = vec![tags::IMPROVISED_GAS_MASK.into()];
    t.corpse_sickness_defense = 10.0;
    let mut m = item(t);
    assert!(!m.has_filter());
    assert_eq!(m.get_corpse_sickness_defense_with(&ctx), 10.0);
    m.set_filter_type("Filter");
    assert_eq!(m.get_corpse_sickness_defense_with(&ctx), 25.0);
    m.set_used_delta(0.5);
    assert_eq!(m.get_corpse_sickness_defense_with(&ctx), 85.0);
    m.drain_gas_mask(1.0, &ctx).unwrap();
    assert!((m.get_used_delta() - 0.3).abs() < 1e-6);
    m.drain_gas_mask(10.0, &ctx).unwrap();
    assert_eq!(m.get_used_delta(), 0.0);
    assert_eq!(m.get_corpse_sickness_defense_with(&ctx), 25.0);
    m.set_filter_type("Missing");
    m.set_used_delta(1.0);
    assert!(m.drain_gas_mask(1.0, &ctx).is_err());
    // SCBA: drains while activated, then turns off.
    let mut tank = ItemTemplate::new(ItemClass::Drainable, "Base", "Tank");
    tank.use_delta = 500.0;
    ctx.add(tank);
    let mut t = shirt();
    t.tags = vec![tags::SCBA.into()];
    let mut s = item(t);
    assert!(!s.has_tank());
    s.set_tank_type("Base.Tank");
    s.activated = true;
    s.set_used_delta(1.5);
    s.drain_scba(&ctx).unwrap();
    assert!(s.has_tank() && s.activated);
    assert!((s.get_used_delta() - 0.5).abs() < 1e-6);
    s.drain_scba(&ctx).unwrap();
    assert_eq!(s.get_used_delta(), 0.0);
    assert!(!s.activated);
    // getUsedDelta writes 0 when missing.
    let mut n = item(shirt());
    assert_eq!(n.get_used_delta(), 0.0);
    let t = n.table.as_ref().unwrap();
    assert_eq!(
        t.rawget(&LuaKey::Str("usedDelta".into())),
        Some(&LuaValue::Num(0.0))
    );
}

#[test]
fn use_drainables_and_replacements() {
    let mut ctx = Ctx::new();
    // A tissue: a drainable whose last use leaves it at -1 when the
    // caller already took a use off.
    let mut t = ItemTemplate::new(ItemClass::Drainable, "Base", "Tissue");
    t.use_delta = 0.25;
    let mut tissue = item(t);
    tissue.uses = 1;
    tissue.set_current_uses(tissue.get_current_uses() - 1);
    let out = tissue.use_();
    assert_eq!(tissue.uses, -1);
    assert!(out.remove && out.remove_from_hands);
    // ReplaceOnDeplete: the empty item replaces it in the container.
    let mut empty = ItemTemplate::new(ItemClass::Normal, "Base", "EmptyBottle");
    empty.actual_weight = 0.1;
    ctx.add(empty);
    let mut b = ItemTemplate::new(ItemClass::Drainable, "Base", "Bleach");
    b.replace_on_deplete = Some("EmptyBottle".into());
    b.use_delta = 0.5;
    b.actual_weight = 1.1;
    let mut bleach = item(b);
    bleach.uses = 2;
    bleach.condition = 7;
    let mut c = ItemContainer::new(Some("none"));
    c.items.push(SavedItem::Item(Box::new(bleach)));
    let out = c.use_item(5, &NoOwner, &ctx).unwrap();
    assert!(out.added.is_none() && !out.remove);
    let left = c.items[0].as_item().unwrap();
    assert_eq!(left.uses, 1);
    assert!(left.custom_weight);
    assert!((left.actual_weight - 0.6).abs() < 1e-6);
    let out = c.use_item(5, &NoOwner, &ctx).unwrap();
    assert!(out.remove && out.added.as_ref().unwrap().takes_hands);
    assert_eq!(c.items.len(), 1);
    let new = c.items[0].as_item().unwrap();
    assert_eq!((new.get_type(), new.condition), ("EmptyBottle", 7));
    // Plain items: ReplaceOnUse adds the item; KeepOnDeplete keeps it.
    let mut p = ItemTemplate::new(ItemClass::Normal, "Base", "Pills");
    p.replace_on_use = Some("EmptyBottle".into());
    let mut pills = item(p);
    pills.color_red = 0.5;
    let out = pills.use_with(false, false, false, &NoOwner, &ctx);
    assert_eq!(pills.uses, 0);
    let added = out.added.unwrap().item.unwrap();
    assert!(added.custom_color && added.color_red == 0.5);
    assert!(out.remove);
    let mut k = ItemTemplate::new(ItemClass::Normal, "Base", "Kit");
    k.disappear_on_use = false;
    let mut kit = item(k);
    assert!(kit.use_in(&NoOwner, &ctx).ignored);
    assert_eq!(kit.uses, 1);
}

#[test]
fn weapon_types() {
    let w = |swing: &str, two: bool, ranged: bool, type_: &str| {
        let mut t = ItemTemplate::new(ItemClass::Weapon, "Base", type_);
        t.swing_anim = Some(swing.into());
        t.two_hand_weapon = two;
        t.ranged = ranged;
        item(t).get_weapon_type()
    };
    assert_eq!(w("Stab", false, false, "Knife"), Some("KNIFE"));
    assert_eq!(w("heavy", true, false, "Sledge"), Some("HEAVY"));
    assert_eq!(w("Throw", false, true, "Molotov"), Some("THROWING"));
    assert_eq!(w("Rifle", true, true, "Rifle"), Some("FIREARM"));
    assert_eq!(w("Handgun", false, true, "Pistol"), Some("HANDGUN"));
    assert_eq!(w("Spear", true, false, "Spear"), Some("SPEAR"));
    assert_eq!(w("Bat", true, false, "Chainsaw"), Some("CHAINSAW"));
    assert_eq!(w("Bat", true, false, "Bat"), Some("TWO_HANDED"));
    assert_eq!(w("Bat", false, false, "Pipe"), Some("ONE_HANDED"));
    assert_eq!(item(shirt()).get_weapon_type(), None);
}

#[test]
fn script_flags() {
    let mut t = shirt();
    t.body_location = Some("base:tshirt".into());
    t.equipped_no_sprint = true;
    t.protect_from_rain_when_equipped = true;
    t.tags = vec![tags::SPIKED.into()];
    let i = item(t);
    assert_eq!(i.get_body_location(), Some("base:tshirt"));
    assert!(i.is_body_location("Base:TShirt"));
    assert!(i.is_equipped_no_sprint() && i.is_protect_from_rain_while_equipped());
    assert_eq!(i.get_blood_clothing_type().unwrap(), ["Shirt".to_string()]);
    assert!(i.is_spiked_front() && !i.is_spiked_behind());
}

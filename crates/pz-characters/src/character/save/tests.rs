//! `IsoPlayer.load`/`save` with invented data: the Java field names are the
//! original's code symbols, every id, item record and number is made up.

use std::rc::Rc;

use pz_inventory::env::ItemEnv;
use pz_inventory::io::{Reader, Writer};
use pz_inventory::lua_table::{LuaKey, LuaTable, LuaValue};
use pz_inventory::saved::SavedItem;
use pz_scripts::ScriptManager;
use pz_world::save::visual::HumanVisual;
use pz_world::save::SaveTables;

use super::super::tests::{item_constant, player, Recorder};
use super::super::thermoregulator::body_tests::body_fixture;
use super::super::*;
use super::*;
use crate::Characters;

/// Items 1..=3 exist (`Base.Thing<n>`); item 9 cannot be created.
struct Env;

impl ItemEnv for Env {
    fn exists(&self, registry_id: i16) -> bool {
        registry_id != 9
    }
    fn full_type(&self, registry_id: i16) -> Option<String> {
        (1..=3).contains(&registry_id).then(|| format!("Base.Thing{registry_id}"))
    }
    fn registry_id(&self, full_type: &str) -> i16 {
        full_type.strip_prefix("Base.Thing").and_then(|n| n.parse().ok()).unwrap_or(-1)
    }
}

/// A raw item record: registry id, save type, id, payload.
fn raw(reg: i16, id: i32, payload: u8) -> SavedItem {
    let mut d = reg.to_be_bytes().to_vec();
    d.push(0);
    d.extend_from_slice(&id.to_be_bytes());
    d.extend_from_slice(&[payload, 1, 2]);
    SavedItem::Raw(d)
}

fn setup() -> (ScriptManager, Characters, Vec<Rc<pz_scripts::registry::RegistryKey>>) {
    let (sm, mut chars) = body_fixture();
    let locs: Vec<_> =
        ["HAT", "SHIRT", "SHOES"].iter().map(|f| item_constant(&sm, "item_body_location", f)).collect();
    let g = chars.body_locations.get_group("Human").unwrap();
    for l in &locs {
        g.borrow_mut().get_or_create_location(Some(l.clone())).unwrap();
    }
    (sm, chars, locs)
}

fn tables() -> SaveTables {
    SaveTables { factory: vec![(0, "IsoObject".into()), (1, "Player".into())], blood_body_parts: 18, character_stats: Vec::new() }
}

/// A player with something in every saved field.
fn dressed(sm: &ScriptManager, chars: &mut Characters, ctx: &Recorder, locs: &[Rc<pz_scripts::registry::RegistryKey>]) -> GameCharacter<()> {
    let mut c = player(sm, chars);
    (c.vars.x, c.vars.y, c.vars.z) = (10_123.5, 9_876.25, 1.0);
    c.vars.dir = pz_world::square::IsoDirections::SW;
    let mut t = LuaTable::default();
    t.rawset(LuaKey::Str("k".into()), Some(LuaValue::Num(2.5)));
    c.mod_data = Some(t);
    {
        let mut d = c.descriptor.as_ref().unwrap().borrow_mut();
        d.forename = Some("Ann".into());
        d.surname = Some("Bee".into());
        d.extra.push("Extra1".into());
        d.xp_boost_map.push(("Axe".into(), 2));
        d.voice_type = 2;
    }
    let mut v = HumanVisual::new(tables().blood_body_parts);
    v.hair_model = Some("Bob".into());
    c.human_visual = Some(v);
    // Two identical things (one run on save), another type, and a third.
    c.items.inventory.borrow_mut().raw_items = vec![raw(1, 101, 7), raw(2, 102, 8), raw(1, 103, 7), raw(3, 104, 9)];
    c.items.worn_items = vec![101, 104];
    c.items.worn_locations = vec![(101, locs[0].clone()), (104, locs[2].clone())];
    c.items.primary_hand_item = Some(102);
    c.items.secondary_hand_item = Some(103);
    c.asleep = true;
    c.force_wake_up_time = 7.5;
    let pain = chars.sim.character_stats.field("PAIN").unwrap();
    c.stats.set(&pain, 12.0);
    c.body_damage.parts[0].bitten = true;
    c.xp.total_xp = 3.0;
    c.xp.level = 1;
    let axe = chars.perks.constant("Axe").unwrap();
    c.xp.xp_map.push((axe.clone(), 15.0));
    c.perk_list.push(PerkInfo { level: 2, perk: axe.clone() });
    c.xp.add_xp_multiplier(&axe, 1.5, 1, 4);
    c.character_traits.add(pz_scripts::objects::items::CharacterTrait::constant(sm, "STRONG").as_ref()).unwrap();
    c.depress_effect = 0.5;
    c.read_books.push(ReadBook { full_type: "Base.Book".into(), already_read_pages: 12 });
    c.known_recipes.push("MakeThing".into());
    c.last_hour_sleeped = 5;
    c.beard_grow_timing = 3.0;
    c.environment.sneaking = true;
    c.read_literature.insert("Title".into(), 4);
    c.read_print_media.insert("flyer".into(), ());
    c.last_animal_pet = 99;
    c.cheats.set(ctx, "GOD_MODE", true);
    c.hours_survived = 26.5;
    c.zombie_kills = 3;
    c.survivor_kills = 1;
    c.nutrition.as_mut().unwrap().set_calories(1234.0);
    let p = &mut c.player_fields;
    p.tag_prefix = "T".into();
    p.display_name = Some("Ann Bee".into());
    p.auto_drink = false;
    p.mechanics_item = vec![(5, 6), (1 << 40, 7)];
    p.already_read_book = vec!["Base.Thing2".into()];
    p.craft_history.insert("Recipé".into(), CraftHistoryEntry { craft_count: 2, last_craft_time: 1.5 });
    c.known_media_lines.insert("RM_1".into(), ());
    c.saved_vehicle = Some(SavedVehicle { x: 1.5, y: 2.5, seat: 1, running: true });
    c
}

fn save(sm: &ScriptManager, chars: &Characters, ctx: &Recorder, c: &mut GameCharacter<()>) -> Vec<u8> {
    let (items, tables) = (Env, tables());
    let env = SaveEnv { items: &items, tables: &tables };
    let mut w = Writer::new();
    c.save_player(sm, chars, ctx, &env, &mut w).unwrap();
    w.buf
}

fn load(sm: &ScriptManager, chars: &mut Characters, ctx: &Recorder, data: &[u8]) -> GameCharacter<()> {
    let (items, tables) = (Env, tables());
    let env = SaveEnv { items: &items, tables: &tables };
    let mut c = player(sm, chars);
    let mut r = Reader::new(data);
    c.load_player(sm, chars, ctx, &env, &mut r, 249, &mut |_| panic!("not on fire")).unwrap();
    assert_eq!(r.remaining(), 0);
    c
}

#[test]
fn player_record_round_trips() {
    let (sm, mut chars, locs) = setup();
    let ctx = Recorder { debug: true, ..Default::default() };
    let mut c = dressed(&sm, &mut chars, &ctx, &locs);
    let data = save(&sm, &chars, &ctx, &mut c);
    // The written order groups the two identical things.
    assert_eq!(c.saved_inventory_items, [Some(101), Some(103), Some(102), Some(104)]);
    let mut back = load(&sm, &mut chars, &ctx, &data);
    assert_eq!((back.vars.x, back.vars.y), (10_123.5, 9_876.25));
    assert_eq!(back.vars.dir, pz_world::square::IsoDirections::SW);
    assert_eq!(back.mod_data, c.mod_data);
    {
        let d = back.descriptor.as_ref().unwrap().borrow();
        assert_eq!((d.forename.as_deref(), d.surname.as_deref()), (Some("Ann"), Some("Bee")));
        assert_eq!((d.extra.clone(), d.xp_boost_map.clone(), d.voice_type), (vec!["Extra1".to_string()], vec![("Axe".to_string(), 2)], 2));
    }
    assert_eq!(back.human_visual, c.human_visual);
    let ids: Vec<i32> = back.items.inventory.borrow().to_container().items.iter().map(|i| i.id()).collect();
    assert_eq!(ids, [101, 103, 102, 104]);
    assert_eq!(back.items.worn_items, [101, 104]);
    assert!(Rc::ptr_eq(back.items.worn_location(104).unwrap(), &locs[2]));
    // IsoPlayer's short indices into the saved list win.
    assert_eq!((back.items.primary_hand_item, back.items.secondary_hand_item), (Some(102), Some(103)));
    assert!(back.asleep && back.body_damage.parts[0].bitten);
    assert_eq!(back.stat(&chars, "PAIN").unwrap(), 12.0);
    assert_eq!((back.xp.total_xp, back.xp.level, back.perk_list[0].level), (3.0, 1, 2));
    assert_eq!(back.xp.multiplier_entry(&chars.perks.constant("Axe").unwrap()), Some(XPMultiplier { multiplier: 1.5, min_level: 1, max_level: 4 }));
    assert!(back.has_trait(&sm, "STRONG").unwrap());
    // Loading the perk level differs from the empty list before.
    assert!(ctx.events.borrow().iter().any(|e| e.starts_with("LogLevelPerk")));
    assert_eq!(back.read_books, c.read_books);
    assert_eq!((back.known_recipes.clone(), back.last_hour_sleeped, back.beard_grow_timing), (vec!["MakeThing".to_string()], 5, 3.0));
    assert!(back.environment.sneaking);
    assert_eq!(back.read_literature.get("Title"), Some(&4));
    assert!(back.read_print_media.contains_key("flyer") && back.known_media_lines.contains_key("RM_1"));
    // The saved cheat set comes back through PlayerCheats.load.
    assert_eq!(back.cheats.names(), ["GOD_MODE"]);
    assert_eq!((back.hours_survived, back.zombie_kills, back.survivor_kills), (26.5, 3, 1));
    assert_eq!(back.nutrition.as_ref().unwrap().calories(), 1234.0);
    let p = &back.player_fields;
    assert_eq!((p.tag_prefix.as_str(), p.display_name.as_deref(), p.auto_drink), ("T", Some("Ann Bee"), false));
    // In HashMap<Long, Long> order.
    assert_eq!(p.mechanics_item, [(1 << 40, 7), (5, 6)]);
    assert_eq!(p.already_read_book, ["Base.Thing2"]);
    assert_eq!(p.craft_history.get("Recipé"), Some(&CraftHistoryEntry { craft_count: 2, last_craft_time: 1.5 }));
    // In a vehicle: the player's height is reset.
    assert_eq!(back.saved_vehicle, c.saved_vehicle);
    assert_eq!(back.vars.z, 0.0);
    // `IsoGameCharacter.save` writes the hand items' indices in the item
    // list, which the load leaves in saved (grouped by type) order, so the
    // second record differs there; IsoPlayer's indices into the saved list
    // still give the same hands, and from then on the record is stable.
    back.vars.z = 1.0;
    let again = save(&sm, &chars, &ctx, &mut back);
    assert_ne!(again, data);
    let mut back2 = load(&sm, &mut chars, &ctx, &again);
    assert_eq!((back2.items.primary_hand_item, back2.items.secondary_hand_item), (Some(102), Some(103)));
    back2.vars.z = 1.0;
    assert_eq!(save(&sm, &chars, &ctx, &mut back2), again);
}

#[test]
fn saved_indices_count_items_that_cannot_load() {
    let (sm, mut chars, locs) = setup();
    let ctx = Recorder::default();
    let mut c = dressed(&sm, &mut chars, &ctx, &locs);
    // The second record (type 9) is dropped on load but keeps its slot in
    // savedInventoryItems.
    c.items.inventory.borrow_mut().raw_items = vec![raw(1, 101, 7), raw(9, 102, 8), raw(3, 104, 9)];
    c.items.secondary_hand_item = Some(102);
    let data = save(&sm, &chars, &ctx, &mut c);
    let back = load(&sm, &mut chars, &ctx, &data);
    assert_eq!(back.saved_inventory_items, [Some(101), None, Some(104)]);
    let ids: Vec<i32> = back.items.inventory.borrow().to_container().items.iter().map(|i| i.id()).collect();
    assert_eq!(ids, [101, 104]);
    assert_eq!(back.items.worn_items, [101, 104]);
    assert_eq!((back.items.primary_hand_item, back.items.secondary_hand_item), (None, None));
    // Without debug mode the cheat setters and the cheat set do nothing.
    assert!(!back.is_god_mod(&ctx));
}

#[test]
fn worn_items_need_a_known_location() {
    let (sm, mut chars, locs) = setup();
    let ctx = Recorder::default();
    let mut c = dressed(&sm, &mut chars, &ctx, &locs);
    let data = save(&sm, &chars, &ctx, &mut c);
    // A group without the hat location drops the hat.
    chars.body_locations.reset();
    let g = chars.body_locations.get_group("Human").unwrap();
    g.borrow_mut().get_or_create_location(Some(locs[2].clone())).unwrap();
    let back = load(&sm, &mut chars, &ctx, &data);
    assert_eq!(back.items.worn_items, [104]);
}

#[test]
fn thermoregulator_reads_older_layouts() {
    let (sm, _chars) = body_fixture();
    let mut t = Thermoregulator::new(&sm);
    let node = t.nodes()[1].body_part_index() as i32;
    // World version 240: no metabolicRateReal, thermalDuration or the
    // per-node insulation, wind and wetness.
    let mut w = Writer::new();
    for f in [36.0f32, 2.0, 3.0, 0.1, 0.2, 0.3, 0.4] {
        w.f32(f);
    }
    w.i32(2);
    w.i32(node);
    for f in [30.0f32, 31.0, 0.5, 0.6, 0.7] {
        w.f32(f);
    }
    // A body part without a node is skipped.
    w.i32(-1);
    for f in [1.0f32; 5] {
        w.f32(f);
    }
    let mut r = Reader::new(&w.buf);
    t.load(&mut r, 240).unwrap();
    assert_eq!(r.remaining(), 0);
    assert_eq!((t.set_point(), t.metabolic_target()), (36.0, 3.0));
    let n = &t.nodes()[1];
    assert_eq!((n.celcius(), n.skin_celcius(), n.secondary_delta(), n.insulation()), (30.0, 31.0, 0.7, 0.0));
}

#[test]
fn survivor_desc_before_voices() {
    let (sm, mut chars) = body_fixture();
    let mut d: crate::SurvivorDesc<()> = crate::SurvivorDesc::new(&mut chars, &sm).unwrap();
    d.id = 40;
    d.character_gender = crate::CharacterGender::Male;
    let mut w = Writer::new();
    d.save(&mut w).unwrap();
    // World version 207 has no voice fields: drop them from the record.
    let n = w.buf.len() - (2 + d.voice_prefix.as_ref().unwrap().len() + 8);
    let mut back: crate::SurvivorDesc<()> = crate::SurvivorDesc::new(&mut chars, &sm).unwrap();
    let mut r = Reader::new(&w.buf[..n]);
    back.load(&sm, &mut chars, &mut r, 207).unwrap();
    assert_eq!(r.remaining(), 0);
    assert_eq!((back.id, back.voice_prefix.as_deref(), back.voice_pitch), (40, Some("VoiceMale"), 0.0));
    assert!((0..3).contains(&back.voice_type));
    assert!(chars.survivor_desc.id_count >= 40);
}

#[test]
fn burning_player_is_set_on_fire_during_load() {
    let (sm, mut chars, locs) = setup();
    let ctx = Recorder::default();
    let mut c = dressed(&sm, &mut chars, &ctx, &locs);
    c.vars.on_fire = true;
    let data = save(&sm, &chars, &ctx, &mut c);
    let (items, tables) = (Env, tables());
    let env = SaveEnv { items: &items, tables: &tables };
    let mut back = player(&sm, &mut chars);
    let mut fired = None;
    back.load_player(&sm, &mut chars, &ctx, &env, &mut Reader::new(&data), 249, &mut |c| fired = Some(c.asleep)).unwrap();
    // Called after the IsoGameCharacter fields before it are loaded.
    assert_eq!(fired, Some(true));
}


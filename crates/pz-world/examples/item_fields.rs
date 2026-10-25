//! Decodes every item of a save's chunk files and prints its fields in
//! the format of a local harness that dumps the same fields from the
//! original game's loaded items, then compares the two dumps.
//!
//! ```text
//! item_fields <game dir> <sprite tables> <save tables> <save dir> <oracle items_fields.txt>
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::{parse_item_names, ScriptItems};
use pz_inventory::item::{InventoryItem, ItemKind};
use pz_inventory::{ItemContainer, ItemEnv, SavedItem};
use pz_world::objects::ObjectKind;
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{IsoSpriteManager, SpriteTables};

fn f(v: f32) -> String {
    format!("f{:x}", v.to_bits())
}

fn b(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

fn s(v: Option<&str>) -> String {
    v.unwrap_or("null").to_string()
}

fn line(i: &InventoryItem) -> String {
    let mut o = format!("{} {}", i.class_name(), i.get_full_type());
    let key_id = match &i.kind {
        ItemKind::Key { key_id, .. } => *key_id,
        _ => i.key_id,
    };
    let base: Vec<(&str, String)> = vec![
        ("id", i.id.to_string()),
        ("uses", i.uses.to_string()),
        ("condition", i.condition.to_string()),
        ("conditionMax", i.condition_max.to_string()),
        ("customColor", b(i.custom_color).into()),
        ("itemCapacity", f(i.item_capacity)),
        ("activated", b(i.activated).into()),
        ("haveBeenRepaired", i.have_been_repaired.to_string()),
        ("name", i.name.clone()),
        ("customName", b(i.custom_name).into()),
        ("customWeight", b(i.custom_weight).into()),
        ("actualWeight", f(i.actual_weight)),
        ("keyId", key_id.to_string()),
        ("remoteControlId", i.remote_control_id.to_string()),
        ("remoteRange", i.remote_range.to_string()),
        ("colorRed", f(i.color_red)),
        ("colorGreen", f(i.color_green)),
        ("colorBlue", f(i.color_blue)),
        ("worker", s(i.worker.as_deref())),
        ("wetCooldown", f(i.wet_cooldown)),
        ("favorite", b(i.favorite).into()),
        ("stashMap", s(i.stash_map.as_deref())),
        ("zombieInfected", b(i.zombie_infected).into()),
        ("currentAmmoCount", i.current_ammo_count.to_string()),
        ("attachedSlot", i.attached_slot.to_string()),
        ("attachedSlotType", s(i.attached_slot_type.as_deref())),
        ("attachedToModel", s(i.attached_to_model.as_deref())),
        ("maxCapacity", i.max_capacity.to_string()),
        ("recordedMediaIndex", i.recorded_media_index.to_string()),
        ("worldScale", f(i.world_scale)),
        ("isInitialised", b(i.is_initialised).into()),
        ("modelIndex", i.model_index.to_string()),
        ("worldXRotation", f(i.world_x_rotation)),
        ("worldYRotation", f(i.world_y_rotation)),
        ("worldZRotation", f(i.world_z_rotation)),
        ("type", i.type_.clone()),
    ];
    for (k, v) in base {
        write!(o, " {k}={v}").unwrap();
    }
    let mut sub: Vec<(&str, String)> = Vec::new();
    let clothing = |c: &pz_inventory::item::ClothingData, sub: &mut Vec<(&str, String)>| {
        sub.push(("spriteName", s(c.sprite_name.as_deref())));
        sub.push(("dirtyness", f(c.dirtyness)));
        sub.push(("bloodLevel", f(c.blood_level)));
        sub.push(("wetness", f(c.wetness)));
        sub.push(("lastWetnessUpdate", f(c.last_wetness_update)));
    };
    let alarm = |a: &pz_inventory::item::AlarmData, sub: &mut Vec<(&str, String)>| {
        sub.push(("alarmHour", a.alarm_hour.to_string()));
        sub.push(("alarmMinutes", a.alarm_minutes.to_string()));
        sub.push(("alarmSet", b(a.alarm_set).into()));
        sub.push(("ringSince", f(a.ring_since)));
        sub.push(("forceDontRing", a.force_dont_ring.to_string()));
    };
    match &i.kind {
        ItemKind::Food(x) => {
            for (k, v) in [
                ("age", f(x.age)),
                ("lastAged", f(x.last_aged)),
                ("calories", f(x.calories)),
                ("proteins", f(x.proteins)),
                ("lipids", f(x.lipids)),
                ("carbohydrates", f(x.carbohydrates)),
                ("hungChange", f(x.hung_change)),
                ("baseHunger", f(x.base_hunger)),
                ("unhappyChange", f(x.unhappy_change)),
                ("boredomChange", f(x.boredom_change)),
                ("thirstChange", f(x.thirst_change)),
                ("heat", f(x.heat)),
                ("lastCookMinute", x.last_cook_minute.to_string()),
                ("cookingTime", f(x.cooking_time)),
                ("cooked", b(x.cooked).into()),
                ("burnt", b(x.burnt).into()),
                ("isCookable", b(x.is_cookable).into()),
                ("dangerousUncooked", b(x.dangerous_uncooked).into()),
                ("poisonDetectionLevel", x.poison_detection_level.to_string()),
                ("poisonPower", x.poison_power.to_string()),
                ("chef", s(x.chef.as_deref())),
                ("offAge", x.off_age.to_string()),
                ("offAgeMax", x.off_age_max.to_string()),
                ("painReduction", f(x.pain_reduction)),
                ("fluReduction", x.flu_reduction.to_string()),
                ("foodSicknessChange", x.food_sickness_change.to_string()),
                ("poison", b(x.poison).into()),
                ("useForPoison", x.use_for_poison.to_string()),
                ("freezingTime", f(x.freezing_time)),
                ("frozen", b(x.frozen).into()),
                ("lastFrozenUpdate", f(x.last_frozen_update)),
                ("rottenTime", f(x.rotten_time)),
                ("compostTime", f(x.compost_time)),
                ("cookedInMicrowave", b(x.cooked_in_microwave).into()),
                ("fatigueChange", f(x.fatigue_change)),
                ("endChange", f(x.end_change)),
                ("stressChange", f(x.stress_change)),
                ("milkQty", x.milk_qty.to_string()),
            ] {
                sub.push((k, v));
            }
        }
        ItemKind::Clothing(c) => clothing(c, &mut sub),
        ItemKind::AlarmClockClothing(c, a) => {
            clothing(c, &mut sub);
            alarm(a, &mut sub);
        }
        ItemKind::AlarmClock(a) => alarm(a, &mut sub),
        ItemKind::Weapon(w) => {
            for (k, v) in [
                ("maxRange", f(w.max_range)),
                ("minRangeRanged", f(w.min_range_ranged)),
                ("clipSize", w.clip_size.to_string()),
                ("minDamage", f(w.min_damage)),
                ("maxDamage", f(w.max_damage)),
                ("recoilDelay", w.recoil_delay.to_string()),
                ("aimingTime", w.aiming_time.to_string()),
                ("reloadTime", w.reload_time.to_string()),
                ("hitChance", w.hit_chance.to_string()),
                ("minAngle", f(w.min_angle)),
                ("fireMode", s(w.fire_mode.as_deref())),
                ("cyclicRateMultiplier", f(w.cyclic_rate_multiplier)),
                ("explosionTimer", w.explosion_timer.to_string()),
                ("maxAngle", f(w.max_angle)),
                ("bloodLevel", f(w.blood_level)),
                ("containsClip", b(w.contains_clip).into()),
                ("roundChambered", b(w.round_chambered).into()),
                ("isJammed", b(w.is_jammed).into()),
                ("weaponSprite", s(w.weapon_sprite.as_deref())),
                ("minSightRange", f(w.min_sight_range)),
                ("maxSightRange", f(w.max_sight_range)),
            ] {
                sub.push((k, v));
            }
        }
        ItemKind::Literature(l) => {
            sub.push(("numberOfPages", l.number_of_pages.to_string()));
            sub.push(("alreadyReadPages", l.already_read_pages.to_string()));
            sub.push(("canBeWrite", b(l.can_be_write).into()));
            sub.push(("lockedBy", s(l.locked_by.as_deref())));
        }
        ItemKind::Key { number_of_key, .. } => sub.push(("numberOfKey", number_of_key.to_string())),
        ItemKind::Moveable(m) | ItemKind::Radio(m, _) => {
            sub.push(("worldSprite", m.world_sprite.clone()));
            sub.push(("isLight", b(m.light.is_some()).into()));
        }
        ItemKind::Container(c) => sub.push(("weightReduction", c.weight_reduction.to_string())),
        ItemKind::Map { map_id, .. } => sub.push(("mapId", map_id.clone())),
        ItemKind::Plain => {}
    }
    for (k, v) in sub {
        write!(o, " {k}={v}").unwrap();
    }
    write!(
        o,
        " md={} visual={}",
        i.table.as_ref().map_or(-1, |t| t.entries.len() as i64),
        b(i.visual.is_some())
    )
    .unwrap();
    if let ItemKind::Container(c) = &i.kind {
        write!(
            o,
            " contId={} contItems={}",
            c.container.id,
            c.container.items.len()
        )
        .unwrap();
    }
    o
}

fn dump(items: &[SavedItem], out: &mut Vec<String>, raw: &mut usize) {
    for it in items {
        match it {
            SavedItem::Raw(_) => *raw += 1,
            SavedItem::Item(i) => {
                if let ItemKind::Container(c) = &i.kind {
                    dump(&c.container.items, out, raw);
                }
                out.push(line(i));
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let oracle = std::fs::read_to_string(&a[5]).unwrap();
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .unwrap();
    sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
        .unwrap();
    let names = parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let dict = WorldDictionary::load(&std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap())
        .unwrap();
    let mut env = ScriptItems::new(&sm, dict, &names);
    env.blood_body_parts = save_tables.blood_body_parts;
    env.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    // Component blocks through pz-entity (set ITEM_COMPONENTS=opaque to keep bytes).
    if std::env::var("ITEM_COMPONENTS").as_deref() != Ok("opaque") {
        let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
        let ctx = pz_entity::BasicContext {
            attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
            fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
            ..Default::default()
        };
        let ctx: &'static pz_entity::BasicContext = Box::leak(Box::new(ctx));
        env.components = Box::new(pz_inventory::component::EntityCodec { ctx });
    }
    // ClothingItem assets (synchWithVisual at load); ITEM_ASSETS=0 skips them.
    if std::env::var("ITEM_ASSETS").as_deref() != Ok("0") {
        env.assets = Some(pz_inventory::ItemAssets::load(game).unwrap());
    }
    let world_age: f64 = a
        .get(6)
        .map(|s| s.parse().unwrap())
        .expect("world age hours");
    env.world_age_hours = world_age;
    let settings = SaveSettings {
        world_age_hours: world_age,
        ..Default::default()
    };
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: &env,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut java: BTreeMap<(i32, i32), Vec<String>> = BTreeMap::new();
    let mut cur = (0, 0);
    for l in oracle.lines() {
        if let Some(r) = l.strip_prefix("chunk ") {
            let (x, y) = r.split_once(',').unwrap();
            cur = (x.parse().unwrap(), y.parse().unwrap());
            java.entry(cur).or_default();
        } else {
            java.get_mut(&cur).unwrap().push(l.to_string());
        }
    }
    let (mut equal, mut differ, mut raw, mut total) = (0usize, 0usize, 0usize, 0usize);
    let mut fields = BTreeMap::<String, usize>::new();
    let mut shown = 0;
    for (&(wx, wy), jl) in &java {
        let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
        let ch = match load_chunk(&bytes, wx, wy, &ctx) {
            Ok(c) => c.chunk,
            Err(e) => {
                println!("LOADERR {wx},{wy} {e}");
                continue;
            }
        };
        let mut lines = Vec::new();
        for sq in ch.squares() {
            for o in &sq.objects {
                let conts: Vec<&ItemContainer> = match &o.kind {
                    ObjectKind::Mannequin(Some(m)) => m.container.iter().collect(),
                    _ => o.containers().collect(),
                };
                for c in conts {
                    dump(&c.items, &mut lines, &mut raw);
                }
                if let ObjectKind::WorldInventory(w) = &o.kind {
                    dump(std::slice::from_ref(&w.item), &mut lines, &mut raw);
                }
            }
            for bd in &sq.static_moving {
                if let Some(c) = &bd.container {
                    dump(&c.items, &mut lines, &mut raw);
                }
            }
        }
        lines.sort();
        let mut jl = jl.clone();
        jl.sort();
        total += jl.len();
        // Pair by item id (sorted lines start with class and type; match by
        // the id field instead).
        let id_of = |l: &str| {
            l.split(' ')
                .find_map(|w| w.strip_prefix("id="))
                .unwrap_or("")
                .to_string()
        };
        let mut rm: BTreeMap<String, Vec<&String>> = BTreeMap::new();
        for l in &lines {
            rm.entry(format!(
                "{} {}",
                id_of(l),
                l.split(' ').nth(1).unwrap_or("")
            ))
            .or_default()
            .push(l);
        }
        for j in &jl {
            let key = format!("{} {}", id_of(j), j.split(' ').nth(1).unwrap_or(""));
            let r = rm.get_mut(&key).and_then(|v| v.pop());
            match r {
                Some(r) if r == j => equal += 1,
                Some(r) => {
                    differ += 1;
                    let rw: Vec<&str> = r.split(' ').collect();
                    let jw: Vec<&str> = j.split(' ').collect();
                    for (x, y) in rw.iter().zip(jw.iter()) {
                        if x != y {
                            let name = y.split('=').next().unwrap_or("");
                            *fields.entry(format!("{} {}", jw[0], name)).or_default() += 1;
                        }
                    }
                    if shown < 20 {
                        shown += 1;
                        println!("DIFF {wx},{wy}\n  rust: {r}\n  java: {j}");
                    }
                }
                None => {
                    differ += 1;
                    if shown < 40 {
                        shown += 1;
                        println!("MISSING {wx},{wy} {}", &j[..j.len().min(160)]);
                    }
                    *fields
                        .entry("missing in rust (raw or not loaded)".into())
                        .or_default() += 1;
                }
            }
        }
    }
    for (k, v) in &fields {
        println!("FIELD {v} {k}");
    }
    println!("items={total} equal={equal} differ={differ} raw={raw}");
    let _ = env.world_age_hours();
}

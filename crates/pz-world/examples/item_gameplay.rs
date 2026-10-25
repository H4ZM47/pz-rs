//! Evaluates the `pz-inventory` gameplay methods on every item of a save's
//! chunk files and on one item created from every item script, and writes
//! them in the format of a local harness that evaluates the original's
//! methods on the same items (`loaded.txt`, `created.txt`).
//!
//! ```text
//! item_gameplay <game dir> <sprite tables> <save tables> <save dir> <out dir> <oracle ctx line file>
//! ```
//!
//! The context line (`#ctx worldAge=.. multiplier=.. eq=.. basic=..
//! improvised=.. standard=.. raining=.. rain=..`) carries the game state the
//! harness read, so both sides evaluate under the same values.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use pz_inventory::component::FactoryRng;
use pz_inventory::context::{ItemContext, NoOwner};
use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::{parse_item_names, ScriptItems};
use pz_inventory::item::{InventoryItem, ItemKind};
use pz_inventory::lua_table::{LuaKey, LuaValue};
use pz_inventory::visual::NoVisualRng;
use pz_inventory::{ItemAssets, ItemContainer, ItemTemplate, SavedItem};
use pz_world::objects::ObjectKind;
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables, WORLD_VERSION};
use pz_world::{IsoSpriteManager, SpriteTables};

/// `java.util.Random`.
struct JavaRandom(u64);

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn bits(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i64 as i32
    }
}

impl FactoryRng for JavaRandom {
    fn next_int(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let mut r = self.bits(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.bits(31);
        }
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        min + (self.bits(24) as f32 / (1 << 24) as f32) * (max - min)
    }
}

/// `String.hashCode()`.
fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

struct Ctx {
    world_age: f64,
    multiplier: f32,
    eq: f64,
    basic: f32,
    improvised: f32,
    standard: f32,
    raining: bool,
    rain: f32,
    scripts: HashMap<String, Rc<ItemTemplate>>,
    assets: ItemAssets,
    dictionary: WorldDictionary,
    entity: &'static pz_entity::BasicContext,
    env: &'static ScriptItems,
}

impl ItemContext for Ctx {
    fn world_age_hours(&self) -> f64 {
        self.world_age
    }
    fn multiplier(&self) -> f32 {
        self.multiplier
    }
    fn rain_intensity(&self) -> f32 {
        self.rain
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        self.eq
    }
    fn basic_mask_defense(&self) -> f32 {
        self.basic
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        self.improvised
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        self.standard
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
        Some(pz_inventory::env::create_from_template(self.env, t, 0))
    }
    fn assets(&self) -> Option<&ItemAssets> {
        Some(&self.assets)
    }
    fn item_type_from_id(&self, id: i16) -> Option<String> {
        self.dictionary.item_type(id).map(str::to_string)
    }
    fn fluid_amount(&self, item: &InventoryItem) -> Option<f32> {
        pz_inventory::gameplay::fluid_container_amount(item, self.entity, WORLD_VERSION)
    }
}

fn fb(v: f32) -> String {
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

fn put(o: &mut String, k: &str, v: impl AsRef<str>) {
    write!(o, "\t{k}={}", v.as_ref().replace('\t', " ")).unwrap();
}

fn is_clothing(i: &InventoryItem) -> bool {
    matches!(
        i.kind,
        ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
    )
}

fn getters(i: &mut InventoryItem, ctx: &Ctx, o: &mut String, loaded: bool) {
    let own = NoOwner;
    put(o, "actualWeight", fb(i.get_actual_weight_with(ctx)));
    put(o, "weight", fb(i.get_weight_with(ctx)));
    put(
        o,
        "contentsWeight",
        fb(i.get_contents_weight_with(ctx, &own)),
    );
    put(
        o,
        "unequippedWeight",
        fb(i.get_unequipped_weight(ctx, &own)),
    );
    put(o, "equippedWeight", fb(i.get_equipped_weight(ctx, &own)));
    put(
        o,
        "hotbarEquippedWeight",
        fb(i.get_hotbar_equipped_weight(ctx, &own)),
    );
    put(o, "isFakeEquipped", b(i.is_fake_equipped(&own)));
    put(o, "isEquipped", b(i.is_equipped(&own)));
    put(o, "currentCondition", fb(i.get_current_condition()));
    put(
        o,
        "bloodClothingType",
        i.get_blood_clothing_type().map_or("null".to_string(), |v| {
            v.iter().map(|n| format!("{n},")).collect::<String>()
        }),
    );
    put(o, "bodyLocation", s(i.get_body_location()));
    put(o, "isEquippedNoSprint", b(i.is_equipped_no_sprint()));
    put(
        o,
        "isProtectFromRainWhileEquipped",
        b(i.is_protect_from_rain_while_equipped()),
    );
    put(o, "clothingItemName", s(i.get_clothing_item_name()));
    put(
        o,
        "alternateModelName",
        s(i.get_alternate_model_name(Some(&ctx.assets), &own)),
    );
    put(
        o,
        "clothingItem",
        s(i.get_clothing_item(Some(&ctx.assets), &own)
            .map(|c| c.name.as_str())),
    );
    let vis = match i.get_visual(Some(&ctx.assets), &own, &NoVisualRng) {
        None => "null".to_string(),
        Some(v) => {
            let alt = if v.alternate_model_name.is_empty() {
                "null"
            } else {
                v.alternate_model_name.as_str()
            };
            let mut x = format!("{}|{}", v.clothing_item_name, alt);
            if loaded {
                let tint = v.tint.map_or("null".into(), |t| {
                    format!("{},{},{}", fb(t[0]), fb(t[1]), fb(t[2]))
                });
                write!(
                    x,
                    "|{tint}|{}|{}|{}|{}",
                    v.base_texture,
                    v.texture_choice,
                    fb(v.hue),
                    s(v.decal.as_deref())
                )
                .unwrap();
            }
            x
        }
    };
    put(o, "visual", vis);
    put(
        o,
        "weaponType",
        match &i.kind {
            ItemKind::Weapon(_) => s(i.get_weapon_type()),
            _ => "-".into(),
        },
    );
    put(o, "maxUses", i.get_max_uses().to_string());
    put(o, "currentUses", i.get_current_uses().to_string());
    if is_clothing(i) {
        put(o, "scratchDefense", fb(i.get_scratch_defense()));
        put(o, "biteDefense", fb(i.get_bite_defense()));
        put(o, "bulletDefense", fb(i.get_bullet_defense()));
        put(
            o,
            "corpseSicknessDefense",
            fb(i.get_corpse_sickness_defense_with(ctx)),
        );
        put(o, "waterResistance", fb(i.get_water_resistance()));
        put(o, "hasFilter", b(i.has_filter()));
        put(o, "hasTank", b(i.has_tank()));
        put(o, "filterType", s(i.get_filter_type()));
        put(o, "tankType", s(i.get_tank_type()));
        let mut d = String::new();
        for p in 0..ctx.assets.blood_clothing_types.max() {
            for (bite, bullet) in [(false, false), (true, false), (false, true)] {
                match i.get_def_for_part(p, bite, bullet, &ctx.assets) {
                    Ok(v) => d.push_str(&fb(v)),
                    Err(_) => d.push_str("E:NullPointerException"),
                }
                d.push(',');
            }
        }
        put(o, "defForPart", d);
    }
}

fn wetness_test(i: &mut InventoryItem, ctx: &Ctx, o: &mut String) {
    if !is_clothing(i) {
        return;
    }
    let age = ctx.world_age as f32;
    let set_last = |i: &mut InventoryItem, v: f32| {
        if let ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) = &mut i.kind {
            c.last_wetness_update = v;
        }
    };
    i.set_wetness(50.0);
    set_last(i, age - 1.0);
    i.update_wetness_with(true, &NoOwner, ctx);
    let w1 = i.get_wetness();
    i.set_wetness(30.0);
    set_last(i, age - 0.5);
    i.flush_wetness(&NoOwner, ctx);
    let last = match &i.kind {
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => c.last_wetness_update,
        _ => 0.0,
    };
    put(
        o,
        "wetUpdate",
        format!("{},{},{}", fb(w1), fb(i.get_wetness()), fb(last)),
    );
}

fn mutations(ctx: &Ctx, full: &str, o: &mut String) {
    let own = NoOwner;
    // setCondition.
    {
        let mut i = ctx.create_item(full).unwrap();
        let max = i.condition_max;
        i.set_condition(max / 2, &own, ctx);
        let (c1, cc) = (i.condition, i.get_current_condition());
        i.set_condition(0, &own, ctx);
        let mut v = format!("{c1},{},{},{}", fb(cc), i.condition, b(i.is_broken()));
        if is_clothing(&i) {
            write!(
                v,
                ",{},{},{}",
                fb(i.get_scratch_defense()),
                fb(i.get_bite_defense()),
                fb(i.get_corpse_sickness_defense_with(ctx))
            )
            .unwrap();
        }
        put(o, "setCondition", v);
    }
    // Use() three times in a container without a parent.
    {
        let mut i = ctx.create_item(full).unwrap();
        i.id = -7;
        let mut c = ItemContainer::new(Some("none"));
        let mut inside = true;
        let mut v = String::new();
        for _ in 0..3 {
            let mut out = i.use_in(&own, ctx);
            if !inside {
                out.added = None;
                out.remove = false;
            }
            c.apply_use(i.id, &mut out, 1.0);
            if out.remove {
                inside = false;
            }
            let mut types: Vec<String> = c
                .items
                .iter()
                .filter_map(|x| x.as_item())
                .map(|x| x.get_full_type())
                .collect();
            if inside {
                types.push(i.get_full_type());
            }
            types.sort();
            write!(
                v,
                "{};{};{};{};{}/",
                i.get_current_uses(),
                b(inside),
                types.join("+"),
                fb(i.get_actual_weight_with(ctx)),
                b(i.custom_weight)
            )
            .unwrap();
        }
        put(o, "use", v);
    }
    // Wetness.
    {
        let mut i = ctx.create_item(full).unwrap();
        wetness_test(&mut i, ctx, o);
    }
    // Gas mask / SCBA.
    {
        let mut i = ctx.create_item(full).unwrap();
        if is_clothing(&i) {
            let set = |i: &mut InventoryItem, k: &str, v: LuaValue| {
                i.table
                    .get_or_insert_with(Default::default)
                    .rawset(LuaKey::Str(k.into()), Some(v));
            };
            set(
                &mut i,
                "filterType",
                LuaValue::Str("Base.GasmaskFilter".into()),
            );
            set(&mut i, "usedDelta", LuaValue::Num(0.5));
            let alt = i.get_alternate_model_name(Some(&ctx.assets), &own);
            let mut v = format!(
                "{},{},{}",
                b(i.has_filter()),
                fb(i.get_corpse_sickness_defense_with(ctx)),
                s(alt)
            );
            let _ = i.drain_gas_mask(1.0, ctx);
            write!(v, ",{}", fb(i.get_used_delta())).unwrap();
            let _ = i.drain_gas_mask(100000.0, ctx);
            write!(
                v,
                ",{},{}",
                fb(i.get_used_delta()),
                fb(i.get_corpse_sickness_defense_with(ctx))
            )
            .unwrap();
            put(o, "gasMask", v);
            set(&mut i, "tankType", LuaValue::Str("Base.Oxygen_Tank".into()));
            set(&mut i, "usedDelta", LuaValue::Num(0.5));
            i.activated = true;
            let _ = i.drain_scba(ctx);
            let mut v = format!(
                "{},{},{}",
                b(i.has_tank()),
                fb(i.get_used_delta()),
                b(i.activated)
            );
            set(&mut i, "usedDelta", LuaValue::Num(0.0));
            let _ = i.drain_scba(ctx);
            write!(v, ",{}", b(i.activated)).unwrap();
            put(o, "scba", v);
        }
    }
    // name == fullType.
    {
        let mut i = ctx.create_item(full).unwrap();
        i.name = full.to_string();
        put(o, "nameRule", fb(i.get_actual_weight_with(ctx)));
    }
}

/// The decoded items in the harness's order (a bag, then its items).
fn collect(items: &[SavedItem], out: &mut Vec<InventoryItem>) {
    for it in items {
        if let SavedItem::Item(i) = it {
            out.push((**i).clone());
            if let ItemKind::Container(c) = &i.kind {
                collect(&c.container.items, out);
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
    let out_dir = Path::new(&a[5]);
    let ctx_line = std::fs::read_to_string(&a[6]).unwrap();
    let kv: HashMap<&str, &str> = ctx_line
        .trim()
        .split('\t')
        .filter_map(|p| p.split_once('='))
        .collect();
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
    let dict_bytes = std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap();
    let mut env = ScriptItems::new(&sm, WorldDictionary::load(&dict_bytes).unwrap(), &names);
    env.blood_body_parts = save_tables.blood_body_parts;
    env.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
    let entity = pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
        ..Default::default()
    };
    let entity: &'static pz_entity::BasicContext = Box::leak(Box::new(entity));
    env.components = Box::new(pz_inventory::component::EntityCodec { ctx: entity });
    let assets = ItemAssets::load(game).unwrap();
    env.assets = Some(assets.clone());
    let world_age: f64 = kv["worldAge"].parse().unwrap();
    env.world_age_hours = world_age;
    // Every script item, as `ScriptManager.FindItem` gives it.
    let mut scripts = HashMap::new();
    for rc in pz_scripts::objects::items::get_all_items(&sm) {
        let Some(t) = ItemTemplate::from_script_ref(&sm, &rc) else {
            continue;
        };
        let it = rc.borrow();
        let t = match names.get(&it.full_name()) {
            Some(n) if *n != t.display_name => {
                let mut t = (*t).clone();
                t.display_name = n.clone();
                Rc::new(t)
            }
            _ => t,
        };
        scripts.insert(it.full_name(), t);
    }
    let env: &'static ScriptItems = Box::leak(Box::new(env));
    let ctx = Ctx {
        env,
        world_age,
        multiplier: kv["multiplier"].parse().unwrap(),
        eq: kv["eq"].parse().unwrap(),
        basic: kv["basic"].parse().unwrap(),
        improvised: kv["improvised"].parse().unwrap(),
        standard: kv["standard"].parse().unwrap(),
        raining: kv["raining"] == "true",
        rain: kv["rain"].parse().unwrap(),
        scripts,
        assets,
        dictionary: WorldDictionary::load(&dict_bytes).unwrap(),
        entity,
    };
    std::fs::create_dir_all(out_dir).unwrap();

    // Created items.
    let mut types: Vec<&String> = ctx.scripts.keys().collect();
    types.sort();
    let mut created = String::new();
    created.push_str(ctx_line.trim());
    created.push('\n');
    for full in types {
        if full.starts_with("Moveables.") {
            continue;
        }
        *ctx.env.rand.borrow_mut() = Some(Box::new(JavaRandom::new(java_hash(full) as i64)));
        let mut i = ctx.create_item(full).unwrap();
        let mut o = format!("{full}\tclass={}", i.class_name());
        getters(&mut i, &ctx, &mut o, false);
        mutations(&ctx, full, &mut o);
        created.push_str(&o);
        created.push('\n');
    }
    std::fs::write(out_dir.join("created.txt"), created).unwrap();

    // Loaded items.
    let settings = SaveSettings {
        world_age_hours: world_age,
        ..Default::default()
    };
    let save_ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: env,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut loaded = String::new();
    loaded.push_str(ctx_line.trim());
    loaded.push('\n');
    let mut chunks = Vec::new();
    for xd in std::fs::read_dir(save_dir.join("map")).unwrap().flatten() {
        let Ok(wx) = xd.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        for f in std::fs::read_dir(xd.path()).unwrap().flatten() {
            let n = f.file_name().to_string_lossy().to_string();
            if let Some(Ok(wy)) = n.strip_suffix(".bin").map(|s| s.parse::<i32>()) {
                chunks.push((wx, wy));
            }
        }
    }
    chunks.sort();
    let mut n = 0;
    for (wx, wy) in chunks {
        let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
        *env.rand.borrow_mut() = Some(Box::new(JavaRandom::new(
            ((wx as i64) << 32) ^ (wy as i64 & 0xffff_ffff),
        )));
        let ch = match load_chunk(&bytes, wx, wy, &save_ctx) {
            Ok(c) => c.chunk,
            Err(e) => {
                writeln!(loaded, "ERROR {wx},{wy} {e}").unwrap();
                continue;
            }
        };
        let mut items: Vec<InventoryItem> = Vec::new();
        for sq in ch.squares() {
            for o in &sq.objects {
                match &o.kind {
                    ObjectKind::Mannequin(Some(m)) => {
                        for c in m.container.iter() {
                            collect(&c.items, &mut items);
                        }
                    }
                    _ => {
                        for c in o.containers() {
                            collect(&c.items, &mut items);
                        }
                    }
                }
                if let ObjectKind::WorldInventory(w) = &o.kind {
                    collect(std::slice::from_ref(&w.item), &mut items);
                }
            }
            for bd in &sq.static_moving {
                if let Some(c) = &bd.container {
                    collect(&c.items, &mut items);
                }
            }
        }
        for mut i in items {
            let i = &mut i;
            let mut o = format!("{wx},{wy}\t{}\t{}", i.id, i.get_full_type());
            getters(i, &ctx, &mut o, true);
            wetness_test(i, &ctx, &mut o);
            if is_clothing(i) {
                put(&mut o, "usedDelta", fb(i.get_used_delta()));
            }
            loaded.push_str(&o);
            loaded.push('\n');
            n += 1;
        }
    }
    std::fs::write(out_dir.join("loaded.txt"), loaded).unwrap();
    println!("loaded items={n}");
}

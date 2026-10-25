//! Runs `InventoryItem.DoTooltipEmbedded(tooltip, layout, 0)` on one item
//! created from every item script (`Rand` seeded with the full type's
//! `String.hashCode()`) and on every item of a save's chunk files, and
//! writes each layout row (label, colours, value, right-justify,
//! progress) in the format of a local harness that does the same with
//! the original classes (`created-tooltips.txt`, `loaded-tooltips.txt`).
//! No game data or harness output is part of this repository.
//!
//! ```text
//! item_tooltip_parity <game dir> <sprite tables> <save tables> <save dir> <out dir> <oracle ctx line file>
//! ```

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::{parse_item_names, ScriptItems};
use pz_inventory::item::{InventoryItem, ItemKind};
use pz_inventory::SavedItem;
use pz_lua_api::bind_items::{create_item, install_item_env, GameItemEnv, ItemLuaEnv, ItemRef};
use pz_lua_api::item_tooltip::{do_tooltip_embedded, item_name};
use pz_ui::tooltip::{java_float_to_string as f, Layout, ObjectTooltip};
use pz_world::objects::ObjectKind;
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{IsoSpriteManager, SpriteTables};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

/// The game environment with the harness's `ZomboidGlobals` and world
/// dictionary.
struct Env {
    inner: GameItemEnv,
    eq: f64,
    dict: Option<Rc<WorldDictionary>>,
    sprites: Rc<HashMap<String, pz_inventory::env::WorldSpriteInfo>>,
}

impl pz_inventory::ItemContext for Env {
    fn world_age_hours(&self) -> f64 {
        self.inner.world_age_hours()
    }
    fn multiplier(&self) -> f32 {
        self.inner.multiplier()
    }
    fn rain_intensity(&self) -> f32 {
        self.inner.rain_intensity()
    }
    fn is_raining(&self) -> bool {
        self.inner.is_raining()
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        self.eq
    }
    fn basic_mask_defense(&self) -> f32 {
        self.inner.basic_mask_defense()
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        self.inner.improvised_gas_mask_defense()
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        self.inner.standard_gas_mask_defense()
    }
    fn script(&self, name: &str) -> Option<Rc<pz_inventory::ItemTemplate>> {
        self.inner.script(name)
    }
    fn create_item(&self, full_type: &str) -> Option<InventoryItem> {
        self.inner.create_item(full_type)
    }
    fn item_type_from_id(&self, id: i16) -> Option<String> {
        self.dict.as_ref()?.item_type(id).map(str::to_string)
    }
    fn fluid_amount(&self, item: &InventoryItem) -> Option<f32> {
        self.inner.fluid_amount(item)
    }
    fn assets(&self) -> Option<&pz_inventory::ItemAssets> {
        self.inner.assets()
    }
}

impl ItemLuaEnv for Env {
    fn context(&self) -> &dyn pz_inventory::ItemContext {
        self
    }
    fn owner(&self, item: &ItemRef) -> Rc<dyn pz_inventory::ItemOwner> {
        self.inner.owner(item)
    }
    fn rand_next(&self, bound: i32) -> i32 {
        self.inner.rand_next(bound)
    }
    fn rand_float(&self, min: f32, max: f32) -> f32 {
        self.inner.rand_float(min, max)
    }
    fn entity(&self) -> Option<&dyn pz_entity::EntityContext> {
        self.inner.entity()
    }
    fn world_sprite(&self, name: &str) -> Option<pz_inventory::env::WorldSpriteInfo> {
        self.sprites.get(name).cloned()
    }
}

fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// The harness's character: no perk levels (the harness has no
/// `PerkFactory` perks), the Nutritionist trait, nothing worn, read or
/// known.
const FAKE_CHARACTER: &str = r#"
Perks = {Cooking = "Cooking", Aiming = "Aiming", Husbandry = "Husbandry"}
CharacterTrait = {NUTRITIONIST = "NUTRITIONIST", NUTRITIONIST2 = "NUTRITIONIST2",
  ILLITERATE = "ILLITERATE", ORGANIZED = "ORGANIZED", DISORGANIZED = "DISORGANIZED"}
local group = {isExclusive = function() return false end}
local worn = {size = function() return 0 end, get = function() return nil end,
  getBodyLocationGroup = function() return group end}
local known = {contains = function() return false end}
return {
  getPerkLevel = function(self, p) return 0 end,
  hasTrait = function(self, t) return t == "NUTRITIONIST" end,
  getWornItems = function() return worn end,
  getKnownRecipes = function() return known end,
  getAlreadyReadPages = function() return 0 end,
  isLiteratureRead = function() return false end,
}
"#;

fn rows(
    t: &mut pz_kahlua::Thread,
    el: &pz_ui::UIElement,
    chr: &pz_kahlua::Value,
    item: &ItemRef,
    o: &mut String,
) {
    let tt = ObjectTooltip::of(el).unwrap();
    tt.set_character(None);
    let mut layout = Layout::default();
    let r = do_tooltip_embedded(t, item, el, Some(&mut layout), 0);
    write!(o, "\tname={}", item_name(&mut item.borrow_mut())).unwrap();
    layout_rows(&layout, o);
    exc(r, o);
    o.push_str("\t||");
    tt.set_character(Some(chr.clone()));
    let mut layout = Layout::default();
    let r = do_tooltip_embedded(t, item, el, Some(&mut layout), 0);
    layout_rows(&layout, o);
    exc(r, o);
    *o = o.replace('\n', " ");
}

fn exc(r: pz_kahlua::LuaResult<()>, o: &mut String) {
    match r {
        Ok(()) => {}
        Err(pz_kahlua::LuaError::Runtime { class, .. }) => write!(o, "\tEXC={class}").unwrap(),
        Err(e) => write!(o, "\tEXC=rust:{e:?}").unwrap(),
    }
}

fn layout_rows(layout: &Layout, o: &mut String) {
    for li in &layout.items {
        write!(
            o,
            "\t[{}|{},{},{},{}|",
            li.label.as_deref().unwrap_or("null"),
            f(li.r0),
            f(li.g0),
            f(li.b0),
            f(li.a0)
        )
        .unwrap();
        if li.has_value {
            write!(
                o,
                "{}|{}|{},{},{},{}|{}",
                li.value.as_deref().unwrap_or("null"),
                li.right_justify as i32,
                f(li.r1),
                f(li.g1),
                f(li.b1),
                f(li.a1),
                f(li.progress_fraction)
            )
            .unwrap();
        } else {
            o.push('-');
        }
        o.push(']');
    }
}

fn collect(items: &[SavedItem], out: &mut Vec<InventoryItem>) {
    for it in items {
        if let SavedItem::Item(i) = it {
            collect_item(i, out);
        }
    }
}

fn collect_item(i: &InventoryItem, out: &mut Vec<InventoryItem>) {
    out.push(i.clone());
    if let ItemKind::Container(c) = &i.kind {
        collect(&c.container.borrow().to_container().items, out);
    }
}

fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables =
        SpriteTables::from_text(&std::fs::read_to_string(&a[2]).map_err(|e| e.to_string())?)?;
    let save_tables =
        SaveTables::from_text(&std::fs::read_to_string(&a[3]).map_err(|e| e.to_string())?)?;
    let save_dir = Path::new(&a[4]);
    let out_dir = Path::new(&a[5]);
    let ctx_line = std::fs::read_to_string(&a[6]).map_err(|e| e.to_string())?;
    let kv: HashMap<&str, &str> = ctx_line
        .trim()
        .split('\t')
        .filter_map(|p| p.split_once('='))
        .collect();
    let mut fs = pz_fs::GameFs::new(game);
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .map_err(|e| e.to_string())?;
    sm.load(&fs, &[]).map_err(|e| e.to_string())?;
    let sm = Rc::new(sm);
    pz_lua_api::bind_registry::install_script_manager(sm.clone());
    {
        let core = pz_lua_api::bind_core::shared_core();
        pz_lua_api::bind_text::boot(&fs, &core.borrow())?;
    }
    pz_lua_api::bind_appearance::shared_population()
        .borrow_mut()
        .init(&mut fs);
    let eq: f64 = kv.get("eq").and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let tod: f32 = kv.get("tod").and_then(|s| s.parse().ok()).unwrap_or(9.0);
    pz_lua_api::bind_climate::shared_game_time()
        .borrow_mut()
        .time_of_day = tod;
    let assets = pz_inventory::ItemAssets::load(game)?;
    let jar = game.join("projectzomboid.jar");
    let entity = Rc::new(pz_lua_api::bind_items::item_entity_context(&jar, &sm)?);

    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[])?;
    sprites.loaded_tile_definitions(&|_| false)?;
    let world_sprites = Rc::new(pz_world::save::items::moveable_sprites(&sprites));
    pz_lua_api::item_tooltip::set_sandbox_fallback(pz_lua_api::item_tooltip::SandboxFallback {
        discomfort_factor: kv
            .get("discomfort")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0),
        tainted_water_text: kv.get("tainted").is_none_or(|s| *s == "true"),
    });
    let mut t = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).map_err(|e| format!("{e:?}"))?;
    let chr = {
        let f = pz_kahlua::stdlib::load_string(FAKE_CHARACTER, Some("chr"), t.globals().clone())
            .map_err(|e| format!("{e:?}"))?;
        t.pcall(&f, &[])
            .map_err(|(m, _)| format!("{m:?}"))?
            .remove(0)
    };
    let el = ObjectTooltip::create("Small");
    ObjectTooltip::of(&el).unwrap().set_measure_only(true);
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;

    // Created items.
    let mut types: Vec<String> = pz_scripts::objects::items::get_all_items(&sm)
        .iter()
        .map(|i| i.borrow().full_name())
        .collect();
    types.sort();
    let mut created = format!("{}\n", ctx_line.trim());
    for ft in &types {
        if ft.starts_with("Moveables.") {
            continue;
        }
        let mut env = GameItemEnv::new(Some(assets.clone()));
        env.rand = Some(pz_core::random::Rand::new(java_hash(ft) as i64));
        env.entity = Some(entity.clone());
        install_item_env(Rc::new(Env {
            inner: env,
            eq,
            dict: None,
            sprites: world_sprites.clone(),
        }));
        let Some(item) = create_item(ft, 1.0) else {
            writeln!(created, "{ft}\tcreate=null").unwrap();
            continue;
        };
        let class = pz_lua_api::bind_items::class_of(&item.borrow());
        let mut o = format!("{ft}\tclass={}", class.rsplit('.').next().unwrap());
        rows(&mut t, &el, &chr, &item, &mut o);
        created.push_str(&o);
        created.push('\n');
    }
    std::fs::write(out_dir.join("created-tooltips.txt"), created).map_err(|e| e.to_string())?;

    // Alarm clocks: the created state, then set/update sequences.
    let nights: f32 = kv.get("nights").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let mut alarms = String::new();
    for ft in &types {
        if ft.starts_with("Moveables.") {
            continue;
        }
        for seed in 0..8 {
            let mut env = GameItemEnv::new(Some(assets.clone()));
            env.rand = Some(pz_core::random::Rand::new(
                java_hash(ft).wrapping_add(seed) as i64
            ));
            env.entity = Some(entity.clone());
            install_item_env(Rc::new(Env {
                inner: env,
                eq,
                dict: None,
                sprites: world_sprites.clone(),
            }));
            let Some(item) = create_item(ft, 1.0) else {
                continue;
            };
            let mut it = item.borrow_mut();
            if pz_inventory::alarm::alarm(&it).is_none() {
                break;
            }
            let digital = pz_lua_api::hud::alarm_clocks::alarm_clock_is_digital(&it);
            let mut o = format!("{ft}#{seed}");
            let dump = |it: &InventoryItem, o: &mut String| {
                let a = pz_inventory::alarm::alarm(it).unwrap();
                write!(
                    o,
                    "\t{},{},{},{},{},{},{}",
                    a.alarm_hour,
                    a.alarm_minutes,
                    a.alarm_set as i32,
                    pz_core::config::double_to_string(a.ring_since),
                    a.force_dont_ring,
                    a.is_ringing() as i32,
                    digital as i32
                )
                .unwrap();
            };
            // GameTime.getWorldAgeHours() and getMinutes() at `tod`.
            let now = |tod: f32| {
                let mut h = nights * 24.0;
                h += if tod >= 7.0 { tod - 7.0 } else { tod + 17.0 };
                pz_inventory::alarm::AlarmTime::from_time_of_day(tod, h as f64)
            };
            dump(&it, &mut o);
            let a = pz_inventory::alarm::alarm_mut(&mut it).unwrap();
            a.set_hour(7);
            a.set_minute(30);
            a.set_alarm_set(true, now(7.2).minutes);
            dump(&it, &mut o);
            for tod in [7.49f32, 7.5, 7.51, 7.67, 7.5] {
                pz_inventory::alarm::alarm_mut(&mut it)
                    .unwrap()
                    .update(&now(tod), false);
                dump(&it, &mut o);
            }
            let a = pz_inventory::alarm::alarm_mut(&mut it).unwrap();
            a.stop_ringing(now(7.5).minutes);
            dump(&it, &mut o);
            pz_inventory::alarm::alarm_mut(&mut it)
                .unwrap()
                .set_force_dont_ring(20);
            dump(&it, &mut o);
            pz_inventory::alarm::alarm_mut(&mut it)
                .unwrap()
                .set_alarm_set(false, now(7.5).minutes);
            dump(&it, &mut o);
            pz_inventory::alarm::alarm_mut(&mut it).unwrap().set_hour(9);
            dump(&it, &mut o);
            alarms.push_str(&o);
            alarms.push('\n');
        }
    }
    std::fs::write(out_dir.join("alarms.txt"), alarms).map_err(|e| e.to_string())?;

    // Loaded items.
    let mut env = GameItemEnv::new(Some(assets.clone()));
    env.entity = Some(entity.clone());
    let dict_bytes =
        std::fs::read(save_dir.join("WorldDictionary.bin")).map_err(|e| e.to_string())?;
    let dict = Rc::new(WorldDictionary::load(&dict_bytes).map_err(|e| e.to_string())?);
    install_item_env(Rc::new(Env {
        inner: env,
        eq,
        dict: Some(dict),
        sprites: world_sprites.clone(),
    }));
    let names = parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json"))
            .map_err(|e| e.to_string())?,
    );
    let mut items_env = ScriptItems::new(
        &sm,
        WorldDictionary::load(&dict_bytes).map_err(|e| e.to_string())?,
        &names,
    );
    items_env.blood_body_parts = save_tables.blood_body_parts;
    items_env.world_sprites = (*world_sprites).clone();
    let ent: &'static pz_entity::BasicContext = Box::leak(Box::new(
        pz_lua_api::bind_items::item_entity_context(&jar, &sm)?,
    ));
    items_env.components = Box::new(pz_inventory::component::EntityCodec { ctx: ent });
    items_env.assets = Some(assets.clone());
    let world_age: f64 = kv
        .get("worldAge")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    items_env.world_age_hours = world_age;
    let settings = SaveSettings {
        world_age_hours: world_age,
        ..Default::default()
    };
    let save_ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: &items_env,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut chunks = Vec::new();
    for xd in std::fs::read_dir(save_dir.join("map"))
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let Ok(wx) = xd.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        for fi in std::fs::read_dir(xd.path())
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let n = fi.file_name().to_string_lossy().to_string();
            if let Some(Ok(wy)) = n.strip_suffix(".bin").map(|s| s.parse::<i32>()) {
                chunks.push((wx, wy));
            }
        }
    }
    chunks.sort();
    let mut loaded = format!("{}\n", ctx_line.trim());
    let mut n = 0;
    for (wx, wy) in chunks {
        let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).map_err(|e| e.to_string())?;
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
                    if let Some(item) = w.item.borrow_item() {
                        collect_item(&item, &mut items);
                    }
                }
            }
            for bd in &sq.static_moving {
                if let Some(c) = &bd.container {
                    collect(&c.items, &mut items);
                }
            }
        }
        for i in items {
            let mut o = format!("{wx},{wy}\t{}\t{}", i.id, i.get_full_type());
            let item = Rc::new(RefCell::new(i));
            rows(&mut t, &el, &chr, &item, &mut o);
            loaded.push_str(&o);
            loaded.push('\n');
            n += 1;
        }
    }
    std::fs::write(out_dir.join("loaded-tooltips.txt"), loaded).map_err(|e| e.to_string())?;
    println!("created={} loaded={n}", types.len());
    Ok(())
}

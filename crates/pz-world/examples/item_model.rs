//! Writes the per-instance model and fluid state of `pz-inventory` items
//! (`isFertilized`/`isRotten`, the model index and lists,
//! `modelWeaponPart`, the model getters, the `FluidContainer`'s amount,
//! capacity and colour, and the saved component bytes) for every item of a
//! save's chunk files and for items created from every item script, in the
//! format of a local harness that reads the same values from the original
//! (`loaded.txt`, `created.txt`).
//!
//! ```text
//! item_model <game dir> <sprite tables> <save tables> <save dir> <out dir>
//! ```
//!
//! Created items draw from a `java.util.Random` seeded with the full type's
//! `String.hashCode()`, and chunk loads from one seeded with
//! `(wx << 32) ^ wy`, as the harness seeds the game's `Rand`.

use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use pz_inventory::component::{FactoryRng, COMPONENT_WORLD_VERSION};
use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::{parse_item_names, ScriptItems};
use pz_inventory::item::{InventoryItem, ItemKind};
use pz_inventory::{ItemAssets, ItemTemplate, SavedItem};
use pz_scripts::objects::models::ModelScript;
use pz_world::objects::ObjectKind;
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
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

fn put(o: &mut String, k: &str, v: impl AsRef<str>) {
    write!(o, "\t{k}={}", v.as_ref().replace('\t', " ")).unwrap();
}

fn list(l: Option<&Rc<Vec<String>>>) -> String {
    l.map_or("null".into(), |l| {
        l.iter().map(|s| format!("{s};")).collect()
    })
}

fn opt(s: Option<&str>) -> String {
    s.unwrap_or("null").to_string()
}

fn ent_hex(i: &InventoryItem) -> String {
    let mut s = format!("{:02x}", i.components.len() as u8);
    for c in &i.components {
        for x in (c.data.len() as i32 + 2)
            .to_be_bytes()
            .iter()
            .chain(c.type_id.to_be_bytes().iter())
            .chain(c.data.iter())
        {
            write!(s, "{x:02x}").unwrap();
        }
    }
    s
}

fn getters(
    i: &InventoryItem,
    entity: &dyn pz_entity::EntityContext,
    sm: &pz_scripts::ScriptManager,
    o: &mut String,
) {
    let food = matches!(i.kind, ItemKind::Food(_));
    let weapon = matches!(i.kind, ItemKind::Weapon(_));
    let model_script = |n: &str| {
        sm.get::<ModelScript>(pz_scripts::ScriptType::Model, n)
            .map(|m| m.borrow().name.clone().unwrap_or_default())
    };
    put(o, "fert", if food { b(i.is_fertilized()) } else { "-" });
    put(o, "rotten", if food { b(i.is_rotten()) } else { "-" });
    put(o, "modelIndex", i.model_index.to_string());
    put(o, "smbi", list(i.get_static_models_by_index()));
    put(o, "wsmbi", list(i.get_world_static_models_by_index()));
    put(
        o,
        "wsbi",
        if weapon {
            list(i.get_weapon_sprites_by_index())
        } else {
            "-".into()
        },
    );
    let mwp: String = if weapon {
        i.get_model_weapon_part().map_or("null".into(), |l| {
            l.iter()
                .map(|p| {
                    format!(
                        "{}|{}|{}|{};",
                        p.part_type,
                        p.model_name,
                        opt(p.attachment_name_self.as_deref()),
                        opt(p.attachment_parent.as_deref())
                    )
                })
                .collect()
        })
    } else {
        "-".into()
    };
    put(o, "mwp", mwp);
    put(
        o,
        "weaponSprite",
        if weapon {
            opt(i.get_weapon_sprite().as_deref())
        } else {
            "-".into()
        },
    );
    put(
        o,
        "staticModel",
        opt(i.get_static_model(&model_script).as_deref()),
    );
    put(
        o,
        "worldStaticModel",
        opt(i.get_world_static_model().as_deref()),
    );
    put(
        o,
        "staticModelException",
        opt(i.get_static_model_exception(&model_script).as_deref()),
    );
    let fluid = i.get_fluid_container(entity).map_or("null".into(), |f| {
        let c = f.get_color(entity);
        format!(
            "{},{},{},{},{},{},{},{},{}",
            fb(f.get_amount()),
            fb(f.get_capacity()),
            fb(f.get_filled_ratio()),
            b(f.is_empty()),
            b(f.is_full()),
            fb(c[0]),
            fb(c[1]),
            fb(c[2]),
            fb(1.0)
        )
    });
    put(o, "fluid", fluid);
    put(o, "ent", ent_hex(i));
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
    env.assets = Some(assets);
    let env: &'static ScriptItems = Box::leak(Box::new(env));
    std::fs::create_dir_all(out_dir).unwrap();
    let _ = COMPONENT_WORLD_VERSION;

    // Created items: two per script from one generator.
    let mut scripts: Vec<(String, Rc<ItemTemplate>)> = Vec::new();
    for it in pz_scripts::objects::items::get_all_items(&sm) {
        let full = it.borrow().full_name();
        let Some(t) = ItemTemplate::from_script_ref(&sm, &it) else {
            continue;
        };
        let t = match names.get(&full) {
            Some(n) if *n != t.display_name => {
                let mut t = (*t).clone();
                t.display_name = n.clone();
                Rc::new(t)
            }
            _ => t,
        };
        scripts.push((full, t));
    }
    scripts.sort_by(|a, b| a.0.cmp(&b.0));
    let mut created = String::new();
    for (full, t) in &scripts {
        if full.starts_with("Moveables.") {
            continue;
        }
        *env.rand.borrow_mut() = Some(Box::new(JavaRandom::new(java_hash(full) as i64)));
        let i = pz_inventory::env::create_from_template(env, t.clone(), 0);
        let i2 = pz_inventory::env::create_from_template(env, t.clone(), 0);
        let mut o = format!("{full}\tclass={}", i.class_name());
        put(&mut o, "id", i.id.to_string());
        getters(&i, entity, &sm, &mut o);
        put(&mut o, "id2", i2.id.to_string());
        put(&mut o, "ent2", ent_hex(&i2));
        created.push_str(&o);
        created.push('\n');
    }
    std::fs::write(out_dir.join("created.txt"), created).unwrap();

    // Loaded items.
    let settings = SaveSettings::default();
    let save_ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: env,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut loaded = String::new();
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
        for i in items {
            let mut o = format!("{wx},{wy}\t{}\t{}", i.id, i.get_full_type());
            getters(&i, entity, &sm, &mut o);
            loaded.push_str(&o);
            loaded.push('\n');
            n += 1;
        }
    }
    std::fs::write(out_dir.join("loaded.txt"), loaded).unwrap();
    println!("loaded items={n}");
}

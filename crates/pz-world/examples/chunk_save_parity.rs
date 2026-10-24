//! Load every chunk file of a save, re-save it, and compare against the
//! output of the original game's own load and save of the same files (made
//! by a local harness on a machine with the game installed). Results are
//! recorded as notes only; no game data is part of this repository.
//!
//! ```text
//! chunk_save_parity <game dir> <sprite tables> <save tables> <save dir> <oracle dir>
//! ```
//!
//! `<oracle dir>` holds the original's `map/<wx>/<wy>.bin` re-saves,
//! `digest.txt`, `items.txt` (registry id, full type, creatable, obsolete)
//! and `summary.txt` (world age). Prints counts; writes `rust_digest.txt`
//! and `rust_diffs.txt` into `<oracle dir>`.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_world::chunk::IsoChunk;
use pz_world::objects::{IsoObject, ItemContainer, ObjectKind, ObjectName, SpriteName};
use pz_world::save::chunk_file::{chunk_path, load_chunk, save_chunk};
use pz_world::save::items::{AllItems, ItemBlob, ItemRegistry};
use pz_world::save::object_io::sprite_save_id;
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{IsoSpriteManager, SpriteTables};

/// Item ids the original could create (`items.txt` of the oracle run).
struct OracleItems(HashMap<i16, (String, bool, bool)>);

/// Every id kept, never decoded: for comparing raw records.
struct RawOnly<'a>(&'a ScriptItems);
impl ItemRegistry for RawOnly<'_> {
    fn exists(&self, id: i16) -> bool {
        self.0.exists(id)
    }
    fn full_type(&self, id: i16) -> Option<String> {
        self.0.full_type(id)
    }
    fn obsolete(&self, t: &str) -> bool {
        self.0.obsolete(t)
    }
    fn blood_body_parts(&self) -> usize {
        self.0.blood_body_parts
    }
}

fn conts<'a>(it: impl Iterator<Item = &'a ItemContainer>) -> String {
    let v: Vec<&ItemContainer> = it.collect();
    let mut s = format!("{}", v.len());
    for c in v {
        write!(
            s,
            ":{}/{}/{}",
            c.kind.as_deref().unwrap_or(""),
            c.item_count(),
            c.capacity
        )
        .unwrap();
    }
    s
}

fn object_line(ctx: &SaveCtx<'_>, o: &IsoObject) -> String {
    let name = match &o.name {
        None => "null".to_string(),
        Some(ObjectName::Grass) => "Grass".into(),
        Some(ObjectName::Str(s)) => s.clone(),
        Some(ObjectName::Dict(i)) => format!("#{i}"),
    };
    let sname = match &o.sprite_name {
        None => "null".to_string(),
        Some(SpriteName::Str(s)) => s.clone(),
        Some(SpriteName::Dict(i)) => format!("#{i}"),
    };
    let containers: Vec<&ItemContainer> = match &o.kind {
        ObjectKind::Mannequin(Some(m)) => m.container.iter().collect(),
        _ => o.containers().collect(),
    };
    format!(
        "  o {} spr={} name={} sname={} att={} wbs={} md={} ent={} conts={}",
        o.kind.class_name(),
        sprite_save_id(ctx, o),
        name,
        sname,
        o.attached_anim_sprite
            .as_ref()
            .map_or(-1, |a| a.len() as i64),
        o.wall_blood_splats.as_ref().map_or(-1, |a| a.len() as i64),
        match &o.kind {
            // The digest's `md` is the subclass's own `table` field.
            ObjectKind::Thumpable(t) => t.table.as_ref(),
            _ => o.mod_data.as_ref(),
        }
        .is_some_and(|t| !t.is_empty()) as u8,
        (o.has_components || !o.components.is_empty()) as u8,
        conts(containers.into_iter())
    )
}

fn digest(ctx: &SaveCtx<'_>, ch: &IsoChunk, out: &mut String) {
    let s = &ch.saved;
    writeln!(
        out,
        "chunk {},{} levels {}..{} splats={} gens={} loot={} rooms={}",
        ch.wx,
        ch.wy,
        ch.min_level,
        ch.max_level,
        s.floor_blood_splats.len(),
        s.generators.as_ref().map_or(0, Vec::len),
        s.loot_respawn_hour,
        s.spawned_rooms.len()
    )
    .unwrap();
    for x in 0..8 {
        for y in 0..8 {
            for z in ch.min_level..=ch.max_level {
                let Some(sq) = ch.get_grid_square(x, y, z) else {
                    continue;
                };
                let seen = sq
                    .seen
                    .iter()
                    .enumerate()
                    .fold(0, |a, (i, &b)| a | ((b as i32) << i));
                writeln!(
                    out,
                    " sq {},{},{} objs={} special={} world={} moving={} table={} overlay={} roof={} burnt={} rope={} seen={}",
                    sq.x, sq.y, sq.z, sq.objects.len(), sq.special_objects.len(), sq.world_objects.len(),
                    sq.static_moving.len(), sq.mod_data.as_ref().is_some_and(|t| !t.is_empty()) as u8,
                    sq.overlay_done as u8, sq.have_roof as u8, sq.burnt_out as u8, sq.have_sheet_rope as u8, seen
                )
                .unwrap();
                for o in &sq.objects {
                    writeln!(out, "{}", object_line(ctx, o)).unwrap();
                }
                for b in &sq.static_moving {
                    writeln!(
                        out,
                        "  b IsoDeadBody spr=20000000 name=null sname=null att=-1 wbs=-1 md={} ent=0 conts={}",
                        b.base.mod_data.as_ref().is_some_and(|t| !t.is_empty()) as u8,
                        conts(b.container.iter())
                    )
                    .unwrap();
                }
            }
        }
    }
}

/// Every item record of the chunk (for comparing with item bodies masked).
fn for_each_item(ch: &mut IsoChunk, f: &mut dyn FnMut(&mut ItemBlob)) {
    fn cont(c: &mut ItemContainer, f: &mut dyn FnMut(&mut ItemBlob)) {
        for r in &mut c.items {
            f(r);
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            for z in ch.min_level..=ch.max_level {
                let Some(sq) = ch.get_grid_square_mut(x, y, z) else {
                    continue;
                };
                for o in &mut sq.objects {
                    if let Some(c) = &mut o.container {
                        cont(c, f);
                    }
                    for c in &mut o.secondary_containers {
                        cont(c, f);
                    }
                    match &mut o.kind {
                        ObjectKind::WorldInventory(w) => f(&mut w.item),
                        ObjectKind::Mannequin(Some(m)) => {
                            if let Some(c) = &mut m.container {
                                cont(c, f);
                            }
                        }
                        ObjectKind::Trap(t) => {
                            if let Some(i) = &mut t.weapon {
                                f(i);
                            }
                        }
                        ObjectKind::CarBatteryCharger(c) => {
                            for i in [&mut c.item, &mut c.battery].into_iter().flatten() {
                                f(i);
                            }
                        }
                        ObjectKind::Hutch(h) => {
                            for nb in h.master.iter_mut().flat_map(|m| m.nest_boxes.iter_mut()) {
                                for e in &mut nb.eggs {
                                    f(e);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                for b in &mut sq.static_moving {
                    if let Some(c) = &mut b.container {
                        cont(c, f);
                    }
                }
            }
        }
    }
}

fn mask_items(ch: &mut IsoChunk) {
    let env = AllItems;
    for_each_item(ch, &mut |i| {
        let mut rec = i.record(&env).unwrap_or_default();
        rec.truncate(7);
        *i = ItemBlob::Raw(rec);
    });
}

/// The value of `key=` up to the next ` <word>=`.
fn field<'a>(l: &'a str, key: &str, next: &str) -> Option<(usize, usize, &'a str)> {
    let a = l.find(key)? + key.len();
    let b = l[a..].find(next)? + a;
    Some((a, b, &l[a..b]))
}

fn without(l: &str, key: &str, next: &str) -> String {
    match field(l, key, next) {
        Some((a, b, _)) => format!("{}{}", &l[..a], &l[b..]),
        None => l.to_string(),
    }
}

/// Digest lines are equal up to what the Rust side cannot print: names
/// stored as `WorldDictionary` ids (checked for a consistent mapping) and
/// the sprite of an `IsoBarricade`, which is not saved but chosen from its
/// state by `chooseSprite()`.
fn lines_match(r: &str, j: &str, dict: &mut HashMap<String, String>) -> bool {
    if r == j {
        return true;
    }
    let (mut r, mut j) = (r.to_string(), j.to_string());
    if let (Some((_, _, rn)), Some((_, _, jn))) = (
        field(&r, " name=", " sname="),
        field(&j, " name=", " sname="),
    ) {
        if rn.starts_with('#') {
            if dict.entry(rn.to_string()).or_insert_with(|| jn.to_string()) != jn {
                return false;
            }
            r = without(&r, " name=", " sname=");
            j = without(&j, " name=", " sname=");
        }
    }
    if field(&r, " sname=", " att=").is_some_and(|f| f.2.starts_with('#')) {
        r = without(&r, " sname=", " att=");
        j = without(&j, " sname=", " att=");
    }
    if r.contains(" IsoBarricade ") {
        r = without(&r, " spr=", " name=");
        j = without(&j, " spr=", " name=");
    }
    r == j
}

/// Names the first object whose saved bytes differ between two chunks.
fn first_object_diff(ctx: &SaveCtx<'_>, r: &IsoChunk, j: &IsoChunk, out: &mut String) {
    use pz_world::save::io::Writer;
    use pz_world::save::object_io::save_object;
    for (rs, js) in r.squares().zip(j.squares()) {
        for (ro, jo) in rs.objects.iter().zip(js.objects.iter()) {
            let (mut a, mut b) = (Writer::new(), Writer::new());
            let _ = save_object(&mut a, ctx, ro);
            let _ = save_object(&mut b, ctx, jo);
            if a.buf != b.buf {
                let name = ro
                    .sprite
                    .and_then(|s| ctx.sprites.sprites[s].name.clone())
                    .unwrap_or_default();
                writeln!(
                    out,
                    "  OBJ {} {} at {},{},{}: rust {:?}\n    java {:?}",
                    ro.kind.class_name(),
                    name,
                    rs.x,
                    rs.y,
                    rs.z,
                    ro.components.iter().map(|c| c.type_id).collect::<Vec<_>>(),
                    jo.components.iter().map(|c| c.type_id).collect::<Vec<_>>()
                )
                .unwrap();
                return;
            }
        }
        if rs.static_moving != js.static_moving {
            writeln!(out, "  BODY at {},{},{}", rs.x, rs.y, rs.z).unwrap();
            return;
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let oracle = Path::new(&a[5]);
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut reg = HashMap::new();
    let mut tile_scale = 2;
    for line in std::fs::read_to_string(oracle.join("items.txt"))
        .unwrap()
        .lines()
    {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] == "#tileScale" {
            tile_scale = f[1].parse().unwrap();
            continue;
        }
        reg.insert(
            f[0].parse::<i16>().unwrap(),
            (f[1].to_string(), f[2] == "1", f[3] == "1"),
        );
    }
    let summary = std::fs::read_to_string(oracle.join("summary.txt")).unwrap();
    let world_age: f64 = summary
        .split_whitespace()
        .find_map(|w| w.strip_prefix("worldAgeHours="))
        .unwrap()
        .parse()
        .unwrap();
    let settings = SaveSettings {
        world_age_hours: world_age,
        world_item_removal_list: "Base.Hat, Base.Glasses, Base.Maggots, Base.Slug, Base.Slug2, Base.Snail, Base.Worm, Base.Dung_Mouse, Base.Dung_Rat"
            .split(',')
            .map(|s| s.trim().to_string())
            .collect(),
        tile_scale,
        ..Default::default()
    };
    let oracle_items = OracleItems(reg);
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .expect("built-in registries");
    sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
        .expect("scripts");
    let dict = WorldDictionary::load(&std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap())
        .unwrap();
    let names = pz_inventory::factory::parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let mut registry = ScriptItems::new(&sm, dict, &names);
    registry.world_age_hours = world_age;
    registry.blood_body_parts = save_tables.blood_body_parts;
    registry.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    // Component blocks through pz-entity (set ITEM_COMPONENTS=opaque to keep bytes).
    if std::env::var("ITEM_COMPONENTS").as_deref() != Ok("opaque") {
        let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
        let ctx = pz_entity::BasicContext {
            attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
            fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
            ..Default::default()
        };
        let ctx: &'static pz_entity::BasicContext = Box::leak(Box::new(ctx));
        registry.components = Box::new(pz_inventory::component::EntityCodec { ctx });
    }
    let mut creatable_mismatch = 0;
    for (id, (ft, ok, _)) in &oracle_items.0 {
        if registry.exists(*id) != *ok {
            creatable_mismatch += 1;
            if creatable_mismatch <= 10 {
                eprintln!("creatable differs: {id} {ft} java={ok}");
            }
        }
    }
    let raw_env = RawOnly(&registry);
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: &registry,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut files = Vec::new();
    for d in std::fs::read_dir(save_dir.join("map")).unwrap() {
        let d = d.unwrap();
        let Ok(wx) = d.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        for f in std::fs::read_dir(d.path()).unwrap() {
            let n = f.unwrap().file_name().to_string_lossy().to_string();
            if let Some(wy) = n.strip_suffix(".bin").and_then(|s| s.parse::<i32>().ok()) {
                files.push((wx, wy));
            }
        }
    }
    files.sort();
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut dg = String::new();
    let mut diffs = String::new();
    let java_digest = std::fs::read_to_string(oracle.join("digest.txt")).unwrap();
    let mut java_chunks = HashMap::new();
    let mut cur: Option<(i32, i32)> = None;
    for line in java_digest.lines() {
        if let Some(rest) = line.strip_prefix("chunk ") {
            let xy = rest.split(' ').next().unwrap();
            let (x, y) = xy.split_once(',').unwrap();
            cur = Some((x.parse().unwrap(), y.parse().unwrap()));
        }
        let e: &mut String = java_chunks.entry(cur.unwrap()).or_default();
        e.push_str(line);
        e.push('\n');
    }
    let mut dict_names: HashMap<String, String> = HashMap::new();
    let mut item_hist = BTreeMap::<String, usize>::new();
    let mut item_examples = HashMap::<String, usize>::new();
    let mut item_diffs = String::new();
    for &(wx, wy) in &files {
        *counts.entry("files").or_default() += 1;
        let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
        let loaded = match load_chunk(&bytes, wx, wy, &ctx) {
            Ok(l) => l,
            Err(e) => {
                *counts.entry("load errors").or_default() += 1;
                writeln!(diffs, "LOADERR {wx},{wy} {e}").unwrap();
                continue;
            }
        };
        if !loaded.crc_ok {
            *counts.entry("crc mismatch in input").or_default() += 1;
        }
        let mut d = String::new();
        digest(&ctx, &loaded.chunk, &mut d);
        // Dictionary names are compared through a consistent id mapping.
        let java = java_chunks.get(&(wx, wy)).cloned().unwrap_or_default();
        let same = d.lines().count() == java.lines().count()
            && d.lines()
                .zip(java.lines())
                .all(|(r, j)| lines_match(r, j, &mut dict_names));
        if same {
            *counts.entry("digest equal").or_default() += 1;
        } else {
            *counts.entry("digest differs").or_default() += 1;
            let first = d
                .lines()
                .zip(java.lines())
                .find(|(r, j)| !lines_match(r, j, &mut dict_names.clone()))
                .map(|(r, j)| format!("rust: {r}\n  java: {j}"))
                .unwrap_or_else(|| "line count".into());
            writeln!(diffs, "DIGEST {wx},{wy}\n  {first}").unwrap();
        }
        dg.push_str(&d);
        let out = match save_chunk(&loaded.chunk, &ctx) {
            Ok(o) => o,
            Err(e) => {
                *counts.entry("save errors").or_default() += 1;
                writeln!(diffs, "SAVEERR {wx},{wy} {e}").unwrap();
                continue;
            }
        };
        if out == bytes {
            *counts.entry("rust resave == input").or_default() += 1;
        }
        if let Ok(dir) = std::env::var("RUST_OUT") {
            let p = chunk_path(Path::new(&dir), wx, wy);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, &out).unwrap();
        }
        let java_bytes = std::fs::read(chunk_path(oracle, wx, wy)).unwrap();
        if out == java_bytes {
            *counts.entry("rust resave == java resave").or_default() += 1;
            continue;
        }
        // Item records of both re-saves, pairwise.
        {
            let rctx = SaveCtx {
                items: &raw_env,
                ..ctx
            };
            if let (Ok(mut ro), Ok(mut jo)) = (
                load_chunk(&out, wx, wy, &rctx),
                load_chunk(&java_bytes, wx, wy, &rctx),
            ) {
                let (mut rr, mut jr) = (Vec::new(), Vec::new());
                for_each_item(&mut ro.chunk, &mut |i| rr.push(i.record(&raw_env).unwrap()));
                for_each_item(&mut jo.chunk, &mut |i| jr.push(i.record(&raw_env).unwrap()));
                let mut any = false;
                if rr.len() != jr.len() {
                    *item_hist
                        .entry(format!("record count {} vs {}", rr.len(), jr.len()))
                        .or_default() += 0;
                    *item_hist
                        .entry("chunk: record count differs".into())
                        .or_default() += 1;
                    any = true;
                }
                for (a, b) in rr.iter().zip(jr.iter()) {
                    if a == b {
                        continue;
                    }
                    any = true;
                    let reg = i16::from_be_bytes([b[0], b[1]]);
                    let class = registry.template(reg).map_or("?", |t| t.class.java_name());
                    let at = a
                        .iter()
                        .zip(b.iter())
                        .position(|(x, y)| x != y)
                        .unwrap_or(a.len().min(b.len()));
                    let key = format!(
                        "{class} len {}",
                        if a.len() == b.len() {
                            "same"
                        } else {
                            "differs"
                        }
                    );
                    *item_hist.entry(key.clone()).or_default() += 1;
                    if item_examples.entry(key).or_insert(0usize) < &mut 6 {
                        *item_examples
                            .get_mut(&format!(
                                "{class} len {}",
                                if a.len() == b.len() {
                                    "same"
                                } else {
                                    "differs"
                                }
                            ))
                            .unwrap() += 1;
                        let lo = at.saturating_sub(8);
                        writeln!(
                            item_diffs,
                            "{wx},{wy} {class} {} at {at} rust={} java={}\n  rust:{:02x?}\n  java:{:02x?}",
                            registry.full_type(reg).unwrap_or_default(),
                            a.len(),
                            b.len(),
                            &a[lo..(at + 24).min(a.len())],
                            &b[lo..(at + 24).min(b.len())]
                        )
                        .unwrap();
                    }
                }
                if any {
                    *counts
                        .entry("chunks with differing item records")
                        .or_default() += 1;
                } else {
                    writeln!(
                        item_diffs,
                        "{wx},{wy} item records equal, grouping or framing differs"
                    )
                    .unwrap();
                }
            }
        }
        // Compare with item bodies masked: the Java side re-saves items
        // through their own classes.
        let java_loaded = load_chunk(&java_bytes, wx, wy, &ctx);
        let (rm, jm) = match java_loaded {
            Ok(j) => {
                let (mut rc, mut jc) = (loaded.chunk, j.chunk);
                mask_items(&mut rc);
                mask_items(&mut jc);
                (
                    save_chunk(&rc, &ctx).unwrap_or_default(),
                    save_chunk(&jc, &ctx).unwrap_or_default(),
                )
            }
            Err(e) => {
                writeln!(diffs, "JAVALOADERR {wx},{wy} {e}").unwrap();
                (Vec::new(), vec![1])
            }
        };
        if rm == jm {
            *counts
                .entry("differs only inside item records")
                .or_default() += 1;
        } else {
            if let (Ok(mut rl), Ok(mut jl)) = (
                load_chunk(&bytes, wx, wy, &ctx),
                load_chunk(&java_bytes, wx, wy, &ctx),
            ) {
                mask_items(&mut rl.chunk);
                mask_items(&mut jl.chunk);
                first_object_diff(&ctx, &rl.chunk, &jl.chunk, &mut diffs);
            }
            *counts.entry("differs outside item records").or_default() += 1;
            let at = rm
                .iter()
                .zip(jm.iter())
                .skip(17)
                .position(|(a, b)| a != b)
                .map_or(rm.len().min(jm.len()), |p| p + 17);
            let lo = at.saturating_sub(12);
            writeln!(
                diffs,
                "BYTES {wx},{wy} (items masked) rust={} java={} first={at} rust:{:02x?} java:{:02x?}",
                rm.len(),
                jm.len(),
                &rm[lo.min(rm.len())..(at + 12).min(rm.len())],
                &jm[lo.min(jm.len())..(at + 12).min(jm.len())]
            )
            .unwrap();
        }
    }
    std::fs::write(oracle.join("rust_digest.txt"), dg).unwrap();
    std::fs::write(oracle.join("rust_diffs.txt"), diffs).unwrap();
    std::fs::write(oracle.join("rust_item_diffs.txt"), item_diffs).unwrap();
    for (k, v) in &item_hist {
        println!("ITEMDIFF {v} {k}");
    }
    for (k, v) in registry.raw_counts.borrow().iter() {
        println!("RAW {v} {k}");
    }
    println!("creatable mismatches vs original: {creatable_mismatch}");
    println!("{counts:?}");
}

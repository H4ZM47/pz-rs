//! Calls `InventoryItem` methods through Lua on items created the way
//! `instanceItem` does, to compare with the original's virtual dispatch.
//!
//! For each item type and each operation line (`fresh`, or a method name
//! and its arguments), creates the item with `Rand` seeded as
//! `new java.util.Random(1000 + 100 * typeIndex + opIndex)`, calls the
//! operation through the item's Lua metatable, then each getter, and prints
//! one tab-separated row: the type, the operation, the item's class, the
//! operation's result and `getter=result` for each getter. Results are
//! `nil`, `true`/`false`, `n:<f64 bits in hex>`, `s:<string>`,
//! `o:<tostring>`, `NOMETHOD` (no such method on the item's metatable) or
//! `ERR`. No game data or harness output is part of this repository.
//!
//! ```text
//! item_dispatch_parity <game dir> <types file> <ops file> <getters file> <out tsv>
//! ```

use pz_kahlua::{Thread, Value};
use pz_lua_api::bind_items::{create_item, install_item_env, item_value, GameItemEnv};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;

const CALL: &str = "local item, name = ...\n\
    local m = item[name]\n\
    if m == nil then return 'NOMETHOD' end\n\
    return m(item, select(3, ...))";

fn fmt(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Nil) => "nil".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Num(n)) => format!("n:{:x}", n.to_bits()),
        Some(Value::Str(s)) => format!(
            "s:{}",
            pz_kahlua::exec::java_value_of(&Value::Str(s.clone()))
        ),
        Some(Value::Object(o)) if o.class_name().ends_with("DeviceData") => "obj:DeviceData".into(),
        Some(Value::Object(o)) => format!("o:{}", o.to_lua_string()),
        Some(other) => format!("?:{}", pz_kahlua::exec::java_value_of(other)),
    }
}

fn arg(a: &str) -> Value {
    match a {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => a
            .parse::<f64>()
            .map(Value::Num)
            .unwrap_or_else(|_| Value::str(a)),
    }
}

fn call(t: &mut Thread, f: &Value, item: &Value, name: &str, args: &[&str]) -> String {
    let mut a = vec![item.clone(), Value::str(name)];
    a.extend(args.iter().map(|s| arg(s)));
    match t.pcall(f, &a) {
        Ok(r) if matches!(r.first(), Some(Value::Str(s)) if s.as_ref() == "NOMETHOD") => {
            "NOMETHOD".into()
        }
        Ok(r) => fmt(r.first()),
        Err(_) => "ERR".into(),
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let game = PathBuf::from(&args[1]);
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    let (types, ops, gets) = (read(&args[2])?, read(&args[3])?, read(&args[4])?);
    let mut fs = pz_fs::GameFs::new(&game);
    let sm = pz_scripts::ScriptManager::new();
    let jar = game.join("projectzomboid.jar");
    sm.load_builtin_registries(&jar)
        .map_err(|e| e.to_string())?;
    sm.load(&fs, &[]).map_err(|e| e.to_string())?;
    let entity = Rc::new(pz_lua_api::bind_items::item_entity_context(&jar, &sm)?);
    pz_lua_api::bind_registry::install_script_manager(Rc::new(sm));
    {
        let core = pz_lua_api::bind_core::shared_core();
        pz_lua_api::bind_text::boot(&fs, &core.borrow())?;
    }
    pz_lua_api::bind_appearance::shared_population()
        .borrow_mut()
        .init(&mut fs);
    let assets = pz_inventory::ItemAssets::load(&game)?;

    let mut t = Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).map_err(|e| format!("{e:?}"))?;
    pz_lua_api::bind_java_util::register(&mut t);
    pz_lua_api::bind_globals::register(&mut t);
    pz_lua_api::bind_items::register(&mut t);
    pz_lua_api::bind_item_visual::register(&mut t);
    pz_lua_api::bind_vehicles::register(&mut t);
    let f = pz_kahlua::stdlib::load_string(CALL, Some("call"), t.globals().clone())
        .map_err(|e| format!("{e:?}"))?;

    let gets: Vec<&str> = gets
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let ops: Vec<&str> = ops.lines().map(str::trim).collect();
    let mut out = String::new();
    for (i, ty) in types.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        for (p, op) in ops.iter().enumerate() {
            if op.is_empty() {
                continue;
            }
            let mut env = GameItemEnv::new(Some(assets.clone()));
            env.rand = Some(pz_core::random::Rand::new(1000 + 100 * i as i64 + p as i64));
            env.entity = Some(entity.clone());
            install_item_env(Rc::new(env));
            let Some(item) = create_item(ty, 1.0) else {
                writeln!(out, "{ty}\t{op}\tnull").unwrap();
                continue;
            };
            let class = pz_lua_api::bind_items::class_of(&item.borrow());
            let v = item_value(&item);
            let _ = write!(
                out,
                "{ty}\t{op}\tclass={}",
                class.rsplit('.').next().unwrap()
            );
            if *op != "fresh" {
                let w: Vec<&str> = op.split(' ').collect();
                let r = call(&mut t, &f, &v, w[0], &w[1..]);
                let _ = write!(out, "\t={r}");
            }
            for g in &gets {
                let r = call(&mut t, &f, &v, g, &[]);
                let _ = write!(out, "\t{g}={r}");
            }
            out.push('\n');
        }
    }
    std::fs::write(&args[5], out).map_err(|e| e.to_string())
}

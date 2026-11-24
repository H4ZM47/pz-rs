//! `LuaManager.GlobalObject` global functions that don't depend on a
//! ported subsystem yet. Subsystem-specific globals live in their bind_*
//! modules.

use crate::bind_java_util::{list_value, JavaList};
use crate::bind_registry::shared_script_manager;
use crate::expose::{bool_arg, global, int_arg, num_arg, str_arg};
use pz_kahlua::{TableRef, Thread, Value};
use pz_scripts::objects::items::find_item;
use pz_scripts::objects::vehicles::VehicleScript;
use pz_scripts::ScriptType;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `LuaManager.copyTable(null, from)`: deep copy of nested tables, keeping
/// iteration order.
pub fn copy_table(from: &TableRef) -> TableRef {
    let to = TableRef::new();
    for (k, v) in from.entries() {
        match v {
            Value::Table(t) => to.rawset(k, Value::Table(copy_table(&t))),
            v => to.rawset(k, v),
        }
    }
    to
}

/// Java `isInstance` by simple class name.
pub fn instance_of(v: &Value, name: &str) -> bool {
    match v {
        Value::Nil => false,
        Value::Object(o) => {
            let simple = |c: &str| crate::expose::simple_name(c).to_string();
            simple(o.class_name()) == name || o.super_classes().iter().any(|c| simple(c) == name)
        }
        Value::Closure(_) => name == "LuaClosure",
        Value::Table(_) => name == "KahluaTableImpl",
        Value::Str(_) => matches!(name, "String" | "Object" | "CharSequence" | "Comparable"),
        Value::Num(_) => matches!(name, "Double" | "Number" | "Object" | "Comparable"),
        Value::Bool(_) => matches!(name, "Boolean" | "Object" | "Comparable"),
        _ => false,
    }
}

thread_local! {
    /// `TextureID.useFiltering`.
    pub static USE_TEXTURE_FILTERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn register(thread: &mut Thread) {
    let ret_false: &[&str] = &[
        "isServer",
        "isClient",
        "isDemo",
        "isCoopHost",
        "isMultiplayer",
        "isServerSoftReset",
        "getDebug",
        // `GameClient.client && connection.getRole() == Roles.getDefaultForAdmin()`.
        "isAdmin",
    ];
    for name in ret_false {
        global(thread, name, |c| Ok(c.push(false)));
    }
    global(thread, "isDesktopOpenSupported", |c| Ok(c.push(true)));
    global(thread, "getCellSizeInSquares", |c| Ok(c.push(256.0)));
    global(thread, "getChunkSizeInSquares", |c| Ok(c.push(8.0)));
    global(thread, "copyTable", |c| {
        let r = match c.arg(0) {
            Value::Table(t) => copy_table(&t),
            _ => TableRef::new(),
        };
        Ok(c.push(r))
    });
    // `showDebugInfoInChat(text)`: only a client (`GameClient.client`) with
    // the Action or Trace debug log shows it in the chat; this is not one.
    global(thread, "showDebugInfoInChat", |_| Ok(0));
    // `sendAddItemToContainer(container, item)`: only a server
    // (`GameServer.server`) sends it to the clients.
    global(thread, "sendAddItemToContainer", |_| Ok(0));
    // `convertToPZNetTable(table)`: a shallow copy.
    global(thread, "convertToPZNetTable", |c| {
        let Value::Table(from) = c.arg(0) else {
            return Err(pz_kahlua::LuaError::runtime("java.lang.ClassCastException"));
        };
        let to = TableRef::new();
        for (k, v) in from.entries() {
            to.rawset(k, v);
        }
        Ok(c.push(to))
    });
    global(thread, "mergeTable", |c| {
        let result = TableRef::new();
        let mut index = 1;
        for i in 0..c.nargs() {
            if let Value::Table(t) = c.arg(i) {
                let n = t.len();
                for j in 1..=n {
                    result.set_int(index, t.get_int(j));
                    index += 1;
                }
            }
        }
        Ok(c.push(result))
    });
    global(thread, "instanceof", |c| {
        let name = str_arg(c, 1).unwrap_or_else(|| std::rc::Rc::from(""));
        let r = instance_of(&c.arg(0), &name);
        Ok(c.push(r))
    });
    // Tables report their `Type` field; engine objects their Java simple
    // class name; everything else its Lua type.
    global(thread, "typeof", |c| {
        let o = c.arg(0);
        let r = match &o {
            Value::Object(obj) => crate::expose::simple_name(obj.class_name()).to_string(),
            Value::Table(t) => match t.get_str("Type") {
                Value::Str(s) => s.to_string(),
                _ => "table".to_string(),
            },
            other => other.type_name().to_string(),
        };
        Ok(c.push(r))
    });
    global(thread, "getClassSimpleName", |c| {
        let r = match c.arg(0) {
            Value::Object(obj) => crate::expose::simple_name(obj.class_name()).to_string(),
            other => crate::expose::simple_name(&pz_kahlua::stdlib::java_class_name(&other)).to_string(),
        };
        Ok(c.push(r))
    });
    global(thread, "getTimestamp", |c| {
        Ok(c.push((now_ms() / 1000) as f64))
    });
    global(thread, "getTimestampMs", |c| Ok(c.push(now_ms() as f64)));
    global(thread, "getTimeInMillis", |c| Ok(c.push(now_ms() as f64)));
    global(thread, "getFileSeparator", |c| {
        Ok(c.push(std::path::MAIN_SEPARATOR.to_string()))
    });
    global(thread, "isSystemMacOS", |c| {
        Ok(c.push(cfg!(target_os = "macos")))
    });
    global(thread, "isSystemWindows", |c| {
        Ok(c.push(cfg!(target_os = "windows")))
    });
    global(thread, "isSystemLinux", |c| {
        Ok(c.push(!cfg!(target_os = "macos") && !cfg!(target_os = "windows")))
    });
    // Steam integration is not part of this port's runtime yet.
    global(thread, "getSteamModeActive", |c| Ok(c.push(false)));
    global(thread, "isSteamOverlayEnabled", |c| Ok(c.push(false)));
    // TextureID.useFiltering: read by texture loading in pz-render.
    global(thread, "useTextureFiltering", |c| {
        USE_TEXTURE_FILTERING.with(|f| f.set(crate::expose::bool_arg(c, 0)));
        Ok(0)
    });
    // Only returns a trace in debug mode with DebugType.ISUIStackTrace on.
    global(thread, "getISUIStackTrace", |c| Ok(c.push(Value::Nil)));
    // Build 42.22 `LuaManager.GlobalObject` math / string / UUID / world-level helpers.
    global(thread, "fastfloor", |c| {
        let v = (num_arg(c, 0) as f32).floor() as i32;
        Ok(c.push(v as f64))
    });
    global(thread, "getMinimumWorldLevel", |c| Ok(c.push(-32.0)));
    global(thread, "getMaximumWorldLevel", |c| Ok(c.push(31.0)));
    global(thread, "moduleDotType", |c| {
        let Some(ty) = str_arg(c, 1) else {
            return Ok(c.push(Value::Nil));
        };
        if ty.contains('.') {
            return Ok(c.push(ty.as_ref()));
        }
        let module = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        Ok(c.push(format!("{module}.{ty}")))
    });
    global(thread, "tabToX", |c| {
        let s = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let max_x = int_arg(c, 1).max(0) as usize;
        let pad = max_x.saturating_sub(s.encode_utf16().count());
        let mut out = String::with_capacity(s.len() + pad);
        out.push_str(&s);
        for _ in 0..pad {
            out.push(' ');
        }
        Ok(c.push(out))
    });
    // Build 42.22: `return !pattern.contains("[");`
    global(thread, "checkStringPattern", |c| {
        let ok = str_arg(c, 0).map_or(true, |s| !s.contains('['));
        Ok(c.push(ok))
    });
    // `callLua(func, arg)`, `callLuaBool(func, arg)`, `callLuaReturn(func, arrayList)`.
    global(thread, "callLua", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(0);
        };
        let f = c.thread.globals().get_str(&name);
        if !f.is_nil() {
            let arg = c.arg(1);
            let _ = c.thread.pcall(&f, &[arg]);
        }
        Ok(0)
    });
    global(thread, "callLuaBool", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(false));
        };
        let f = c.thread.globals().get_str(&name);
        if f.is_nil() {
            return Ok(c.push(false));
        }
        let arg = c.arg(1);
        let ok = match c.thread.pcall(&f, &[arg]) {
            Ok(vals) => matches!(vals.first(), Some(Value::Bool(true))),
            Err(_) => false,
        };
        Ok(c.push(ok))
    });
    global(thread, "callLuaReturn", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(list_value(Vec::new())));
        };
        let f = c.thread.globals().get_str(&name);
        if f.is_nil() {
            return Ok(c.push(list_value(Vec::new())));
        }
        let args: Vec<Value> = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaList>()
                .map(|l| l.items.borrow().clone())
                .unwrap_or_default(),
            Value::Nil => Vec::new(),
            other => vec![other],
        };
        let out = c.thread.pcall(&f, &args).unwrap_or_default();
        Ok(c.push(list_value(out)))
    });
    // Item & recipe script lookup helpers.
    global(thread, "getItemDisplayName", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let sm = shared_script_manager();
        let display = find_item(&sm, &name)
            .and_then(|item| item.borrow().display_name.clone())
            .unwrap_or_else(|| name.to_string());
        Ok(c.push(display))
    });
    global(thread, "getItemFoodType", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let sm = shared_script_manager();
        let food_type = find_item(&sm, &name).and_then(|item| item.borrow().food_type.clone());
        Ok(c.push(food_type.map_or(Value::Nil, |s| Value::str(&s))))
    });
    global(thread, "getItemActualWeight", |c| {
        let weight = str_arg(c, 0)
            .and_then(|name| {
                let sm = shared_script_manager();
                find_item(&sm, &name).map(|item| f64::from(item.borrow().actual_weight))
            })
            .unwrap_or(0.0);
        Ok(c.push(weight))
    });
    global(thread, "getItemWeight", |c| {
        let weight = str_arg(c, 0)
            .and_then(|name| {
                let sm = shared_script_manager();
                find_item(&sm, &name).map(|item| {
                    let b = item.borrow();
                    if b.is_item_type(&sm, "WEAPON") {
                        f64::from(b.weapon_weight)
                    } else {
                        f64::from(b.actual_weight)
                    }
                })
            })
            .unwrap_or(0.0);
        Ok(c.push(weight))
    });
    global(thread, "getItemConditionMax", |c| {
        let max = str_arg(c, 0)
            .and_then(|name| {
                let sm = shared_script_manager();
                find_item(&sm, &name).map(|item| item.borrow().condition_max as f64)
            })
            .unwrap_or(0.0);
        Ok(c.push(max))
    });
    global(thread, "getRecipeDisplayName", |c| {
        let name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let display = crate::bind_text::with_translator_mut(|t| t.get_recipe_name(&name))
            .unwrap_or(name);
        Ok(c.push(display))
    });
    global(thread, "getAllVehicles", |c| {
        let sm = shared_script_manager();
        let mut names: Vec<String> = sm
            .collection(ScriptType::Vehicle)
            .get_all_scripts()
            .into_iter()
            .filter_map(|s| s.downcast::<VehicleScript>().map(|v| v.borrow().full_name()))
            .collect();
        names.sort();
        Ok(c.push(list_value(
            names.into_iter().map(|n| Value::str(&n)).collect(),
        )))
    });
    global(thread, "getVehicleById", |c| {
        let id = int_arg(c, 0) as i16;
        let v = pz_vehicles::manager::vehicle_by_id(id)
            .map_or(Value::Nil, |v| pz_vehicles::lua::vehicle_value(&v));
        Ok(c.push(v))
    });
    global(thread, "useStaticErosionRand", |c| {
        crate::bind_erosion::set_static_erosion_rand(bool_arg(c, 0));
        Ok(0)
    });
    // `sledgeDestroy(object)`: sends `PacketType.SledgehammerDestroy` only when
    // `GameClient.client && ServerOptions.allowDestructionBySledgehammer`.
    global(thread, "sledgeDestroy", |_| Ok(0));
    // getPlayer, getSpecificPlayer, getNumActivePlayers: bind_players.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_table_is_deep_and_ordered() {
        let inner = TableRef::new();
        inner.set_str("z", 1);
        let t = TableRef::new();
        t.set_str("b", inner.clone());
        t.set_str("a", 2);
        let c = copy_table(&t);
        assert_eq!(
            c.keys()
                .iter()
                .map(|k| k.as_str().unwrap().to_string())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(!c.get_str("b").as_table().unwrap().ptr_eq(&inner));
    }

    #[test]
    fn global_object_helpers_match_build_42_22() {
        let mut thread = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        crate::bind_erosion::register(&mut thread);
        crate::bind_zombie::register(&mut thread);
        crate::bind_world::register(&mut thread);
        register(&mut thread);

        // Install a minimal meta_grid with a regular zone and a vehicle zone.
        let mut meta = pz_world::IsoMetaGrid::with_bounds(10, 20, 12, 22);
        let _ = meta.register_zone("Town", "TownZone", 10 * 256 + 5, 20 * 256 + 5, 0, 20, 20);
        let _ = meta.register_vehicles_zone("Parking", "ParkingStall", 10 * 256 + 5, 20 * 256 + 5, 0, 10, 10, None);
        crate::bind_world::with_world(|w| w.meta_grid = Some(meta));

        let script = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            assert(fastfloor(3.9) == 3)
            assert(fastfloor(-1.2) == -2)
            assert(getMinimumWorldLevel() == -32)
            assert(getMaximumWorldLevel() == 31)
            assert(moduleDotType("Base", "Axe") == "Base.Axe")
            assert(moduleDotType("Base", "Custom.Axe") == "Custom.Axe")
            assert(moduleDotType("Base", nil) == nil)
            assert(tabToX("abc", 6) == "abc   ")
            assert(tabToX("abcdef", 3) == "abcdef")
            assert(checkStringPattern("Hammer") == true)
            assert(checkStringPattern("Bad[Pattern") == false)

            _G.CalledArg = nil
            function TestLuaVoid(x) _G.CalledArg = x end
            callLua("TestLuaVoid", 42)
            assert(_G.CalledArg == 42)

            function TestLuaBool(x) return x > 10 end
            assert(callLuaBool("TestLuaBool", 20) == true)
            assert(callLuaBool("TestLuaBool", 5) == false)
            assert(callLuaBool("MissingFunc", 5) == false)

            function TestLuaRet(a, b) return a + b, a * b end
            local args = ArrayList.new()
            args:add(3)
            args:add(4)
            local out = callLuaReturn("TestLuaRet", args)
            assert(out:size() == 2 and out:get(0) == 7 and out:get(1) == 12)

            assert(getItemDisplayName("Base.MissingItem") == "Base.MissingItem")
            assert(getItemFoodType("Base.MissingItem") == nil)
            assert(getItemActualWeight("Base.MissingItem") == 0)
            assert(getItemWeight("Base.MissingItem") == 0)
            assert(getItemConditionMax("Base.MissingItem") == 0)
            assert(getRecipeDisplayName("Make Bread") == "Make Bread")
            assert(getAllVehicles():size() >= 0)
            assert(getVehicleById(9999) == nil)

            assert(getCellMinX() == 10 and getCellMaxX() == 12)
            assert(getCellMinY() == 20 and getCellMaxY() == 22)
            local vz = getVehicleZoneAt(10 * 256 + 7, 20 * 256 + 7, 0)
            assert(vz ~= nil and vz:getName() == "Parking")

            useStaticErosionRand(true)
            AddNoiseToken(nil, 10)
            sledgeDestroy(nil)
            return true
        "#;
        let f = pz_kahlua::stdlib::load_string(
            script,
            Some("bind_globals_test"),
            thread.globals().clone(),
        )
        .unwrap();
        let res = thread
            .pcall(&f, &[])
            .unwrap_or_else(|(e, tb)| panic!("lua failed: {e:?}\n{tb}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
        assert!(crate::bind_erosion::static_erosion_rand());
        crate::bind_erosion::set_static_erosion_rand(false);
    }
}

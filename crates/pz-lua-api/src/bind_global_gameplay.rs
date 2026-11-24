//! Additional Build 42.22 `LuaManager$GlobalObject` gameplay, sync, closure,
//! sorting, and overhead-map helpers:
//! - Variadic command & SP-safe sync helpers: `sendClientCommandV`,
//!   `sendServerCommandV`, `syncItemFields`, `syncHandWeaponFields`,
//!   `syncBodyPart`, `syncPlayerStats`, `sendPlayerExtraInfo`,
//!   `sendPlaySound`, `playServerSound`, `createItemTransaction`,
//!   `removeItemTransaction`, `isItemTransactionConsistent`, `removeAction`
//! - World/character/item helpers: `addXpNoMultiplier`, `addBloodSplat`,
//!   `stopFire`, `setPuddles`, `getItemStaticModel`,
//!   `getAllAnimalsDefinitions`, `getMapFoldersForMod`
//! - Closure/sorting/UI helpers: `getFirstLineOfClosure`,
//!   `getFilenameOfClosure`, `timSort`,
//!   `translatePointXInOverheadMapToWindow`,
//!   `translatePointYInOverheadMapToWindow`, `detectBadWords`,
//!   `isAnimationRecorderActive`, `setPlayerJoypad`,
//!   `revertToKeyboardAndMouse`, `doLuaDebuggerAction`

use crate::bind_java_util::JavaList;
use crate::bind_registry::shared_script_manager;
use crate::expose::{global, int_arg, num_arg, str_arg};
use pz_kahlua::{TableRef, Thread, Value};
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static NEXT_TX_ID: Cell<i8> = const { Cell::new(1) };
}

pub(crate) fn reset_session_handles() {
    NEXT_TX_ID.with(|id| id.set(1));
}

fn build_varargs_table(c: &pz_kahlua::NativeCall<'_>, start: usize) -> TableRef {
    let tbl = TableRef::new();
    let mut i = start;
    while i + 1 < c.nargs() {
        let k = c.arg(i);
        let v = c.arg(i + 1);
        if !k.is_nil() {
            tbl.rawset(k, v);
        }
        i += 2;
    }
    tbl
}

pub fn register(thread: &mut Thread) {
    // ---- Variadic client/server commands & SP-safe sync helpers ------------

    // `sendClientCommandV(player, module, command, k1, v1, ...)`:
    // packs key/value pairs into a table and delegates to `sendClientCommand`.
    global(thread, "sendClientCommandV", |c| {
        let player = c.arg(0);
        let module = c.arg(1);
        let command = c.arg(2);
        let args = Value::Table(build_varargs_table(c, 3));
        let send = c.thread.globals().get_str("sendClientCommand");
        if !send.is_nil() {
            let _ = c.thread.call(&send, &[player, module, command, args])?;
        }
        Ok(0)
    });

    // `sendServerCommandV(module, command, k1, v1, ...)`: SP local dispatch.
    global(thread, "sendServerCommandV", |c| {
        let module = c.arg(0);
        let command = c.arg(1);
        let args = Value::Table(build_varargs_table(c, 2));
        let send = c.thread.globals().get_str("sendServerCommand");
        if !send.is_nil() {
            let _ = c.thread.call(&send, &[module, command, args])?;
        }
        Ok(0)
    });

    for noop in [
        "syncItemFields",
        "syncHandWeaponFields",
        "syncBodyPart",
        "syncPlayerStats",
        "sendPlayerExtraInfo",
        "sendPlaySound",
        "playServerSound",
        "removeItemTransaction",
        "removeAction",
        "setPlayerJoypad",
        "revertToKeyboardAndMouse",
        "doLuaDebuggerAction",
    ] {
        global(thread, noop, |_c| Ok(0));
    }

    global(thread, "createItemTransaction", |c| {
        let id = NEXT_TX_ID.with(|cell| {
            let cur = cell.get();
            let next = if cur == i8::MAX { 1 } else { cur + 1 };
            cell.set(next);
            cur
        });
        Ok(c.push(id as f64))
    });

    global(thread, "isItemTransactionConsistent", |c| Ok(c.push(true)));

    // ---- World / character / item helpers ----------------------------------

    // `addXpNoMultiplier(player, perk, amount)` -> `player:getXp():AddXPNoMultiplier(perk, amount)`.
    global(thread, "addXpNoMultiplier", |c| {
        let player = c.arg(0);
        let perk = c.arg(1);
        let amount = c.arg(2);
        if player.is_nil() || perk.is_nil() {
            return Ok(0);
        }
        let get_xp = c.thread.table_get(&player, &Value::str("getXp"))?;
        if !get_xp.is_nil() {
            let xp = c.thread.call1(&get_xp, &[player])?;
            if !xp.is_nil() {
                let add = c.thread.table_get(&xp, &Value::str("AddXPNoMultiplier"))?;
                if !add.is_nil() {
                    let _ = c.thread.call(&add, &[xp, perk, amount])?;
                }
            }
        }
        Ok(0)
    });

    global(thread, "addBloodSplat", |_c| Ok(0));

    global(thread, "stopFire", |c| {
        let obj = c.arg(0);
        if !obj.is_nil() {
            let ext = c.thread.table_get(&obj, &Value::str("extinguishFire"))?;
            if !ext.is_nil() {
                let _ = c.thread.call(&ext, &[obj])?;
            }
        }
        Ok(0)
    });

    global(thread, "setPuddles", |c| {
        let v = (num_arg(c, 0) as f32).clamp(0.0, 1.0);
        let puddles = crate::bind_puddles::shared_puddles();
        let mut p = puddles.borrow_mut();
        p.state.muddy_puddles.set(v);
        p.state.wet_ground.set(v);
        Ok(0)
    });

    global(thread, "getItemStaticModel", |c| {
        let Some(name) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let sm = shared_script_manager();
        let model = sm
            .find_item(&name, true)
            .and_then(|s| s.downcast::<pz_scripts::objects::items::Item>())
            .and_then(|item| item.borrow().world_static_model.clone());
        Ok(c.push(model.as_deref().map_or(Value::Nil, Value::str)))
    });

    global(thread, "getAllAnimalsDefinitions", |c| {
        Ok(c.push(Value::Object(Rc::new(JavaList::new(Vec::new())))))
    });

    global(thread, "getMapFoldersForMod", |c| {
        let _mod_id = str_arg(c, 0);
        Ok(c.push(Value::Object(Rc::new(JavaList::new(Vec::new())))))
    });

    // ---- Closure, sorting, overhead-map & misc helpers ---------------------

    global(thread, "getFirstLineOfClosure", |c| {
        let line = match c.arg(0) {
            Value::Closure(cl) => cl.prototype.lines.first().copied().unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(line as f64))
    });

    global(thread, "getFilenameOfClosure", |c| {
        let name = match c.arg(0) {
            Value::Closure(cl) => cl
                .prototype
                .filename
                .as_deref()
                .or(cl.prototype.file.as_deref())
                .map(Value::str)
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        Ok(c.push(name))
    });

    // `timSort(table, comparator)`: stable insertion/merge sort on 1-based Lua
    // array `tbl` using `cmp(a, b)` returning true when `a < b`.
    global(thread, "timSort", |c| {
        let Value::Table(tbl) = c.arg(0) else {
            return Ok(0);
        };
        let cmp = c.arg(1);
        let mut items = Vec::new();
        let mut idx = 1i64;
        loop {
            let v = tbl.get_int(idx);
            if v.is_nil() {
                break;
            }
            items.push(v);
            idx += 1;
        }
        let n = items.len();
        if n <= 1 {
            return Ok(0);
        }
        // Stable insertion sort (matches TimSort stability without risking
        // panics on Lua comparator errors).
        for i in 1..n {
            let mut j = i;
            while j > 0 {
                let less = if cmp.is_nil() {
                    let a = pz_kahlua::exec::java_value_of(&items[j]);
                    let b = pz_kahlua::exec::java_value_of(&items[j - 1]);
                    a < b
                } else {
                    c.thread
                        .call1(&cmp, &[items[j].clone(), items[j - 1].clone()])?
                        .truthy()
                };
                if less {
                    items.swap(j - 1, j);
                    j -= 1;
                } else {
                    break;
                }
            }
        }
        for (i, v) in items.into_iter().enumerate() {
            tbl.rawset(Value::Num((i + 1) as f64), v);
        }
        Ok(0)
    });

    // `translatePointXInOverheadMapToWindow(x, ui, zoom, offset)` / `Y`:
    // `(x - offset) * zoom + ui:getAbsoluteX()`
    global(thread, "translatePointXInOverheadMapToWindow", |c| {
        let x = num_arg(c, 0);
        let ui = c.arg(1);
        let zoom = num_arg(c, 2);
        let offset = num_arg(c, 3);
        let abs_x = if !ui.is_nil() {
            let f = c.thread.table_get(&ui, &Value::str("getAbsoluteX"))?;
            if f.is_nil() {
                0.0
            } else {
                let r = c.thread.call1(&f, &[ui])?;
                pz_kahlua::exec::raw_tonumber(&r).unwrap_or(0.0)
            }
        } else {
            0.0
        };
        Ok(c.push((x - offset) * zoom + abs_x))
    });

    global(thread, "translatePointYInOverheadMapToWindow", |c| {
        let y = num_arg(c, 0);
        let ui = c.arg(1);
        let zoom = num_arg(c, 2);
        let offset = num_arg(c, 3);
        let abs_y = if !ui.is_nil() {
            let f = c.thread.table_get(&ui, &Value::str("getAbsoluteY"))?;
            if f.is_nil() {
                0.0
            } else {
                let r = c.thread.call1(&f, &[ui])?;
                pz_kahlua::exec::raw_tonumber(&r).unwrap_or(0.0)
            }
        } else {
            0.0
        };
        Ok(c.push((y - offset) * zoom + abs_y))
    });

    global(thread, "detectBadWords", |c| Ok(c.push(false)));
    global(thread, "isAnimationRecorderActive", |c| Ok(c.push(false)));
    let _ = int_arg;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib;

    #[test]
    fn global_gameplay_helpers_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_puddles::register(&mut t);
        register(&mut t);

        let src = r#"
            local received = nil
            function sendClientCommand(player, mod, cmd, args)
                received = { mod = mod, cmd = cmd, a = args.a, b = args.b }
            end
            sendClientCommandV(nil, "farming", "water", "a", 10, "b", "ok")

            local tx1 = createItemTransaction(nil, nil, nil, nil)
            local tx2 = createItemTransaction(nil, nil, nil, nil)
            local consistent = isItemTransactionConsistent(nil, nil, nil, nil, nil)

            local arr = { 4, 1, 3, 2 }
            timSort(arr, function(a, b) return a < b end)

            local fn = function() return 1 end
            local fn_line = getFirstLineOfClosure(fn)
            local fn_file = getFilenameOfClosure(fn)

            local ui = { getAbsoluteX = function() return 10 end, getAbsoluteY = function() return 20 end }
            local wx = translatePointXInOverheadMapToWindow(120, ui, 2.0, 100)
            local wy = translatePointYInOverheadMapToWindow(20, ui, 4.0, 10)

            setPuddles(0.75)
            local animals = getAllAnimalsDefinitions()
            local folders = getMapFoldersForMod("SomeMod")

            return received.mod, received.cmd, received.a, received.b,
                tx1, tx2, consistent,
                arr[1], arr[2], arr[3], arr[4],
                fn_line > 0, fn_file,
                wx, wy,
                animals:size(), folders:size(),
                detectBadWords("hello"), isAnimationRecorderActive()
        "#;
        let f = stdlib::load_string(src, Some("gameplay_helpers"), t.globals().clone()).unwrap();
        let r = t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?}\n{tr}")).unwrap();
        assert_eq!(pz_kahlua::exec::java_value_of(&r[0]), "farming");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[1]), "water");
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[2]), Some(10.0));
        assert_eq!(pz_kahlua::exec::java_value_of(&r[3]), "ok");
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[4]), Some(1.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[5]), Some(2.0));
        assert!(r[6].truthy());
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[7]), Some(1.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[8]), Some(2.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[9]), Some(3.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[10]), Some(4.0));
        assert!(r[11].truthy());
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[13]), Some(50.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[14]), Some(60.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[15]), Some(0.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[16]), Some(0.0));
        assert!(!r[17].truthy());
        assert!(!r[18].truthy());
        let puddles = crate::bind_puddles::shared_puddles();
        assert!((puddles.borrow().state.muddy_puddles.get() - 0.75).abs() < 1e-5);
    }
}

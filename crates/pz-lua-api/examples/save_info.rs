//! Print `getFullSaveDirectoryTable()` and `getSaveInfo()` of every save in
//! a user folder, with table keys sorted, to compare against the original.
//!
//! Usage: `save_info <game-dir> <user-folder> [game-mode]`. The user folder
//! (the one holding `Saves/`) should be a scratch copy: `getSaveDir` creates
//! `Saves/` when it is missing.

use pz_kahlua::Value;
use std::collections::BTreeMap;
use std::rc::Rc;

fn show(th: &mut pz_kahlua::Thread, v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(b) => format!("Boolean:{b}"),
        Value::Num(n) => format!("Double:{}", pz_kahlua::number::java_double_to_string(*n)),
        Value::Str(s) => format!("String:{s}"),
        Value::Table(t) => {
            let m: BTreeMap<String, String> = t
                .entries()
                .iter()
                .map(|(k, v)| (pz_kahlua::exec::java_value_of(k), show(th, v)))
                .collect();
            let parts: Vec<String> = m.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{{{}}}", parts.join(", "))
        }
        // ActiveMods: its two lists, as the oracle prints them.
        Value::Object(_) => {
            let mut list = |name: &str| {
                let f = th.table_get(v, &Value::str(name)).unwrap();
                let l = th.call1(&f, std::slice::from_ref(v)).unwrap();
                pz_kahlua::exec::java_value_of(&l)
            };
            format!("ActiveMods{}{}", list("getMods"), list("getMapOrder"))
        }
        other => pz_kahlua::exec::java_value_of(other),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("game directory containing media/");
    let user = args.next().expect("user folder holding Saves/");
    let mode = args.next().unwrap_or_else(|| "Apocalypse".into());
    let mut gfs = pz_fs::GameFs::new(game);
    gfs.set_cache_dir(user);
    let mut mgr =
        pz_lua_api::LuaManager::init(Rc::new(std::cell::RefCell::new(gfs))).expect("init");
    pz_lua_api::bind_core::shared_core()
        .borrow_mut()
        .set_game_mode(&mode);
    let call = |mgr: &mut pz_lua_api::LuaManager, name: &str, args: &[Value]| {
        let f = mgr.thread.globals().get_str(name);
        mgr.thread.call1(&f, args).expect(name)
    };
    let full = call(&mut mgr, "getFullSaveDirectoryTable", &[]);
    println!("FULL {}", show(&mut mgr.thread, &full));
    let t = full.as_table().unwrap().clone();
    for i in 1..=t.len() {
        let k = t.get_int(i);
        let info = call(&mut mgr, "getSaveInfo", std::slice::from_ref(&k));
        println!(
            "INFO {} {}",
            k.as_str().unwrap_or(""),
            show(&mut mgr.thread, &info)
        );
    }
}
